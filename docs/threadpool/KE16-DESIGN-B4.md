# KE16 — B4: a remedy for defect B on the `a3` substrate

Written 2026-09-07 against `feat/threadpool-ke16` @ `7f294afe`. This is the re-plan that
`KE16-DESIGN-B.md`'s B1(i) row named in advance: *"A2/A3/A5 wins Step A beyond the band (then the
worker joiner also has no registered deque) — a re-plan point, not a fallback taken silently."*

**Its answer is that the premise in parentheses is false.** See the 2026-09-07 correction in
[KE16-RESULTS.md](KE16-RESULTS.md) §B and the rewritten `compile_error!` at `src/lib.rs:115`.

## 0. Corrections to the framing this re-plan started from

| Statement | Status |
|---|---|
| "`a3` sends every spawn to `injector_global`" | Correct. `place_task` returns before the lane test: `worker.rs:1059-1061`. |
| "the joiner batch-steals into a `scratch` buffer only it drains" | Correct. `scope.rs:1262` (`let scratch: Worker<Task> = Worker::new_fifo();`), consumed at `:1310`, `:1318`, drained whole at `:1744-1751` with no `is_drained` in between. |
| "batches of <= 33" | Correct for the injector, wrong for sibling deques. `Injector::steal_batch_and_pop` passes `MAX_BATCH + 1` = **33**; `Stealer::steal_batch_and_pop` passes `MAX_BATCH` = **32**. |
| "the worker joiner needs a registered destination deque, and only A1 gives it one" | **FALSE.** Every arm builds and registers the deques (`thread_pool.rs:718-737`); each worker gets one by move (`worker.rs:33`). Only the TLS deposit is A1-gated (`worker.rs:60-61`), and `tls::worker_lane_for` already has a working non-A1 branch (`tls.rs:238-248`). |
| "`top_lane = 33/31/24` with `lanes_used = 16`" | **Not in the corpus.** The recorded `a3` reading is `top_lane = 33` on all three reps with `lanes_used = 15/16/15` (`KE16-RESULTS.md` appendix). `13/24/21` is `a1`'s row. |

**The lever the framing omitted.** On the occupancy fixture `wall ~= top_lane * BODY` in every mode:
33 -> 6.682/6.656/6.661 ms; 5/7/5 -> 1.03 ms. `BODY = 200 us` (`tests/ke16_nested_scope_occupancy.rs:67`),
tasks = `4W` = 64 (`:71`), serial floor 12.8 ms. On this fixture `top_lane` is not an instrument that
can disagree with the clock — it **is** the makespan. That lets the receipt be written in wall-clock
terms and sidestep the "occupancy lost the ranking" objection entirely.

## 1. What hoards, and why it is a REACHABILITY defect

Two paths, both in the B0 joiner, both feeding the same unregistered sink:

* **P1 — the injector grab.** `scope.rs:1310`. With head and tail straddling a block boundary the
  batch is `advance = (BLOCK_CAP - offset).min(33)` — exactly 33: one popped, 32 into `scratch`.
  That reproduces the recorded `top_lane = 33` to the digit.
* **P2 — the self-steal.** `try_steal_any` (`scope.rs:1777-1787`) sweeps `0..n` **including the
  joiner's own index**, taking `min((len+1)/2, 32)` from `stealers[wid]` into `scratch`. Under `a3`
  worker X's deque is not empty: `pop_global_injector` (`worker.rs:272`) parks up to 32 stolen tasks
  there before X entered the outer body. The joiner batch-moves its own re-stealable residue into a
  sink nobody can reach.

Workers hoard too (`worker.rs:272` also takes 33) — **but into a registered deque**, so siblings take
it back via `try_steal_random` (`worker.rs:316`). **That asymmetry is the entire defect.**

### Measured, not inferred

`the_worker_joiner_does_not_run_a_residue_before_re_checking_its_scope`
(`tests/ke16_b_join_arms.rs:343`) under `--features ke16-a3`, five runs, `running 1 test` each:
192142 / 192205 / 192111 / 188129 / 192133 us, **median 192.1 ms**, against a 60 ms budget
(`:310`) and a 132 ms B0 residue floor (`:301-305`). `ke16-a1,ke16-b1` records 4 us and
`,ke16-b3` 5 us. The separation is four orders of magnitude.

### External corroboration

A survey of Rayon, Go, Tokio, Java ForkJoinPool, TBB, Cilk-5, .NET and `async-executor` found **no
production scheduler that batch-steals into a private buffer only the stealer can drain.** They
either steal one task (Rayon — current `rayon-core/src/registry.rs` contains zero `steal_batch`
calls — FJP, TBB, Cilk, .NET) or batch into a destination that remains stealable by everyone (Go's
`runqsteal` into the P's own runq, whose `runqgrab` is documented *"Can be executed by any P"*;
Tokio's `steal_into2`; `async-executor`'s registered `Runner` queue). The three "return the surplus"
mechanisms in the wild (Go `runqputslow`, Tokio `push_overflow`, `async-executor`'s `Drop for
Runner`) are all **owner-side overflow valves**, never a stealer giving work back after a steal.

Our own `tests/shutdown.rs:24-37` already records the consequence of the private buffer — a task
blocking on a peer that sits unrun in the same `scratch` deadlocks — and states that both B arms
remove it.

### Two constraints any remedy inherits

1. **`Injector`'s "steals about half" rule applies only within one 63-slot block.** It is a linked
   list of blocks; once head and tail are in different blocks the formula is
   `(BLOCK_CAP - offset).min(limit)` — a flat 33 per call. Reasoning of the form "it only takes
   half, so it self-limits" is wrong for any real wave.
2. **The loom models are written against `steal_batch_and_pop` specifically**
   (`tests/loom_pool.rs:208-236`, `:330`), citing the `SeqCst` fence it carries. Changing the
   joiner's steal call changes what those models cover, and that is an exit condition, not a detail.

## 2. Options, and why five of six lose

| Option | Mechanism | Verdict |
|---|---|---|
| **(c) `injector_global` as the joiner's destination** | — | **Structurally impossible.** Every crossbeam batch API takes `dest: &Worker<T>`. There is no `Injector` destination. Rejected on the API, not on taste. |
| **(b) push the surplus back to `injector_global`** | steal 33, run 1, re-push 32 | **Dominated.** `Injector::push` is a `tail.index` CAS per item: 33 shared RMWs to consume one task, on the very line `a3`'s win depends on. No prior art exists for a stealer returning surplus. Rejected. |
| **(b') push the surplus to a sibling's `injector_local`** | A5's shape | **Dead queue under `a3`.** `worker_main` stage 1 and `pop_any` stage 1 are both compiled out (`worker.rs:80`, `:234`). Nothing would drain it. Rejected. |
| **(d) do not batch at all on the worker route** | `steal_batch_with_limit_and_pop(dest, 1)`, or `Injector::steal()` | **Viable, unnecessary, and it trips the granularity clause.** With `limit = 1`, `batch_size = 0`, so it is atomically identical to `Injector::steal()`. It is what Rayon/FJP/TBB/Cilk do. But it discards `a3`'s amortisation on the one thread guaranteed to be consuming, and once (e) makes the residue reachable there is nothing left for it to fix. Kept as B4-3. |
| **(a) bound the batch at the joiner** | `steal_batch_with_limit_and_pop(&scratch, L)` | Buildable today (the limited APIs are `pub` since crossbeam-deque 0.8.3). But it fixes the defect by *taking less* rather than by *making the surplus reachable*, and it is squarely "a smaller batch cap". Retained as B4-3. |
| **(e) give the joiner the deque it already owns** | publish the TLS deposit under `a3`; dispatch the worker joiner to the existing `join_on_worker` | **RECOMMENDED (B4-1).** |

### If B4-3 is ever needed, what `L` should be a function of

1. **`L = max(1, ceil(pending / W))` — the joiner's fair share of its OWN wave.** `pending` is
   already `Acquire`-loaded at the top of every join iteration (`scope.rs:1452` -> `is_drained`,
   `:709-711`); reading the value instead of the predicate costs **zero extra shared traffic**. It is
   also correct in the foreign-wave regime: when the injector holds someone else's 96-task wave and
   the joiner's own `pending` is 1, it takes 1.
2. `L = f(injector length)` — Go's `globrunqget` shape. Requires `Injector::len()`, a three-`SeqCst`
   -load stable-tail retry loop on the contended lines; `worker.rs:802-809` prices exactly this and
   `NoProbe` exists to avoid it. **Reject.**
3. A constant. Wrong at both ends.
4. Lanes alone (`L = 33/W`). Ignores wave size; degenerates to 2 at W=16 for every shape.

## 3. B4-1, concretely

`join_workers_until_drained` becomes B1's two-way dispatch (`scope.rs:1377-1386`) — *code that
already exists and has been through five review rounds*:

* `Some(lane)` -> `join_on_worker` (`scope.rs:1423-1563`) verbatim: step 1 pops the own deque
  (`:1472`), step 2 `pop_global_injector(inner, wid, lane.deque())` (`:1484`), step 3
  `try_steal_random` (`:1494`), step 4 parks idle-marked (B1-P, `:1503-1558`).
* `None` -> **today's B0 body, unchanged.** Not `join_external_helping` — that is B4-2, and it moves
  the acceptance reference.

New code is only the cfg widening: `tls::WORKER_DEQUE`, `WorkerLane::deque`, `WorkerDequeDeposit`
and the deposit branch of `worker_lane_for` (`tls.rs:58, 61, 101, 144, 148, 225, 294, 301, 314`) and
`worker.rs:60-61` go from `any(ke16-a1, ke16-a1-fifo)` to `any(ke16-a1, ke16-a1-fifo, ke16-b4)`.

**Nothing on the spawn path changes**: `place_task` returns at `worker.rs:1060-1061` before the lane
test, so `a3`'s placement is byte-identical.

Four properties this buys that B0 lacks: no sink; an `is_drained` re-check per task
(`scope.rs:1452`) instead of after a whole 33-residue; own-deque-first, which kills P2 outright; and
B1-P, where the parked joiner is a claimable lane at one `fetch_or`/`fetch_and` per *park*.

### Soundness under `a3` is STRICTER than under A1

Discipline D5 (`tls.rs:44-53`) exists under A1 because a task body run inline by the joiner may push
a *nested* spawn through the same TLS deque, so a live `&Worker` must never span a body. **Under
`a3` that hazard does not exist**: nested spawns go to `injector_global` (`worker.rs:1060-1061`),
never through the slot. The only writer of the TLS-reached deque is the steal path, always on the
owner thread. `join_on_worker` is written to D5 anyway, so B4-1 inherits a discipline stricter than
it needs.

⚠ Consequence that must be written down rather than left implicit: the Miri obligation
`nested_scope_inline_body_spawns_through_tls_deque_under_live_join`
(`KE16-DESIGN-B.md:313-316`) is A1-specific and would be **VACUOUS** under `ke16-b4`. It must be
marked so, not run as though it covered something.

## 4. Does the cure cost what `a3` won?

| Site | B0 | B4-1 | delta |
|---|---|---|---|
| worker's injector drain (`worker.rs:272`) | 33 | 33 | none |
| joiner's injector drain | 33 -> `scratch` (`scope.rs:1310`) | 33 -> own deque (`scope.rs:1484`) | destination only |
| joiner's sibling steal | <=32 -> `scratch`, `0..n` incl. self (`scope.rs:1777`) | <=32 -> own deque, random start, self skipped (`worker.rs:297-331`) | destination + order |
| spawn placement | global | global | none |

**The batch cap is untouched at every site** — mechanically checkable: `grep steal_batch` must show
no `_with_limit` call and no arity change.

New shared traffic B4-1 does add: `lane.deque().pop()` per task the joiner runs. Under `a3` the
deques are `new_fifo()` (`thread_pool.rs:733-734`), and a FIFO owner pop contends with thieves on
`front`. That is one multi-writer RMW per task the joiner **executes** — about `n/W`, i.e. 6 of 96 on
the physics dispatch, not 33. Against B0's 33 serial 200 us bodies it is not a contest; at 1 us
bodies it is unmeasured and is the risk to price.

### The plain statement

**B4-1 does not trip the letter of `a1f`'s return condition** (`KE16-REJECTED.md:157-164`, `:471`:
"any change to steal granularity — a smaller batch cap, a steal-half policy, or the C-axis batch
spawn"). It is none of those.

**It does change the shipped `a3`.** The head-to-head measured `a3+b0` vs `a1f+b0` on the worker
route, and the joiner is on that route. The verdict survives without a re-run only under a monotone
argument, whose thresholds are:

| Row | `a3+b0` | `a1f+b0` | headroom before `a1f` is in contention |
|---|---|---|---|
| physics `in_scheduled_system` | 11.709 ms | 13.640 ms | **16.5 %** |
| worker `body_10us_tasks_4W` (deciding cell) | 114 960 ns | 159 470 ns | **38.7 %** |

* faster or tied -> the verdict holds *a fortiori*; nothing is owed.
* slower but inside the headroom -> `a3` still beats `a1f+b0`, but the register row must be rewritten.
* outside the headroom -> the axis-A verdict is void, and the correct re-run is **`a3+b4` vs
  `a1f+b1`**, not vs `a1f+b0`, because `a1f` is the arm that makes `b1` buildable.

**B4-2 and B4-3 trip the clause on its letter.** B4-2 takes the external joiner from a 33-batch to
steal-one, which changes granularity on the fontbake consumer **and moves the physics acceptance
reference row** (`REF = bench_thread_install_Wminus1` holds only while the external joiner helps);
clause (2) currently sits at 93.5 % of budget, so a REF that moves 6.5 % flips it. B4-3 is a smaller
batch cap on the worker route, i.e. the clause verbatim.

## 5. Receipts

### Why the obvious receipt does not work

`top_lane` for `a3` reads 33/33/33, then 16, then 5/7/5 across processes while
`worker_spawned_wave_reaches_at_least_half_the_workers`
(`tests/ke16_nested_scope_occupancy.rs:474-507`) is green in all three. That gate asserts
`max_in_flight >= W/2` — a quantity that is 15/16 in **every** mode, so it is structurally blind to
defect B and cannot become the B gate by tightening a constant.

The bimodality is not jitter: `wall ~= top_lane * BODY` in both modes. It is two different races —
whether the 15 siblings have woken and drained the injector before the joiner reaches `Scope::drop`.
So the right statistic is the **envelope, not the centre**.

### Leg 1 — route receipt (structural, load-insensitive)

The discriminator for "the worker joiner took the new path" is **B1-P's idle bit**: only
`join_on_worker` marks it (`scope.rs:1504`); B0's joiner explicitly does not (`:1336-1339`).
`tests/ke16_nested_scope_occupancy.rs:535` and `tests/ke16_b_join_properties.rs:313` already assert
this and are deterministic by construction; widen their cfg to include `ke16-b4`.

*Why this leg exists*: `off_pool` **cannot** discriminate — under the worker route it is 0 whether
the joiner ran `join_on_worker` or fell through to B0. A cure whose TLS deposit was silently written
as `let _ = WorkerDequeDeposit::new(..)` (the statement-scoped-drop defect `tls.rs:691-705`
documents) would produce a plausible number with the B0 body running.

### Leg 2 — the cure, and the test already exists and already fails

`tests/ke16_b_join_arms.rs:343` runs in every build, selects its assertion from `KE16_B` at run
time, and is **not bimodal by construction**: the joiner is guaranteed to be running when the
injector holds 96 x 4 ms foreign bodies. Recorded: `a1+b0` 192 ms, `a1+b1` 4 us, `a1+b3` 5 us;
budget 60 ms; B0 residue floor 132 ms. **Measured 2026-09-07 under `a3+b0`: median 192.1 ms of five**
— the model transfers exactly. Flipping `KE16_B` to `"b4"` arms the assertion.

### Leg 3 — the distribution receipt, over the envelope

Pre-registered rather than invented: `KE16-DESIGN-MEASUREMENT.md:268-269` already says *"on the
worker route `top_lane <= 2 * tasks / W` for the winner at 200 us x 4W; B0's ~33-of-64 signature must
be gone unless B0 won."* At W=16 that is `top_lane <= 8`.

Stated in the clock instead, which is what ranks: worker route, W = `available_parallelism()`,
tasks = 4W, `BODY = 200 us`, serial floor 12.8 ms; pass iff **`speedup_vs_serial_floor >= W/2`**
(wall <= 1.6 ms at W=16). `a3+b0`'s recorded 6.66 ms is 1.92x -> RED; the good mode's 1.03 ms is
12.4x -> GREEN. The gate separates the two observed modes exactly, and it is a throughput statement.

**Pass condition is over the WORST sample, never the median.** A bimodal quantity whose bad mode *is*
the defect is characterised by its upper tail; taking the max can only make the gate redder.

**Sample shape**: the recorded modes look **process-correlated** (33/33/33 within one process, 5/7/5
within another), so within-process repeats are not independent. Requirement: **>= 3 separate
processes x >= 10 waves each, max over all 30**. Whether the mode is per-process or per-wave is
**UNMEASURED** and is rung B4-0(2); if per-process, the process count *is* the sample size.

### Leg 4 — the ranking, and it can veto legs 1-3

Legs 1-3 green is necessary and **not sufficient**. The remedy is REJECTED if, measured
**interleaved pass-by-pass in one session** (session drift, not ambient load, was this campaign's
dominant error term):

* it regresses any of the six 1 us veto cells beyond the 2x band, or
* it regresses physics `in_scheduled_system` beyond the band against `a3+b0`, or
* it regresses `worker/body_10us_tasks_4W` at all beyond the band.

**This is how "flattened the wave" is told from "won".** A remedy that takes `top_lane` from 33 to 4
and costs 20 % on physics is a rejected remedy.

### The regressions each leg must go red on

| # | Regression | Which leg reds | Why it is the right test |
|---|---|---|---|
| R1 | Build `--features ke16-a3` with no `ke16-b4`. | 1, 2, 3 all red. | Red-first, one build. |
| R2 | Keep `ke16-b4`, but steal into a stack-local `Worker` instead of `lane.deque()`. | **3 red; 1 and 2 stay GREEN.** | The mutation that matters: shape right, B1-P still marks, re-check still happens, only reachability gone. If leg 3 does not red here, it is not measuring reachability. |
| R3 | `let _ = WorkerDequeDeposit::new(..)` — the statement-scoped drop at `tls.rs:691-705`. `worker_lane_for` answers `None` forever, dispatch silently falls to B0, and `KE16_B` still prints `b4`. | **1 red**, 2 red, 3 red. | The quiet mislabelling the KE16 witness exists to forbid, and `off_pool` cannot see it. Leg 1 exists solely for this. |
| R4 | Delete the `is_drained` re-check at `scope.rs:1452`. | **2 red**; 1 and 3 possibly green. | Separates *reachable* from *re-checked* so neither certifies the other. |
| R5 | Declare `ke16-b4` with no arm. | Every leg red, and `ke16_check_expected_variant` still certifies. | Why the `compile_error!`-until-the-arm-lands discipline is re-armed for `ke16-b4`. |

## 6. Staging

### B4-0 — measurement only, no code. Blocking.

1. ✅ **DONE 2026-09-07**: the residue test under `--features ke16-a3` reads **192.1 ms** (median of
   five). The model holds.
2. The bimodality census: `a3+b0`, worker route, W = hardware, 3 reps x 30 processes, reporting
   max/median `top_lane` and `wall`, and whether the mode is constant *within* a process. This fixes
   leg 3's sample size and is unknowable a priori.
3. `a3+b0`'s six 1 us veto cells and the deciding cell recorded in the **same session** that will
   later carry `a3+b4`. Interleaved, or the comparison is worthless. **This is the only item that
   needs a quiet machine.**

### B4-1 — publish the lane; worker joiner only. The rung.

Widen the cfgs; dispatch on `tls::worker_lane_for`; `None` keeps today's B0 body verbatim.
`KE16_B = "b4"`; a `compile_error!` refusing `ke16-b4` until the arm lands; mutual exclusion with
`ke16-b1`/`ke16-b3`. Widen the two B1-P receipts' cfgs. **Zero change to the spawn path, to any batch
cap, and to the external joiner.**

### B4-2 — the external joiner. Separate, because it moves the acceptance reference.

`join_external` -> `join_external_helping` (`scope.rs:1586-1620`). Ship only after B4-1 is green and
only with the physics acceptance line re-run.

### B4-3 — bound the joiner's take. Conditional.

Only if B4-1 leaves a measured residual. Trips `a1f`'s clause on its letter. **Do not build it
speculatively: if B4-1 puts leg 3 at `top_lane <= 8`, this rung is deleted, not deferred.**

### Deferred, named so it is not silently dropped

* **Residue left behind when a joiner returns.** B4-1's joiner may return to its task body with up to
  32 tasks in its deque. Under `w0` every push wakes, so the next push recovers them; under a future
  `ke16-w-gate` the thief-residue cascade (`worker.rs:645-656`) covers it. Between the two there is
  no site that wakes on that transition. **W-axis work, and the W axis is unmeasured.**
* **`try_steal_random` early-returns `None` at `n <= 1`** (`worker.rs:304-306`), where B0's
  `try_steal_any` did not. At W=1 step 1's own-deque pop covers it; add W=1 to leg 2's widths.
* **B1(i) registered `scratch` stays refused.** B4-1 does not approach it: the registry is fixed at
  `build()` and never withdrawn, so no hazard-pointer or epoch protocol is introduced.

## 7. Open questions for the owner

1. **Is the do-nothing option live?** `a3+b0` clears clause (1) by 2.4x and clause (2) at 93.5 % of
   budget with defect B live. B4-1's cost is one TLS store; its benefit on the occupancy fixture is
   6.66 ms -> <=1.6 ms. Recommendation: fix. The number that would justify inaction — B4-1's effect
   on physics — does not exist and is rung B4-0(3).
2. **`new_lifo()` deques under B4?** The joiner's step-1 `pop` on a FIFO deque is a contended `front`
   RMW. LIFO makes the owner's pop local but changes every *worker's* pop too, which is
   placement-adjacent and would need its own row. Not decided here; recorded.
3. **B4-2's acceptance-line consequence** is a VALUES call, not a perf call: it moves `REF` and
   clause (2) sits at 93.5 %.
