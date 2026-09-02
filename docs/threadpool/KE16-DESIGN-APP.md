# KE16 design — caller-side items, scheduler scoping, the locality instrument

Part of the KE16 design; index in `KE16-DESIGN.md`. Everything in §1, §5–§10 ships in the pass
regardless of which candidates win the tournament; §3 and §4 state what is NOT built and why.
Revision 4: the acceptance line's primary reference is selected by LANE COUNT — the W−1 row when
the shipped B keeps the external helper (B0/B1), `bench_thread_install` when it parks (B3) — and
the "coincide" sentences are deleted (§11; the critic's round-3 blocking item 2); App-8 states the
EVT1 lane-interleaving consequence of an inline sibling system (§7; non-blocking item 8);
`ThreadPool::parked_mask()` joins the harness surface (§10). Revision 3: the foreign-install test
asserts on a (pool address, worker id) receipt, not on a bare id (§5); the
`bench_thread_install_Wminus1` reference row is mandatory (§11); the doc-correction checklist gains
the `sync.rs` fence note and the loom fidelity note (§9).

## 1. App-1 — caller-aware lane count at the three physics sites (S9)

The three sites compute `lanes = pool.num_threads() + 1` with the rationale "the dispatcher lane
that called `pool.scope` ALSO work-steals while the scope is open":
`crates/boyko_physics/src/solver/colored.rs:2628-2640`, `crates/boyko_physics/src/soft/colored.rs:
991-996`, `crates/boyko_physics/src/resources.rs:1470-1477,1493-1496`. On the production caller the
lane pool is **1** today and **W** after any A-fix, never W+1: the frame-path dispatcher is parked
(J11) and the route-(b) joiner is one of the W workers. The `+1` came from the bench route, where the
external joiner is an extra lane whose share is bimodal and measured, not counted.

Edit at each site: `let lanes = pool.num_threads();` and the comment rewritten (index §7). At
`resources.rs:1494-1496` the guard `if lanes < 2 { return false; }` was dead (`num_threads() ≥ 1`,
`thread_pool.rs:586`) and becomes LIVE with the right meaning: a one-worker pool takes the O2 serial
`build`. Re-aim, do not delete. `n_chunks = clamp(lanes × CHUNKS_PER_WORKER, 1, n)` then targets
`W × {4, 6, 6}` chunks — 64 / 96 / 96 at W=16 instead of 68 / 102 / 102. Bit-identity is
chunk-count- and chunk-shape-independent (`colored.rs:2635-2638`), so the `{1, N}` oracles do not
move; the `CHUNKS_PER_WORKER` constants are unchanged (they are the knob Step App may sweep later,
not now). `lanes` only sizes the chunk count at all three sites (never a per-lane buffer index), so
an external joiner that helps on the bench route cannot index out of bounds.

## 2. App-2 and App-3 live inside A1 and B1

`Worker::new_lifo()` is A1's sub-choice (`KE16-DESIGN-A.md` §1.4); the randomised, self-skipped
joiner sweep is B1's step 3 (`KE16-DESIGN-B.md` §2.2). Neither is a separate switch. If B0 wins Step
B the fixed `0..n` self-including sweep at `scope.rs:534-541` is STILL replaced by
`try_steal_random`'s order in the shipped code (App-3 ships regardless: it is a one-line call
change with no behavioural risk and removes the joiner's bias toward worker 0).

## 3. S6 / S10 / M1-a — out of this pass, into KE17

**Decision: a separate ticket.** Reasons, in order of weight:

1. **Different invariant, different crate.** S6 (apply-window early release) changes when
   `running.set(i, false)` and `pred_remaining[s] -= 1` happen (`schedule.rs:722-762` today, only
   inside `apply_window_drain`). The SAFETY block at `schedule.rs:603-611` ties the exclusive
   `&mut world` to the drain; releasing readiness early must guarantee a successor never observes a
   predecessor's DEFERRED commands before they are applied — the conflict graph would have to carry
   deferred-command targets, or early release be restricted to successors whose edge is a direct
   component access. That is an architect's question for the ECS scheduler, with its own loom/Miri
   surface (`tests/miri_phase9.rs`), not a pool change.
2. **Independent of every pool candidate.** Nothing on axes A/B/W changes the apply-window gate; the
   pool pass can be verdicted without it, and its own verdict needs the pool fixed first (otherwise
   a lane freed early has nothing intra-system to take).
3. **Its share is unmeasured.** The owner's observation ("lanes drain at different times") has two
   causes; the pool one is being fixed. M1-a — measure the barrier's share with a CONFLICTING
   schedule of unequal system costs, subtracting the dispatcher's ≥1 ms Windows park between rounds
   (axis 36) — is KE17's first rung. The `RoundProbe` (`zones.rs:222-260`) records round width
   already; the instrument needs per-round "workers idle while ready-but-blocked successors exist".
4. **The owner's precondition is met.** Assignment is dynamic (`schedule.rs:967-1326`); an idle
   worker already takes any conflict-free ready system. The lever is the gate, and S10 (inline
   successor on the completing worker, the cheapest answer to the locality objection — Nanos6's
   `immediate_successor` and TBB's scheduler bypass are the shipped shapes, round 2) presupposes S6.

What KE17's row in `docs/aether-v2/KERNEL-BACKLOG.md` must carry (the orchestrator opens it at pass
end): the SCH7 question above; M1-a's instrument; S6's two shapes (full early release vs
direct-access-edge-only); S10 with a probability knob and the conflict check; the frame-scope
count-gated wake note and the EXTERNAL-joiner lifetime-independent wake target (a leaked per-thread
`'static` `WakeHandle` slot with decrement-then-unpark kept unconditional for the frame scope) from
`KE16-DESIGN-W.md` §3.5.

## 4. L10 — the cache-residency axis: what is and is not measurable at this checkout

**Not measurable here:** the cost of handing a whole SYSTEM to another lane in L1/L2 misses.
`pin_workers` is a no-op (`thread_pool.rs:551-557`); nothing in the tree reads a hardware counter;
the target's topology is unrecorded (App-9 fixes that). Any claim that "system S on lane B pays a
migration" is unfalsifiable at this checkout and this pass makes none.

**Measurable here, and measured by the tournament:** locality at CHUNK granularity. A1 keeps a
worker's own chunks on its core until a sibling steals; A3 puts every chunk on a shared line with no
producer-consumer affinity; A5 moves chunks to other cores by construction. At equal reachability
(all three reach W) and equal wake-decision cost (the fenced prologue is charged to every arm,
`KE16-DESIGN-W.md` §1), the A1-vs-A3 delta on the worker route is the locality signal at this
granularity, and the A5-vs-A1 delta at 100 µs–1 ms × W is the price/gain of moving work to an idle
core on purpose. The ECS harness's two populations (4096 vs 65536 rows: 4-chunk vs 16-chunk waves)
give the same signal at two working-set sizes.

**What it would take to measure L10 (recorded; not built here, no reason to build it before KE17
needs it):**

1. App-9's topology record (one command) — the precondition for any per-core claim.
2. `pin_workers` made real: `SetThreadAffinityMask` (Windows) / `sched_setaffinity` (Linux) per
   worker at `thread_pool.rs:645-660`, so a controlled experiment "system S on the lane that last
   ran it vs on another lane" is repeatable. In-house, ~40 lines behind the existing builder flag.
3. A residency probe. The in-house route is a software touch-set estimate: a `boyko_diag` sample
   per system recording the sum of accessed column widths × rows (an archetype query knows its
   footprint exactly — L7's observation) compared against the recorded L2 size; the hardware route
   (PMU counters: `perf` on Linux, `PdhCollectQueryData`/ETW on Windows) is third-party at the
   tooling level and outside the engine-library scope the owner's in-house rule covers. Neither
   exists today.
4. The experiment: a schedule of two systems sharing a working set that fits L2 but not L1, run with
   pinned lanes in the same-lane and other-lane configurations, wall-clock per system from the
   `SystemSpan` zones. Linux's 500 µs `sysctl_sched_migration_cost` (L13) is the only shipped
   numeric proxy for "hot".

## 5. App-6 — the ONE identity predicate for the push arm, the joiner and the W-d′ target (J14)

Today three places decide "am I a worker of this pool" three ways: `push_task` with the pool
pointer AND `wid < len` (`worker.rs:369-370`); `join_workers_until_drained` with `wid < len` alone
(`scope.rs:441-442` — a pool-A worker joining a pool-B scope drains B's `injector_local[wid_A]`, a
slot owned by B's worker `wid_A`); and the round-1 W-d′ text with the deque pointer alone (which,
inside `pool.install` on a worker of that pool, would have produced `joiner = Some(WORKER_ID_
DISPATCHER)` and indexed `inner.workers` out of bounds — `install` rewrites `CURRENT_WORKER_ID` to
the sentinel at `thread_pool.rs:205` but does not touch a deque slot).

Fix, shipped under every configuration: `tls::worker_lane_for(inner) -> Option<WorkerLane>`
(`KE16-DESIGN-A.md` §1.1) is the only predicate. It is `Some(lane)` iff (the pool tag is `inner`)
AND (`current_worker_id() < inner.worker_count`); under A1 the tag is the deque deposit, under
A2/A3/A5 it is `active_pool_ptr()`. Consequences, all three arms agreeing: a cross-pool joiner is
EXTERNAL to the target pool; a worker inside `pool.install` of ITS OWN pool is external to it for
that frame (its spawns go to `injector_global` — today's behaviour; its join takes the external arm;
no count-gated target is set); a worker inside `pool.scope` is that worker's lane. The install-frame
case is not a production route today (`install` on a worker has no caller in the tree; `distance.rs:
433` installs from an application thread) but `install` is public API and the day it is called from
a system body it must not abort the process.

Tests, `tests/cross_pool_routing.rs` gains:

- `cross_pool_join_does_not_drain_foreign_local_slot` (pre-fix: a pool-A worker joining a B scope
  drains B's `injector_local[wid_A]`; post-fix under A2/A3 it cannot; under A1 the slot no longer
  exists — the test asserts the routing receipt).
- `install_on_foreign_worker_routes_to_global` — a pool-A worker task calls `pool_b.install`,
  spawns N, joins. **The receipt is the pair** `(ThreadPool::current_pool() address,
  current_worker_id())` recorded inside every body, the way `tests/ke16_nested_scope_occupancy.rs:
  257-333` already records it: worker ids of A and B both range over `[0, W)`, so an id alone
  cannot say which pool ran a body, and inside `pool_b.install` on the A thread
  `CURRENT_WORKER_ID` is `WORKER_ID_DISPATCHER` (`thread_pool.rs:205`), so a body the B1 external
  joiner runs INLINE on that A thread reports the dispatcher sentinel — a bare "no id in
  `[0, W_A)`" assertion would pass while a body did run on an A thread (the critic's non-blocking
  item 8). Assert: every body's receipt is `(B, wid < W_B)` (a B worker) or `(B, DISPATCHER)` (the
  inline joiner inside B's install frame); never `(A, _)`. Under B3 the second form must not occur.
- `install_on_same_pool_worker_is_external` (a pool-A worker task calls `pool_a.install`, spawns N,
  joins; asserts completion within a bound and — the row that catches the round-2 OOB — that the
  process is alive afterwards under `ke16-w-count`).

`tls.rs` unit test: a deposited deque with `CURRENT_WORKER_ID = WORKER_ID_DISPATCHER` →
`worker_lane_for` is `None`; with a worker id → `Some`.

## 6. App-7 — the timed-wait resolution (axis 36), measured, and the decision

`park_timeout(Duration::from_micros(50))` at `scope.rs:512` and `PARK_TIMEOUT = 100 µs` at
`schedule.rs:69` reach `WaitOnAddress` through `dur2timeout`, which rounds nanoseconds UP to whole
milliseconds (toolchain `std/src/sys/pal/windows/mod.rs:240-254`, `sys/sync/futex/windows.rs:57-68`
`[L]`); nothing in the tree calls `timeBeginPeriod`/`NtSetTimerResolution`. The wait is ≥1 ms,
bounded above by the system timer resolution in effect, which is unmeasured.

**Measure:** a criterion group `ke16_park_timeout` in `crates/boyko_threadpool/benches/
ke16_nested_scope.rs` (Step 0): rows `park_timeout_50us`, `park_timeout_1ms`, `park_timeout_2ms`,
each `b.iter(|| std::thread::park_timeout(d))` with no pending unpark; criterion's median IS the
expiry latency and the three rows make the timer quantum visible.

**Decision (perf, taken here):** do NOT call `timeBeginPeriod(1)`. Microsoft documents the cost
("the thread scheduler switches tasks more often … can also prevent the CPU power management system
from entering power-saving modes" `[D]`), and the pass makes the backstop non-load-bearing on the
route that ships instead (W-d′ §3.4: the last completer's unpark is exact on route (b); the joiner
under B1 parks only after exhausting stealable work). The external arm keeps its window
(`KE16-DESIGN-W.md` §3.5), snooze-masked on the frame path. The constants stay; their comments say
"≥1 ms on Windows; defensive" (index §7). If, after the pass, a production trace shows a joiner's
park expiring rather than being woken (a `boyko_diag` counter on the backstop-expiry path is a
one-line follow-up), raising the resolution is the owner's VALUES call (index §6 Q4). A
spin-until-deadline backstop is rejected: it burns the core the wave is trying to use.

## 7. App-8 — `InSystemRunGuard` becomes a depth counter

`tls.rs:49,83-84,184-203`: `IN_SYSTEM_RUN: Cell<bool>` → `Cell<u32>`; `enter` increments
(`debug_assert!(depth < 64, "InSystemRunGuard depth runaway")` replaces the no-nesting assert);
`drop` decrements; `is_in_system_run()` = `depth > 0`. Every consumer is a boolean predicate
(`KE16-DESIGN-B.md` §2.5) and is unchanged. The doc comment's "nested system runs are a contract
violation under SCH7" becomes: "a helping joiner may run a sibling conflict-free system inline
inside a system body (reachable today through the global-injector drain, likely under B1); SCH7's
apply window is unaffected because both systems are in `running` and the gate counts completions".
`tests/miri_phase9.rs:84-121` guard tests gain a depth-2 case; the ECS nested-system test of index
§8 is the behavioural gate.

Two per-lane facts travel with this (both stated in `KE16-DESIGN-B.md` §2.5): a profiler sees two
overlapping `SystemSpan`s on one lane; and EVT1's "single writer per lane"
(`event_dispatcher.rs:269-272`, `:524-536`) is per THREAD, so a sibling system run inline appends
to the SAME event lane — sequentially and soundly (still one writer per lane at any instant), but
the two systems' events INTERLEAVE within that lane. No reader in the tree relies on per-system
contiguity within a lane (`event_reader.rs:111,176,192` treat a lane as an opaque sequence); the
sentence goes into the EVT1 doc comment so a future reader is warned.

## 8. App-9 — record the bench box's topology (M1-0)

One PowerShell command at Step 0, output pasted into `KE16-RESULTS.md` §0:

```powershell
Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors,L2CacheSize,L3CacheSize,MaxClockSpeed | Format-List
```

plus `[Environment]::ProcessorCount` and the Windows build (`[Environment]::OSVersion`). Core-class
heterogeneity (P/E) and L2-cluster sharing are not in that output; if the `Name` is a hybrid part,
note it — the equal-row `par_iter` chunks are equal work only on homogeneous cores (L12).

## 9. Doc corrections

The table in `KE16-DESIGN.md` §7 is the checklist; each row is edited in the same commit as the code
it describes (the memory note on doc rot applies: use `git log -G` on the quoted phrase, not `-S`,
when a comment's origin is needed). Rows new in revision 3: `sync.rs:39` ("no production code uses
`core::sync::atomic::fence`" — false once `publish_fence` exists, and `fence` joins the shim); the
`tests/loom_pool.rs:143-166, 208-217` fidelity note (the producer's fence is production code, the
consumer's is the STEAL path's); `worker.rs:302-319` (`unpark_one_idle`'s algorithm comment gains
the fence and the `exclude` mask). Rows from revision 2: the `tests/loom_pool.rs` "#246" sentence
(loom 0.7.2 persists the token) and the `tests/miri_scope.rs:56-59` scoping (external arm only).

## 10. Harness: receipts, un-ignore, commit

- **Receipts added before Step 0** (`KE16-DESIGN-MEASUREMENT.md` §6): `outer_worker_id` in
  `benches/ke16_nested_scope.rs::worker_wave` (assert `< MAX_WORKERS` after each wave, else panic:
  "the outer task ran on the dispatcher; healthy route measured twice"); a worker-id receipt inside
  the system closures of `boyko_ecs/benches/ke16_par_iter_in_system.rs::build_schedule` and
  `boyko_physics/benches/ke16_solve_in_system.rs::spawn_solve_schedule` (assert `< MAX_WORKERS`);
  the `ke16_variant()` witness + `KE16_EXPECT` check in all three benches and both red-first gates;
  the physics bench's `bench_thread_install_Wminus1` row (§11); `ThreadPool::parked_mask() -> u64`
  (a public read-only diagnostics snapshot: one `Acquire` load of `inner.idle`; used by the B1-P
  receipt test, `KE16-DESIGN-B.md` §2.7, and available to the occupancy harness for a
  "parked right now" column).
- **Un-ignore at Step App**, by NAME. `crates/boyko_threadpool/tests/ke16_nested_scope_occupancy.rs`
  holds THREE tests (`nested_scope_occupancy_numbers_are_recorded_for_both_routes` at `:381`,
  `worker_route_outer_task_runs_on_a_registered_worker_of_the_installed_pool` at `:418`, and the
  ignored `worker_spawned_wave_reaches_at_least_half_the_workers` at `:446-447`);
  `crates/boyko_ecs/tests/ke16_occupancy_gate.rs` holds TWO (`par_iter_from_dispatcher_reaches_more_
  than_one_thread` at `:122` and the ignored `par_iter_in_system_reaches_more_than_one_thread` at
  `:167-168`). An UNFILTERED run of either file prints `running 3 tests` / `running 2 tests` — that
  is the expected count for the file, not a defect. The `running 1 test` reading applies to the
  FILTERED invocation of the un-ignored test:

  ```powershell
  cargo test -p boyko-threadpool --test ke16_nested_scope_occupancy worker_spawned_wave_reaches_at_least_half_the_workers -- --test-threads=1 --nocapture
  cargo test -p boyko-ecs --test ke16_occupancy_gate par_iter_in_system_reaches_more_than_one_thread -- --test-threads=1 --nocapture
  ```

  Each must print `running 1 test` … `test result: ok. 1 passed` (a `running 0 tests` line is a
  filter that matched nothing — a renamed test — and is a refused shape). The unfiltered runs must
  then read `running 3 tests` / `running 2 tests` with every name listed, and the census
  `tests/ignore_reasons_census.rs` must still pass (two fewer ignores). Compare NAMES, not counts.
- **Commit** the five instrument files and the three `Cargo.toml` hunks with the pass (owner
  question 1).

## 11. Re-taking the O-series colored-solve numbers on the shipping route

Every O-series colored-solve number on record was taken from the bench thread
(`benches/parallel_solve.rs`, `benches/colored_solve.rs` — the healthy route). After the verdict,
`ke16_solve_in_system`'s `in_scheduled_system` row IS the shipping number; record it, with
`bench_thread_install`, `bench_thread_install_Wminus1`, `empty_schedule_control` and
`single_threaded_O5` beside it, in `docs/threadpool/KE16-RESULTS.md` §App, and add one dated line
to the O-series record that cites it (find the record with `grep -rln 'parallel_solve' docs` — the
bench header names it "O6 Gate 10").

**The `bench_thread_install_Wminus1` row is mandatory** (the critic's round-2 non-blocking item
5): the same `bench_thread_install` shape with `ThreadPoolBuilder::num_threads(W − 1)` (one more
`benchmark_group` row in `crates/boyko_physics/benches/ke16_solve_in_system.rs`, a second pool built
once beside the W-pool at `:343`). The bench-thread route's structural advantage over the scheduled
route is AT MOST (W+1)/W (its extra lane is oversubscribed on a W-hardware-thread box, so the true
ratio lies between 1 and (W+1)/W), and an allowance of (W+1)/W would loosen a `≤` line by up to
6.25 % at W=16 — a pass inside that margin is not evidence the fix is complete. The raw comparison
against a W-LANE reference is the falsifiable one.

**Which row IS the W-lane reference depends on the shipped B (revision 4; the critic's round-3
blocking item 2).** The lane count of each row is a function of whether the bench thread's
EXTERNAL joiner helps:

| shipped B | external joiner | `bench_thread_install` (W workers + joiner) | `bench_thread_install_Wminus1` (W−1 workers + joiner) |
|---|---|---|---|
| B0, B1 | helps | W + 1 lanes | **W lanes — the primary reference** |
| B3 | parks | **W lanes — the primary reference** | W − 1 lanes |

Under B3 the two rows do not coincide: they differ by exactly one lane, in the direction that
would LOOSEN the line by up to 1/(W−1) ≈ 6.7 % at W=16 if the W−1 row were kept as the reference
— the very allowance this section exists to remove. So the reference is chosen by LANE COUNT, not
by row name: `REF = bench_thread_install_Wminus1` if the shipped B keeps the external helper (B0,
B1); `REF = bench_thread_install` if it parks (B3). The tester writes the row name chosen and the
lane-count reasoning into the results file beside the number.

The acceptance line the owner reads has two parts. (1) `in_scheduled_system < single_threaded_O5`
(31.52 ms today): parallel physics is finally faster than leaving it off on the route that ships.
(2) **Primary:** `in_scheduled_system − empty_schedule_control ≤ REF × (1 + band)` — W lanes
against W lanes, no structural allowance. Reported beside it, raw, for information: the ratio
against the OTHER row (W+1 lanes under B0/B1; W−1 lanes under B3 — where
`Wminus1 / bench_thread_install ≈ W/(W−1)` is itself a receipt that the external joiner parked).
Under B0/B1 the W−1 reference's helper lane is a weaker lane (no own deque, one CAS per task under
B1; the ≤33 scratch batch under B0) than the scheduled route's joiner-worker (own deque under B1),
so (2) is a conservative line in the scheduled route's favour only by that difference — the tester
reports both ratios raw.

## 12. Backlog

At pass end the orchestrator updates the KE16 row in `docs/aether-v2/KERNEL-BACKLOG.md` (status,
the winning configuration, the two un-ignored gates) and opens KE17 (§3). This design does not edit
that file.
