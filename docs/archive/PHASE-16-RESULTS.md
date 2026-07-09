> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 16 — Results (Run Conditions `.run_if`)

**Status:** ✅ COMPLETE (committed locally on branch `ecs`). Touches the
per-frame executor; 0%-regression verified; race-free single-threaded eval.

## What shipped
`.run_if(predicate)` on systems and sets — a system (or every member of a set)
runs this frame only if its condition(s) return true.
- **A condition is `impl IntoSystem<(), bool, M>`** — any `fn(SystemParams…) -> bool`
  reusing the existing `SystemParam`/`FunctionSystem` machinery. New erased type
  `BoolSystem = Box<dyn System<Out = bool>>` (the `System` trait already permits
  `Out = bool`; `SystemBox` — the `Out=()` 1-cache-line hot-path slot — is UNTOUCHED).
- `SystemConfig::run_if` + `ConfigureSet::run_if`; multiple `.run_if(a).run_if(b)`
  AND (eager fold — all run every frame, NO short-circuit, so stateful conditions
  advance). Built-in `run_once` (`fn(Local<bool>) -> bool`). `resource_exists`
  deferred (no `Option<Res<R>>` SystemParam); typed combinators / `on_event` /
  `in_state` deferred.
- **Executor integration:** a new `evaluate_ready_conditions` pass runs at the
  apply-window boundary, gated on `running.count_ones() == 0` (no worker holds a
  cell → race-free; verified). A false fold → mark completed + decrement
  successors' `pred_remaining` WITHOUT running the body (Bevy's skip-but-keep-
  dependents: a skipped system's `before` successors still run). Set conditions
  evaluated ONCE per frame (memoized in `ExecutorScratch`), gated via Phase-15
  transitive membership; gate = AND(own) AND(gating-set).
- **`run_condition` does NOT call `apply`** — conditions are pure read-only
  predicates; a `Commands`-using condition's deferred commands are dropped (logic
  error), never flushed mid-eval-pass.

## Pipeline
research (NO dormant scaffold this time; the `Out=()` storage pin is the blocker,
but `System`/`IntoSystem`/`FunctionSystem` are already `Out`-generic) → architect
→ critic R1 (**REVISE, no CRITICAL** — 0%-gate + race-freedom VERIFIED correct; 2
HIGH proof/test-precision + 3 MEDIUM + 3 LOW) → §0 Round-2 patches (P1 proof, P2
cascade test, P3 `n_final` guard, P4 tick footgun reframe, P5 citation, P6
`run_condition`-no-`apply` decision) → developer → code-review **APPROVED** (10/10
targets verified, no findings) → tester (34 tests, race-guard genuine, 0% gate).

## Findings (resolved)
- The critic VERIFIED the two high-stakes items correct: **0%-gate** (`try_dispatch_ready`
  byte-identical, `SystemBox` untouched, one `has_condition.is_clear()` not-taken
  branch) and **race-freedom** (`running==0` ⟹ no live worker, via dispatcher
  `running` set-before-spawn / clear-after-drain). No redesign needed.
- **Tick footgun (§0-P4, documented not fixed):** a read-only `Changed<T>`/`Added<T>`/`Ref<T>`
  condition compiles but silently reports all-changed (always-true) because
  `run_condition` doesn't `set_change_ticks`. Documented as an unguarded footgun;
  correct tick-aware conditions are a Phase-16.1 follow-up.

## Measured results
- **Correctness:** 13 in-module unit + 15 integration + 6 Miri-file = **34 tests**,
  all pass. Coverage: run_once, true/false/resource-reading gates, skip-successor,
  skip_run_skip mixed cascade, eager-fold-advances-locals, set-condition-once-per-
  frame, set-AND-own (4 truth-table corners), and the **race guard** (instrumented:
  8 workers overlapped, condition observed `running==0` always — a broken guard
  would fail it).
- **Full workspace:** 803 passed, 0 failed, 22 ignored. No regressions.
- **Miri** (`-Zmiri-tree-borrows`): 3/3 clean (the `run_condition` cell reborrow is
  UB-free). Full-schedule Miri gated `#[cfg(not(miri))]` (the known Phase-9
  `Scope::spawn` protected-tag deferral, not Phase 16).
- **0%-regression gate:** A/B via `git stash` vs pre-Phase-16 (`c83426c`) on
  `g1_boyko_50_empty_systems` — criterion **"No change in performance detected"**
  (12.85 µs → 13.03 µs, +0.85% within ±2-3% noise, CI straddles 0). The
  `has_condition.is_clear()` early-out makes a condition-free schedule statistically
  indistinguishable from pre-Phase-16.
- **Build:** `cargo build --release` + `cargo clippy -p boyko-ecs --lib -- -D warnings` clean.

## Residuals / follow-ups
| Item | Status |
|---|---|
| `resource_exists` built-in | Deferred — needs `Option<Res<R>>` SystemParam (not supported today) |
| Typed combinators (`.and`/`.or`/`.not`) | Deferred |
| `on_event` / `in_state` conditions | Deferred (`in_state` → Phase 17 States) |
| Tick-aware conditions (`Changed`/`Added`) | Phase 16.1 — needs cross-frame `set_change_ticks` on conditions |
| **ZST resource → heap corruption at exit** (pre-existing, NOT Phase 16) | Flagged by the tester; FEATURE_MAP notes ZSTs unsupported — should reject cleanly instead of corrupting. Own task. |

## Key files
- Modified (impl): `crates/boyko_ecs/src/ecs/core/schedule/{schedule,schedule_builder,system_config,system_box,system_descriptor,executor_scratch,mod}.rs`, `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`. New: `schedule/common_conditions.rs`.
- Untouched (hot path): `try_dispatch_ready` (byte-identical), `SystemBox` struct.
- Tests: `tests/phase16_run_conditions.rs`, `tests/miri_phase16.rs` + in-module unit tests.
- Docs: `PHASE-16-RESEARCH.md`, `PHASE-16-PLAN.md` (incl. §0 Round-2), this file.
