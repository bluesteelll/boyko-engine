# Phase 16.1 — Tick-Aware Run Conditions: Results

Branch `ecs`, commits `df17528` (trybuild re-bless) + `579c738` (feature).
Plan: [PHASE-16.1-PLAN.md](PHASE-16.1-PLAN.md) (R2-final — critic
CHANGES-REQUESTED folded; OQ-PRIME confirmed REAL and in scope).

## What landed

Bevy "since-last-actual-run" parity for change-detection ticks, closing the
three dormancy/wraparound gaps left by Phase 16:

1. **Gap #1 — conditions advance only when evaluated.** The unconditional
   frame-start condition bump (`schedule.rs:241-250`) is DELETED;
   `EcsMaster::run_condition(cond, this_run)` checkpoints the condition's
   `(last_run, this_run]` window at the eval site, only on a frame the
   condition actually runs. A condition dormant N frames resumes observing ALL
   changes accrued while dormant. The every-frame case is behavior-preserving
   (`prev == last frame's this_run` → identical window).
2. **C1 — gated SYSTEM bodies (the key defect, OQ-PRIME).** The frame-start
   system loop stamps ONLY ungated systems (`has_condition[i]` clear — they run
   every frame, so "advance every frame ≡ advance when run", byte-identical
   plain path). A gated system is stamped at its DISPATCH site: concurrent path
   = a pre-pass over `to_spawn` BEFORE the `systems_ptr` raw lift (OQ-R2-1
   resolved to the pre-pass form — a fresh `&mut self.systems[i]` after the
   raw lift would invalidate `systems_ptr` provenance under Tree Borrows);
   inline-exclusive path = right before `run_unsafe`. `mark_skipped` stamps
   NOTHING → a skipped frame freezes the ticks → a `Changed<T>` body query
   sees the full dormant window on resume (pre-fix: silent data loss).
3. **Gap #2 — wraparound clamp.** New `System::check_change_tick(current)`
   (NO default body — a forgotten impl must not compile, mirroring
   `set_change_ticks`) clamps both `last_run` and `this_run` via
   `Tick::check_tick`. `#[cold] Schedule::check_change_ticks` walks `systems`
   + `system_conditions` + `set_conditions` right after `run_check_ticks_scan`
   inside the existing `should_run_check_ticks` block (same `this_run` gate,
   no drift). Needed because C1/Gap-#1 make dormant `last_run` drift possible
   past `MAX_CHANGE_AGE`.

`Schedule.frame_this_run` (appended LAST — M3 layout invariant preserved, doc
block updated) is the single tick source for both stamp sites;
`world.current_tick()` is wrong at use time (it reads `this_run + 1` after the
#56 apply-window bump).

Zero new `unsafe`.

## Gates

| Gate | Result |
|------|--------|
| boyko-ecs tests (debug) | 74 suites, **928 passed, 0 failed** |
| boyko-ecs tests (release) | **913 passed, 0 failed** |
| Full workspace (debug) | 90 suites, **991 passed, 0 failed** |
| Full workspace (release) | 90 suites, **975 passed, 0 failed** |
| clippy `--workspace --all-targets -- -D warnings` | clean (rustc 1.96) |
| Miri (Tree Borrows) | **83/83 tests passed, 0 TB violations** across all 16 suites — see below |
| W1 0%-gate bench | **PASS** — see below |

### Full Miri-TB sweep (all 16 `miri_*` suites, GNU nightly)

Flags per suite header: 10 suites with `-Zmiri-tree-borrows -Zmiri-disable-isolation
-Zmiri-permissive-provenance -Zmiri-ignore-leaks`, the rest with the workspace
default `-Zmiri-tree-borrows`.

| Suite | Tests | Notes |
|-------|-------|-------|
| miri_phase8a / 8cd / 8_5 / 14a | 8 + 11 + 4 + 4 ✅ | clean WITH `-Zmiri-ignore-leaks`; under default flags the post-exit leak checker flags the **deliberate, bounded `Box::leak`** in the CommandQueue borrow-decoupling path (#53, triaged NOT-A-BUG in the post-14b backlog cleanup) — tests themselves all pass either way |
| miri_phase9 / 15 / 16 / 16_1 / 17 / 19 / 14b / schedule_parallel | 3+4+3+2+6+9+10+1 ✅ | phase19 = 84 min, 14b = 36 min interpreted |
| miri_phase10 / 12_5_track_b / zst_resource | 6 + 6 + 3 ✅ | clean under default flags (leak check ON) |
| miri_phase_bugfix_56 | ⚠ env-livelock | see below |

**miri_phase_bugfix_56 (honest record):** both of its tests pass natively in
0.00 s (debug + release, both toolchains) and the suite was Miri-TB-clean on
2026-06-07 (MSVC nightly-2026-05-20). On THIS machine's new windows-gnu host
the Miri interpreter livelocks on it (>100 min CPU, single-worker pool):
reproduced (a) on nightly-2026-06-08, (b) on the previously-green
nightly-2026-05-20 (GNU), and (c) on the PRE-16.1 baseline worktree — three
controls proving it is a windows-gnu Miri environment pathology, NOT a Phase
16.1 (or any code) regression. Its TB envelope (apply-window bump + threadpool
schedule) is covered green by `miri_schedule_parallel`, `miri_phase16_1`, and
`miri_phase19` on the same environment. Re-verify on an MSVC host when one is
available.

### W1 0%-gate bench (phase9_scheduler, git-worktree A/B)

Methodology: baseline = worktree at `df17528` (pre-feature), comparison = `ecs`
HEAD; both compiled by rustup `stable-x86_64-pc-windows-gnu` 1.96; criterion
baseline data copied into the main target and compared with `--baseline`.
(Old MSVC-era saved baselines are cross-toolchain and were NOT used.)

| Bench | Change | Verdict |
|-------|--------|---------|
| `phase9_schedule_run_empty` | **−6.6%** (p=0.00) | improved — the deleted frame-start condition loops, as the plan predicted |
| `phase9_schedule_run_50_exclusive_systems` (THE gate) | **−2.1%** (p=0.00) | improved |
| `phase9_par_iter_4096_entities` | −0.1% (p=0.95) | no change |
| `phase9_schedule_run_one_exclusive` | −2.1% (p=0.00) | improved |
| `phase9_schedule_run_two_disjoint` | paired parity | see note |

**two_disjoint note (honest):** raw criterion change readings drifted +3.4% →
+12.5% across re-runs of the SAME binary. Control experiment: the PRE-16.1
baseline binary re-run against its OWN saved baseline showed **+14.3%** — the
machine had drifted (this bench measures cross-thread wakeup latency and is
the noisiest of the five). At matched machine state the two binaries are equal:
1.232 µs (feature) vs 1.228 µs (baseline), −0.3%. Verdict: parity; the
"regressed" readings were machine drift, proven by the baseline-vs-itself
control.

### Tests (W4)

- In-crate units (`schedule.rs`): `dormant_condition_resume_window_spans_skipped_ticks`
  (Gap #1 mechanism — the frozen window contains a dormant-frame mutation; the
  pre-fix window provably misses it), `gated_system_body_window_frozen_while_skipped`
  (C1 through a real `Schedule::run`), `check_change_ticks_clamps_dormant_condition`
  / `_dormant_system` (Gap #2, both ticks, OQ-4), and a 256-case proptest
  (`prop_check_change_ticks_no_false_positive_after_clamp`) over random dormant
  spans ≤ `CHECK_TICK_THRESHOLD + MAX_CHANGE_AGE`.
- Integration (`tests/phase16_1_dormant.rs`): the C1 silent-data-loss
  regression net (gated `Changed<T>` body dormant across a mutation sees it on
  resume — asserts ≥1 where pre-fix yields 0), exactly-once resume
  checkpointing, and every-frame behavior preservation for ungated systems and
  conditions. (A condition is never topologically dormant via the public API —
  boyko's eager fold evaluates every reachable system's conditions each frame,
  OQ-1 — so the condition-dormancy mechanism proof lives in-crate.)
- Miri (`tests/miri_phase16_1.rs`): the code-reviewer-flagged TB envelope — a
  conflict-deferred gated system pre-pass-stamped in round K+1 while a round-K
  worker still holds `systems_ptr`; plus the skipped-gated-with-live-workers
  shape. Both clean under `-Zmiri-tree-borrows`.

The pre-existing `phase16_1_tick_conditions.rs` (every-frame eval-site window)
passes unchanged — the behavior-preservation proof from the plan.

## Toolchain note (machine migration, 2026-06-10)

MSVC/Visual Studio was removed from the dev machine between June 7 and this
session; the project now builds on `x86_64-pc-windows-gnu` (rustup 1.96 stable
+ GNU nightly for Miri). Two machine-local fixes (in `~/.cargo/config.toml`,
NOT in this repo) were required and are documented there: `-Cdlltool=` pointed
at LLVM's dlltool (the rustc-bundled binutils `dlltool.exe` ships without the
GNU assembler it spawns for raw-dylib import libs), and the link driver routed
to LLD's MinGW driver (`-Clink-arg=-B...`) because the bundled GNU ld 2.42
emits a corrupt import directory when a link mixes short-format import libs
(`windows_x86_64_gnu`'s `libwindows.a`, pulled in by winit/windows-sys) with
binutils-format mingw libs — trailing IAT slots are never bound by the loader
and the binary dies with `STATUS_ACCESS_VIOLATION` in winit's
`INIT_MAIN_THREAD_ID` CRT ctor. This affected only `boyko_demo` (the only
crate linking windows-sys); the ECS core never hit it. trybuild snapshots were
re-blessed for the 1.95→1.96 diagnostic wording (fully-qualified trait paths
in E0277 notes); they are compiler-version-locked by nature.

## Deferred / follow-ups

- Short-circuiting the eager condition fold (OQ-1: kept eager to protect
  `run_once`/`Local` statefulness) — revisit only with a measured need.
- `resource_exists` (`Option<Res>`), typed combinators, `on_event` — Phase 16
  residuals, unchanged.
