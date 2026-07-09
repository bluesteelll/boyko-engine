> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9.2 — Results: executor `Scope` is Tree-Borrows-GREEN (outcome i)

Branch `ecs`. Phase 9.2 turned the work-stealing `Scope` fork/join from Phase 9.1's
**outcome (ii)** ("sound by proof; Tree Borrows over-approximates one pattern") into
**outcome (i): genuinely Tree-Borrows + data-race CLEAN under Miri**, with **0% native
regression** and the multi-thread executor unchanged in behavior.

Plan: [`docs/PHASE-9.2-PLAN.md`](PHASE-9.2-PLAN.md).

## Status: COMPLETE — TB-green via two changes

1. **`NonNull<ScopeShared>` field refactor** (the TB-protector fix). `Scope::shared`
   changed `Box<ScopeShared>` → `NonNull<ScopeShared>`. `NonNull::as_ptr` is a `Copy`
   that copies the pointer **without retagging the pointee**, so `Scope::drop`'s
   `&mut self` protector covers only the 8-byte field, never the heap allocation —
   removing the Phase-9.1 protected-tag over-approximation (the dispatcher no longer
   holds a protected `&ScopeShared` spanning the workers' `pending.fetch_sub`).
   `Box::into_raw` in `new`; single `Box::from_raw` reclaim in `drop`. No Arc, no
   refcount, no lock; byte-identical native cost to the former `Box`.

2. **Candidate U — unpark-before-decrement** (the data-race fix). `complete_task` now
   does `self.waker.unpark()` **then** `self.pending.fetch_sub(1, AcqRel)`. The
   `waker` read happens while `pending >= 1` (allocation provably alive); the
   `fetch_sub` is the worker's **last** byte-access to the allocation, after which the
   dispatcher's `Scope::drop` may free it with no use-after-free. The box is freed at
   the **single** `Scope::drop` site, unconditionally, after the join.

## The journey: a handshake design that passed Miri/loom/stress but heap-corrupted in the integrated bench

The first 9.2 attempt was a per-scope `AtomicU8 free_state` "second-swapper-frees"
handshake (worker reads `waker` after the decrement, then a release-swap orders the
free after that read; whichever of {last worker, dispatcher} swaps second frees).

It **passed** `miri_scope` (16 seeds, TB+data-race clean), **loom** (4/4, with an
exactly-one-freer invariant), **stress** (Drop-accounting 6+1), and **threadpool unit**
(18/18) — then **deterministically crashed `0xc0000374 STATUS_HEAP_CORRUPTION`** in the
`phase9_scheduler` bench (`phase9_schedule_run_two_disjoint`). An A/B (git-stash the 9.2
changes, full bench on baseline) proved it: **baseline-full is clean; 9.2-full crashes**.

**Root cause — multi-drain.** The ECS `Schedule::run` executor uses **one
`pool.install(|scope| …)` per frame** and dispatches systems in **waves** gated by
apply-window barriers; between waves the dispatcher parks and is woken by
`complete_task`'s `waker.unpark()` at each wave's `pending -> 0`. So a single scope's
`pending` oscillates to **0 multiple times** per frame. The handshake's premise — "`pending`
hits 0 exactly once per scope" — is **false** for the executor: wave 1's last completer
swaps `free_state` RUNNING→JOINED (returns false, OK); wave 2's last completer swaps
JOINED→JOINED, observes JOINED, **frees the box mid-scope** → double-free at `Scope::drop`.

**Why every isolated oracle missed it:** `miri_scope`, `loom`, `stress`, and
`par_iter` are all **single-drain** — they spawn a batch and join once. Only the
**full integrated `phase9_scheduler` bench** exercised the executor's multi-wave
pending-oscillation. **The integrated bench was the only oracle that caught it.**

Candidate U is immune by construction: the free is tied to scope **END**, never to an
intermediate wave's `pending -> 0`. It is also a **net simplification** — it deletes the
`free_state` atomic, the `FREE_*` consts, the `spawned_any` `Cell`, the worker-side
`Box::from_raw`, and the handshake entirely.

## Verification gate (all run by the orchestrator)

| Oracle | Result |
|--------|--------|
| **`phase9_scheduler` full bench** (the multi-drain crash oracle) | **exit 0, no `0xc0000374`**; `two_disjoint` 1.23 µs (was a crash) |
| 0%-regression | 50-sys 4.12 µs (baseline 4.08), par_iter 19.2 µs (baseline 20.3), two_disjoint 1.23 µs (baseline 1.25), empty 5.4 ns, one_excl 248 ns — **within noise** |
| **`miri_scope`** (TB + data-race), default seed | **3/3 pass, zero UB** |
| `miri_scope` `-Zmiri-many-seeds=0..16` | **15/16 pass; 0 UB in any seed.** 1 seed: a *liveness* timeout (see caveat) — not UB |
| **loom** (`loom_pool`) M1/M2/M2b/M3 | **4/4 pass.** M1's joiner re-poll changed `park()` → `yield_now()`: under Candidate U `complete_task` unparks BEFORE its `fetch_sub`, which loom (#246) does not persist as a pre-`park` token and cannot recover via `park_timeout` (which loom can't model), so a `park()` model false-deadlocks and blows loom's state space (STATUS_STACK_OVERFLOW). The `yield_now` re-poll models the production timeout-backstop; the total-ordered `fetch_sub` RMW still proves `is_drained()` observes 0 in every interleaving (no permanent lost wakeup) — mirroring how M2/M2b/M3 model the loom-opaque deque. |
| **stress** (`stress.rs`) | **6 pass + 1 ignored** (Drop-accounting: exactly-once free under steal contention) |
| **threadpool unit** (`--lib`) | **16/16** (incl. the new `scope_multi_drain_frees_once`; the first attempt's 3 handshake-specific unit tests were removed with the handshake) |
| native `cargo build --release` / `clippy --workspace --all-targets -D warnings` | clean |
| `boyko-ecs --lib` | **494 pass, 0 failed, exit 0** — verified clean ×3 under Candidate U (the handshake era's `0xc0000374` here was the SAME multi-drain double-free in the linked `boyko-threadpool`, not a resource bug; see the correction note below) |

## Caveat: 1/16 Miri many-seeds liveness timeout (not UB; real-hardware-safe)

Candidate U has a documented **lost-wakeup window**: because `unpark` precedes the
decrement, the dispatcher can re-poll a stale `pending > 0`, re-park, and miss the final
decrement (which issues no further unpark). On **real hardware** this is recovered by the
`park_timeout` backstop already present in both the join loop (50 µs) and the executor
Step-5 park (100 µs) — proven by the bench (millions of iterations, never hung) and the
deterministic default-seed `miri_scope` pass. **Miri cannot model `park_timeout`**, so on
1/16 adversarial seeds the window surfaces as the test's bounded-spin timeout (a panic,
**not** UB). All 16 seeds are data-race + TB clean. If this Miri-many-seeds flake ever
becomes a CI nuisance, the escalation is **Candidate K** (move the waker to pool-stable
storage so the post-decrement box read disappears and the unpark can stay conditional) —
deferred as over-engineering for a Miri-only modeling artifact.

## Hard-won lessons

1. **Isolated Miri/loom/stress are not enough for a fork/join primitive used by a
   multi-wave executor.** They were all single-drain and unanimously green on a design
   that deterministically heap-corrupted under the real executor. **Always run the
   integrated bench/workload as a gate** — it was the sole oracle that caught the
   multi-drain double-free.
2. **A/B against baseline is the definitive attribution.** "par_iter passed so the
   handshake is fine" was a strong-but-wrong inference; `git stash` + full-bench on
   baseline (clean) vs 9.2 (crash) settled it.
3. **The simplest correct design won.** The handshake was clever and passed every
   isolated check; unpark-before-decrement (a 2-line reorder + deleting machinery) is
   correct by construction for multi-drain and has 0% native cost.
4. **Process discipline (re-learned the hard way):** batching many `Edit`/`Bash` calls in
   one message caused chaotic interleaving that left `scope.rs` half-edited and
   inconsistent. Recovery was `git checkout` to a known baseline + a clean re-implement
   with **one operation per message**. Compiler + Miri exit codes are the only oracles.

## Pre-existing bugs surfaced (separate from 9.2; filed as follow-ups)

These were exposed (not caused) by 9.2 finally getting Miri/bench past the old TB error:

- **Pool-thread Arc-cycle leak.** `worker_main(pool: Arc<ThreadPool>, …)` — each worker
  holds an owned `Arc<ThreadPool>`, so `ThreadPool::drop` (which signals shutdown + joins)
  never runs while workers are alive; the shutdown path is effectively unreachable and the
  pool's threads leak to process exit. Benign (a leak, not UB) for a process-lifetime
  singleton pool, but real. → Phase 9.3 candidate.
- **ECS executor not Miri-cooperative** — **PARTIALLY ADDRESSED, still open (Phase 9.3).**
  `Schedule::run`'s Step-5 wait-loop used `park_timeout`, which does not advance Miri's
  deterministic scheduler. Adding `#[cfg(miri)] std::thread::yield_now()` to that branch
  (native keeps `park_timeout`; `PARK_TIMEOUT` + `Duration` now `#[cfg(not(miri))]`) was
  NECESSARY but NOT SUFFICIENT: `miri_schedule_parallel` STILL livelocks (300 s `timeout`
  kill, exit 124). There is at least one more non-cooperative wait site on the Miri path
  (candidates: `apply_window_drain`/completion-queue spin, `try_dispatch_ready`, or the
  pool-side `join_workers_until_drained` interaction). The test remains `#[ignore]`d; the
  boyko `Scope` surface stays gated by the green `miri_scope`. The yield addition is
  native-byte-identical (cfg-split) ⇒ 0% regression and is kept as one necessary step.
  Full integrated-executor Miri coverage is deferred until every Miri-path wait site
  yields. → Phase 9.3 (open).

## Correction: the `boyko-ecs --lib` `0xc0000374` was NOT the ZST-resource bug

During the failed-handshake phase I saw `0xc0000374 STATUS_HEAP_CORRUPTION` from BOTH
`phase9_scheduler` AND `boyko-ecs --lib`, and initially attributed the latter to the
"ZST-resource heap-corruption-at-exit (Phase 16 incidental)". That attribution was
**wrong**. A project-analyst pass confirmed the **ZST-resource bug was already fixed** by
commit `a0bdf5d` (an ancestor of HEAD — both manual `dealloc` sites in
`resources.rs` guarded by `if layout.size() != 0`, with a `miri_zst_resource.rs` proof)
and **does not reproduce**. The real cause of the `boyko-ecs --lib` crash was the **same
multi-drain double-free** in the linked `boyko-threadpool` (the handshake `complete_task`
freed a scope box mid-frame; `boyko-ecs` links that library and the parallel `Schedule`
exercises it). Under Candidate U, `boyko-ecs --lib` is clean (494 pass, exit 0, verified
×3). There is **no** outstanding ZST-resource bug; bug A-1 is RESOLVED.

## Files

- `crates/boyko_threadpool/src/scope.rs` — NonNull field + `complete_task`
  unpark-before-decrement + single-free `Scope::drop` + `scope_multi_drain_frees_once` test.
- `crates/boyko_threadpool/src/lib.rs` — loom-shim doc updated (`complete_task` → `()`).
- `crates/boyko_threadpool/src/sync.rs` — unused `AtomicU8` re-export removed.
- `crates/boyko_threadpool/tests/loom_pool.rs` — M1 doc note (loom #246 wake model).
- `crates/boyko_ecs/tests/miri_schedule_parallel.rs` — re-`#[ignore]`d (executor livelock).
- `crates/boyko_ecs/tests/miri_phase9.rs` — deferred-note updated to RESOLVED.

Not committed (no explicit request). HEAD remains `a4fed3b`.
