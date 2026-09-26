DESIGN READY (rev 1): T1/T8 2.72 vs Jolt 3.83 is fully explained by our serial time (1.20 ms against Jolt's ~0.58 ms) and our 1.9× smaller parallel work, at equal parallel efficiency (E(8) 0.62 against 0.60). Seven bit-identical levers (S1–S7; the core is a Box2D-v3-style solve region whose stage table stays fixed while it runs, replacing ~109 colour scopes per step) take the post-C4 W8 step from 1.97 (+0 to 0.16) ms to **1.01–1.65 ms (0.75–1.22× Jolt per manifold; parity inside the bracket, not guaranteed)**, plus one measurement window that sizes them and demonstrates the W8/W16 resolution first.

# W8+ scaling plan: why our T1/T8 is 2.72 against Jolt's 3.83, and the ranked levers for W8/W16

Architect, 2026-09-26 (design, then rev 1 answering its architecture review); rulings the same day; committed
on `u/phys-w8s` with the W8S instrument. This is the design with every replacement of rev 1 applied in place,
one document. Read-only analysis of trunk `integ/unified` at `d8262be1`: **every code anchor in §1–§8 is a line
of `d8262be1`**; §12 relocates them to the tree the lane built on. This file is not in
`tests/internal_docs_anchors.rs`'s `GATED_DOCS`, so its anchors are not machine-checked.

No cargo, no build and no timing ran for the design. Every number cites a measured source. A number marked
**(arith.)** is arithmetic on measured numbers, which is an estimate and not a measurement.

**Row names used throughout:**
- **J-T** is the Jolt headline row: `--cfg default --broadphase tree`, sleeping off.
- **J-A** is `--cfg a`: AllPairs pinned under `Manual`, scalar colours (`simd_solve` off).
- Both run with contact reuse on since L9 C4 unless a row says "off".
- Window 7 (`docs/measurements/2026-09-25-physics-window7/analysis.md` plus `analyst/tables.txt`) ran on trunk
  `93b2615b` with reuse off, msvc, on a Ryzen 9 5900HS: **8 physical cores / 16 SMT threads, one 8-core CCX**
  (`docs/threadpool/KE16-RESULTS.md` §0 "Topology").

---

## 1. The answer in one paragraph

**At W8, 56 % of our step is serial.** Our W8 step (J-T, reuse off, 2.219 ms) spends 1.252 ms on serial stages:
- bp 0.268, solve_build 0.312, warm_apply 0.308, graph 0.117, the integrate group 0.122;
- gather/apply 0.040, narrow colours 0.014, the narrowphase's serial pieces 0.035, and the executor gap 0.036.

That is 56 % of the step, flat from W1 (none of these stages gets faster with W). The parallel work is 0.967 ms
of wall for 4.83 ms of W1 work (E(8) = 0.62).

**Jolt has half our serial time.** Its W8 step (2.51 ms) has about 0.58 ms of serial time: the deterministic
contact sort plus the large-island split, done by one thread while the other seven yield inside the solve job
(its broadphase refit runs concurrently, so it is not serial; §4.3). The other ~1.9 ms is parallel work of
about 9.0 ms, at E(8) ≈ 0.60.

**So our parallel machinery is not worse than Jolt's.** Two things produce the whole T1/T8 gap:
1. our serial time is about twice Jolt's;
2. our parallel work is 0.54× Jolt's, because we carry 0.53× the manifolds and our AVX2 colour solve is
   186–224 ns per manifold cheaper at W1.

Swap in Jolt's value for both and our model gives T1/T8 = 3.94, against Jolt's measured 3.83 (§4.3).

**T1/T8 is therefore the wrong objective.** It punishes a cheap W1: L9 C4, now in the trunk, lowers our T1/T8 to
about 2.18 while it makes W8 faster (arith.). The objective is **T(8) and T(16) per manifold** (ruling 4).

**The gap is in the serial stages.** After C4 our W8 step is about 1.97 ms (arith.), against Jolt's 301 ns per
manifold × 4,467.66 manifolds = 1.345 ms at parity. What remains is the serial stages. The ranked levers in §6
move each one onto the parallel machinery, bit-identically:
- the central lever (S1) is a persistent **solve region**: one scope per solve instead of ~109, a spin barrier
  between stages, and atomic block claiming, which is the shape of Box2D v3 and of Jolt's split-island batches;
- S1 is what makes the warm apply (S2) and the integrate group (S3) parallel for free.

---

## 2. Our step at W1 and W8, stage by stage

### 2.1 The spans

Source: window 7 P4, reading A (the per-process mean; the median over K = 6), trunk `93b2615b`, J-T with reuse
off, [100,500). Code sites are in `d8262be1`.

| stage | code | W1 ms | W8 ms | W1/W8 | at W8 |
|---|---|---|---|---|---|
| gather + select + apply | `systems.rs:229`, `:2011` | 0.0389 | 0.0398 | 0.98 | serial |
| **bp** (tree): query / assemble / build / verify | `systems.rs:589` (`tree.step`) | 0.2656 (0.2084 / 0.0289 / 0.0219 / 0.0064) | **0.2684** (0.2088 / 0.0309 / 0.0221 / 0.0064) | 0.99 | serial, on the calling thread |
| np | `systems.rs:702`, `narrowphase/dispatch.rs` | 2.4274 | 0.3823 (dispatch 0.3475, compact 0.0190, axis commit 0.0047, rest 0.0111) | 6.35 | one parallel wave, plus 0.035 serial |
| **graph** (union-find + greedy colouring) | `systems.rs:1824` | 0.1153 | **0.1166** | 0.99 | serial |
| **solve_build** (P-a, P-b, P-c fill) | `solver/colored.rs:4098` | 0.2841 | **0.3122** | 0.91 | serial |
| **warm_apply** (4× per step) | `colored.rs:4257` | 0.2990 | **0.3080** | 0.97 | serial |
| colours wide (4 substeps × (biased + 2 relax) = 12 passes) | `colored.rs:4263`, `:4299`, scope at `:3646` | 2.4340 | 0.6193 | 3.93 | 109.32 waves per step (P0), one `pool.scope` each |
| colours narrow + restitution | inline, below `MIN_PARALLEL_SLOTS_PER_COLOR` = 256 (`colored.rs:288`) | 0.0126 | 0.0144 | — | serial |
| **integrate group**: gravity / integrate / store / write_back | `colored.rs:4243`, `:4281`, `:4327`, `:4376` | 0.0978 | **0.1216** (0.0125 / 0.0808 / 0.0256 / 0.0027) | 0.80 | serial (gravity and integrate run 4× per step) |
| g + r + u (executor gap and unzoned) | — | 0.0492 | 0.0360 | — | between systems |
| **wall** | | **6.0239** | **2.2187** | **2.72** | |

**How the stages are wired:**
- The physics block is a strict chain of eight systems (`plugin.rs:771-872`): integrate (gated off for the
  colored solver) → gather → select → broadphase → narrowphase → build_graph → solve_colored → apply.
- Inside the solve, each substep runs `gravity → warm_apply → biased colours → integrate → relax colours ×2`, all
  on the calling thread except the wide colours (`colored.rs:4238-4311`).

### 2.2 The identity at W8

This is the plan's identity (`01-PLAN-REV1.md` §2), recomputed from the table above (arith.):

- **S(1)**, the serial work at W1, is **1.197 ms**. It is 5.9747 − np 2.4274 − wide 2.4340 = 1.1133, plus the
  narrowphase's serial pieces (≈ 0.035, read from the W8 sub-zones), plus the gap and unzoned time (0.049).
- **P(1)**, the parallelizable work, is **4.826 ms** (np 2.392 + wide colours 2.434).
- **I(8) = +0.055 ms.** Serial stages get slightly slower at W8: integrate 0.066 → 0.081, solve_build 0.284 →
  0.312.
- **P(1)/8 = 0.603 ms.**
- **L(8) = 0.364 ms**, the parallel loss: np 0.3475 − 0.2991 = 0.048, and wide colours 0.6193 − 0.3043 =
  **0.315**.
- **T(8) = 1.197 + 0.055 + 0.603 + 0.364 = 2.219 ms**, which matches the measured wall (2.2187).
- The parallel efficiency is **E(8) = P(1)/(8 · t_par(8)) = 4.826/(8 × 0.9668) = 0.62**; the wide colours alone
  are 0.49.

**Where the excess over a linear speed-up goes.** The linear ideal is T(1)/8 = 0.753 ms, and we are 1.466 ms
above it:
- **71 %** is serial work that does not divide: 1.197 × 7/8 = 1.047;
- **25 %** is parallel loss: 0.364;
- **4 %** is interference: 0.055.

**The Amdahl fraction** is s = S(1)/T(1) = **0.199**. The window-7 analysis's 18.4 % counts only the five stages
plus "other". The ceilings follow:
- T1/T8 ≤ 1/(s + (1−s)/8) = **3.34**, against 2.72 measured;
- T1/T16 ≤ **4.02**, against 2.52 measured (block B);
- T1/T∞ ≤ **5.03**.

### 2.3 After L9 C4 (in `d8262be1`; arith., as window-7 §6 does)

**The narrowphase shrinks.** np falls by the armed J-A on/off ratios: W1 2.4274 × 0.2877 = 0.698 ms, and W8
0.3823 × 0.3516 = 0.134 ms.

**The new step times are:**
- T(1) ≈ **4.30 ms** and T(8) ≈ **1.97 ms**;
- per manifold that is **441 ns against Jolt's 301 ns (1.47×)**;
- T1/T8 falls to **≈ 2.18**, while W8 gets faster.

**The serial stages now dominate.** They are about 63 % of T(8), and S(1) is unchanged, so after C4 **the W8
gap is the serial stages**. For parity per manifold, T(8) must reach 1.345 ms, which means −0.63 ms.

**One unexplained term.** With reuse on, J-A's narrow colours rose ×6.5: +0.151 ms at W1 and +0.163 ms at W8,
claimed (window 7 §2.4). Narrow colours are serial. Whether J-T shows the same rise is **unmeasured**, so the
post-C4 J-T baseline is 1.97 ms **+0 to +0.16 ms** (r below).

---

## 3. The dispatch machinery at W8: the wake, barrier and per-wave costs

| cost | value | source |
|---|---|---|
| Physics systems per step | 8, in a strict `.after` chain | `plugin.rs:771-872` |
| Executor gap g(8) | 0.031 ms per step, about 3.9 µs per system boundary | window 7 P4 `g_ns` |
| Wide-colour waves per step | 109.32 (12 passes × 9.11 wide colours) | P0 `ANALYSIS.md` §2 |
| Scopes per step | on J, ≈ 111: 1 install + 1 np + 109.32 colour scopes; each allocates a boxed `ScopeShared` plus a closure cell per spawn. (The 122–134 figures are the census pile's, S1c / S1e, not J's.) | lever rulings, L5 open question 1 and L11 G7 correction; rev 1 O3 |
| Per-wave loss ω(8) = L_wide(8)/waves | **2.88 µs** today (0.315 ms / 109.32, arith.). P0, before L11, read 6.54 µs. ω₁ (no cross-thread wake) = 0.855 µs | window 7 P4; P0 §2 |
| Work per wave at W8 | 22.3 µs of W1 work per wave (2.434 ms / 109.32), 2.8 µs per thread at W8, cut into ≤ 32 tasks of about 0.7 µs | arith.; `colored.rs:3539-3560`, and "a colour spawns at most 32 tasks" (L11 G7) |
| Idle helper policy | crossbeam `Backoff`: 127 `PAUSE` plus 4 `yield_now`, then `mark_idle` and `park` | `boyko_threadpool/src/worker.rs:102-146`; crossbeam-utils 0.8 `SPIN_LIMIT` 6 / `YIELD_LIMIT` 10 |
| Serial gaps between passes | warm_apply ≈ 77 µs per substep, integrate ≈ 20 µs per substep, bp 268 µs, graph 117 µs, solve_build 312 µs | table §2.1 |
| Re-widening after a park | "the thief-residue cascade restores width one unpark-round at a time, too late for the wave" | KE16-RESULTS §W (the `wg` elimination) |
| OS wake cost (external) | "10–70+ µs OS wake" (bevy #8304); on Windows, "double-digit percentages of the frame time just unblocking worker threads" (Schöner) | KE16-EVIDENCE N28, N33 |
| `park_timeout` floor | 15.6 ms (the 64 Hz Windows tick); only the join backstop uses it | KE16-RESULTS §0; `scope.rs:72` |

**The mechanism (inferred from code; the split is not measured).**
- A wave has 2.8 µs of work per thread, so today's per-wave overhead (2.88 µs) is **as large as the work itself**.
- At least 8 of the 12 pass starts per step follow a serial gap of 20–77 µs. That is longer than the helpers'
  backoff, so they have parked, and every such wave pays a wake cascade.
- The narrowphase wave and every stage boundary pay it too.
- L(8) is therefore mostly per-wave overhead, which a spin barrier removes, and partly imbalance at a 0.7 µs task
  grain, which it does not.
- The protocol (§7) splits the two before S1 is sized for good.

---

## 4. Jolt v5.6.0 decomposed the same way

### 4.1 Its job graph (v5.6.0 source, read only in `D:/tmp/jolt/wt-v5.6.0-parity`)

**The whole step is queued at once.** `PhysicsSystem::Update` creates every job of the step up front, with
dependency counts (`PhysicsSystem.cpp:240-576`). Each stage has `max_concurrency` job instances that pull work
through atomic cursors.

**Workers are woken by count, not by a cascade.**
- `QueueJobs` calls `mSemaphore.Release(min(inNumJobs, threads))`: one call wakes n threads
  (`JobSystemThreadPool.cpp:224`).
- Workers block in `Semaphore::Acquire`, which is a Windows semaphore with no spin (`JobSystemThreadPool.cpp:320`;
  `Semaphore.cpp:57-101`).

**FindCollisions fuses the broadphase with the narrowphase.**
- Each job CAS-claims a batch of active bodies, queries the broadphase for them (`FindCollidingPairs`), and
  pushes the pairs into its own lock-free ring. When there is no body work left, it steals pairs from other
  jobs' rings (`PhysicsSystem.cpp:893-1010`).
- Contact constraints are created, with their Jacobians and effective masses, *inside* the narrowphase: 8,488 of
  8,489 manifolds per frame go through "Add Constraint From Cached Manifold" (window 7 §5).
- Islands are linked lock-free during the same jobs (Architecture.md, "Find Collisions").

**The large island is split and solved in batches.**
- `FinalizeIslands` is one serial O(N) job: 0.016 ms.
- In `SolveVelocityConstraints` the first job to take the large island sorts its contacts, "costly but needed
  for a deterministic simulation" (`PhysicsSystem.cpp:1497-1507`), and splits it (`:1515`). The split allows up
  to 32 splits, and the last one is non-parallel; the large-island threshold is 128 constraints
  (`LargeIslandSplitter.h:32-34`).
- Workers then claim batches of 16 (`cBatchSize`, `:171`) through one 64-bit status word. Iteration 0 of each
  split is its warm start (`PhysicsSystem.cpp:1416-1421`).
- **A worker with no batch yields inside the job and never blocks** (`:1554`).
- Integration runs in parallel jobs by body batch.
- The broadphase refit (`UpdateBroadPhasePrepare`) builds a new tree concurrently with the step's start.

### 4.2 Jolt's critical path at W8, and a correction to window-7 §5

**Jolt's jobs run in sequence.** The W8 profile's job walls (`analyst/tables.txt` §5, profiled) are:
- ApplyGravity 0.0098;
- FindCollisions 0.8207;
- FinalizeIslands 0.0158;
- SolveVelocityConstraints 1.8850;
- IntegrateVelocity 0.0057;
- SolvePositionConstraints 0.2330.

They sum to **2.970 ms against the 3.035 ms Update**.

**solve_prep sits inside the solve job's wall.** It is 0.5801 ms of one thread's time, and its union wall is also
0.5801 ms.

**While it runs, the other seven threads are inside `SolveVelocityConstraints` jobs, yielding on
`WaitingForBatch`.** The scope walk therefore books their wait as solve_vel:
- solve_vel cpu is 13.99 ms at W8 against 7.00 ms at W1;
- seven threads × 0.58 ms = 4.06 ms of that difference is consistent with the wait (arith.).

**So solve_prep's 0.073 ms partition share is an artefact of the partition.** Jolt's serial wall at W8 is about
0.58 ms: the sort and split plus the islands, with FinalizeIslands and a few small single jobs; its broadphase
refit runs concurrently and is not on the serial path (rev 1 O2). This is my reading of code plus arithmetic, not
a measurement. The protocol (§7) adds the reading that confirms or refutes it (the `WaitingForBatch` counter).

This matters for the design. Jolt does not win by *overlapping* serial work. It wins by having **one** serial
block where we have eight.

### 4.3 Why 2.72 against 3.83 (arith.)

| | ours (J-T, reuse off) | Jolt v5.6.0 |
|---|---|---|
| T(1), [0,500), block B | 5.957 | 9.605 |
| S(1), serial | **1.197** (s = 0.199) | **≈ 0.575** (s ≈ 0.060): solve_prep 0.554 + islands and FinalizeIslands 0.028; BroadPhasePrepare runs concurrently and is excluded (rev 1 O2); these scopes have 2–56 samples each, so profiler inflation is small |
| P(1), parallel work | **4.83** | **≈ 9.03** |
| T(8), block B | 2.188 | 2.509 |
| E(8) on the parallel part | **0.62** | **≈ 0.60** |
| T1/T8 | 2.72 | 3.83 |

Counterfactuals, with our I(8) kept and L scaled with P (the design's own formula):
- **(a)** our P with Jolt's S(1): T1/T8 = **3.38**;
- **(b)** our S with Jolt's P(1): 3.33 at the design's P_J = 8.97 (rev 1 did not recompute it at 9.03);
- **(c)** both Jolt's S(1) and P(1): **3.94**, against Jolt's measured 3.83 — the model over-predicts by ≈ 3 %.

**Half of the gap is serial time and half is the size of the parallel work.** Our parallel machinery is not the
cause. And the parallel work is small *because we are fast per manifold at W1*.

**At W16** our T(16) (block B) is **+8.0 %** over T(8) (2.188 → 2.363 ms), while Jolt's is **−4.0 %**. W16 means
16 SMT threads on 8 cores. Our per-stage W16 spans were never taken (P4 ran W1 and W8 only). There are three
candidate causes, and §7 measures them:
1. the wake cascade over 16 lanes;
2. spinning or yielding SMT siblings slowing the serial caller's core;
3. serial stages that are 56 % of the step and cannot use the extra lanes.

---

## 5. Reference engines at the points where we serialize

| concern | ours today | Jolt 5.6 | Box2D v3 (`main`) | others |
|---|---|---|---|---|
| Dispatch granularity | one `pool.scope` per wide colour per pass: ~109 per step, plus 1 in the np | whole-step job graph; one semaphore release wakes n threads | **one task per worker per solve**; the caller joins as worker 0 and CAS-claims the orchestrator role (`solver.c`, `b2SolverStep`) | Rapier 0.35: "a staged multithreaded solver (parallelism within a single island), for lower per-step overhead and steadier timings" |
| Barrier between colours or stages | scope join; helpers park after the backoff | 64-bit split status word; waiting workers `yield()` inside the job | `atomicSyncBits` (stage and sync index); workers spin with "`if (spinCount > 5) b2Yield(); else b2Pause();`"; the orchestrator spins on `stage->completionCount` | Bullet `btBatchedConstraints`: phases of batches, synchronized between phases |
| Work claiming | static cohort-snapped chunks, ≤ 32 per colour, plus stealing | CAS batches of 16 constraints | per-block CAS on `syncIndex` from a staggered start (`GetWorkerStartIndex`); at most `4 * workerCount` blocks per stage | — |
| Warm start | serial, 4× per step | iteration 0 of each split, in parallel | a per-colour stage | — |
| Integrate | serial, 4× per step | parallel jobs by body batch | a per-body stage (integrate velocities, integrate positions) | — |
| Contact setup | serial `solve_build` | inside the narrowphase job | `b2_stagePrepareContacts`, a parallel stage | — |
| Broadphase pairs | serial tree query | fused into the FindCollisions jobs | threaded pair finding ("broad-phase threading", 3.0 release post) | — |
| Colouring / islands | serial, rebuilt every step | lock-free linking in the np, plus a serial finalize and split | **persistent, incremental** colouring on contact begin/end; static bodies use no colour; an overflow colour ("Don't expect the same result for overflow and graph colored") | PhysX: constraint partitions (32 initial, plus `processOverflowConstraints`); Rapier 0.35: "persistent contact graph" |
| Determinism across W | bit-identical, by construction (ordered emit, colour disjointness) | contact sort, serial, 0.55 ms | per-thread bit arrays merged by OR on the main thread ("used bit arrays to maintain order") | Rapier 0.35: "multithreaded trajectories may shift within solver tolerance" (we do not accept that) |

The shared shape across Jolt, Box2D v3 and Rapier 0.35: **persistent participation plus spin barriers inside the
solve, and no per-colour fork/join**. Box2D's release notes add that "v3 scales quite well with cores as long as
they share an L2/L3 cache". The 5900HS has a single 8-core CCX, which is that case.

---

## 6. Ranked levers for W8 and W16

**All seven levers are bit-identical.** Each keeps the pose hash at every W and the {1,N} oracle, and **moves no
pose or oracle pin**. They do move structural pins; the pin ledger (§6.10) lists them.

**Where the Δ figures come from.** Δ is the expected change at W8 (arith.), on one consistent baseline:
- J-T starts from the post-C4 baseline, **1.97 ms**, plus r ∈ [0, 0.16] ms, J-T's unmeasured narrow-colour rise
  (§2.3). The S1 narrow term is counted only where r is in the baseline.
- J-A starts from window 7's armed reuse-on W8 cell, **5.613 ms**. J-A's stage spans are bp 1.964 (AllPairs),
  np 0.159, setup 0.305, warm 0.529, wide colours 2.147, narrow colours 0.193, graph 0.116; its wide colours at
  W1 read 12.21 ms. Its narrow rise is measured and already inside that cell.

**E** is taken from the matching measured stage (E_wide(8) = 0.49 on J-T and 0.71 on J-A), so 1 − 1/(8E) = 0.745
on J-T and 0.824 on J-A.

**ω_b**, the stage-barrier cost, is bracketed at **0.3–1.0 µs**: it is unmeasured, and the §7 microbench measures
it.

| rank | lever | Δ W8 J-T ms | Δ W8 J-A ms | Δ W16 | value change | risk | depends on |
|---|---|---|---|---|---|---|---|
| 1 | **S1: solve region.** One scope per solve; stages separated by a spin barrier over a fixed stage table; blocks claimed by CAS on an execution count. Replaces the ~109 colour scopes, and lowers the narrow-colour threshold | 0.015–0.184 after the recruitment ramp (wide); narrow 0 if r = 0, 0–0.12 if r = 0.16 | 0.08–0.40 (wide), 0–0.14 (narrow) | at least W8's | no | **high** (new kernel primitive, loom) | none on F3/C4. C1b: the freeze capture, the freeze restore and `write_back` stay serial and run inline on the orchestrator between published stages. C3d/C3e: the stage table is built from `StepInputs` |
| 2 | **S2: warm apply per colour inside S1** (L7 revived) | 0.16–0.22 | 0.36–0.43 | — | no | low after S1 | S1; C3e (the latched warm flag) |
| 3 | **S4: solve_build P-a and P-c in parallel** (L11 C4 / D10, plus P-a by manifold range) | 0.10–0.15 | 0.11–0.17 | — | no | low–medium | none; C1b plan flags and C3e are read per manifold |
| 4 | **S5: parallel tree query** (tree C2 / D6, reopened on a W8 rule) | 0.13–0.17 | 0 (cfg-A pins AllPairs) | — | no | medium | **after F3** (its c_q sets the size). Reaches the default world only **after tree C4**. C1b: the hint is a per-row predicate; L10's epilogue and release stay serial |
| 5 | **S7: W16 region participants capped at the physical cores** | 0 | 0 | **−0.175** on J-T (to the W8 level) | no | low | S1, or a lanes cap on today's scope path |
| 6 | **S3: gravity, integrate + inertia and store per block inside S1** | 0.080–0.086 | 0.07–0.09 | — | no | low after S1 | S1; C1b's freeze |
| 7 | **S6: memo of graph and P-b** when the manifold key sequence and the dynamic set are unchanged | 0–0.15 (hit rate unknown; the sleeping-on rate lower) | 0–0.10 | — | no | medium | a hit-rate counter first; C1b (the held-store epoch in the key); C3d (the `StepInputs` epoch) |

**Totals (arith.; the levers are not guaranteed additive, S4 and S6 overlap on P-b):**
- **J-T at W8, r = 0 (baseline 1.970):** −0.485 to −0.960 ms, giving **1.01–1.49 ms = 226–332 ns per manifold =
  0.75–1.10× Jolt** (Jolt: 301 ns, block B; ours on 4,467.66 manifolds).
- **J-T at W8, r = 0.16 (baseline 2.130):** −0.485 to −1.080 ms, giving **1.05–1.65 ms = 235–368 ns per manifold
  = 0.78–1.22× Jolt**.
- **Envelope: 1.01–1.65 ms, 0.75–1.22× Jolt.**
  - Parity (1.345 ms) needs −0.625 ms at r = 0 or −0.785 at r = 0.16. That is 29 % and 50 % of the way up each
    bracket.
  - **Parity is inside the bracket, not guaranteed by it.**
- **J-A at W8:** −0.62 to −1.33 ms, from 5.613 to **4.28–4.99 ms**.
- **The S1 + S2 + S3 build decision survives.** Its J-T low end is 0.015 + 0.16 + 0.080 = **0.255 ms = 12.9 % of
  T(8)**, which is ≥ 5 %.

**J-A is a poor row for scaling.** 35 % of its W8 step is the AllPairs broadphase (1.964 ms), which the cfg-A row
pins.

**Context rows (existing lanes, not new levers):**
- **Tree C4, the default flip.** For the shipped default world, which runs AllPairs until C4, this is the largest W8
  lever there is: Δbp(8) = **1.698 ms** measured (window 6; lever rulings, "C3b SHIPS"). J-T already pins the tree.
- **F3.** c_q falls from 167.6 to ≤ 150 ns per row: −0.02 ms (arith.: 0.2088 × (1 − 150/167.6)).
- **L9 C4** (already in the trunk): W8 np −0.25 ms (arith.). J-D measured −0.227 ms (n/n/n), and its quiet pass
  alone −0.238 ms (Y/Y/Y).

### 6.1 S1: the solve region (the kernel primitive)

**What it is.** A **phased region** API in `boyko_threadpool`. It is a kernel feature under Principle 0, usable
by every multi-stage consumer (the soft-body coloured solve, the narrowphase plus fill), not a physics adapter.

**The frame** lives in `ColoredSoftStepSolver` (a Resource) on `ScratchColumn`s. It is written only while no
region is open.
- **The stage table** is one entry per (kind, colour): kind, colour, block range, and `n_blocks ≤ 4 ×
  participants`.
  - It is built once per step, after P-b and before the first publish.
  - It is never written again until the region's scope join has returned.
  - A stage that runs several times per step (warm ×4, biased ×4, relax ×8 per colour) keeps one entry and one set
    of blocks.
- **The claim words**, one `AtomicU32` per block. Each is set to 0 when the table is built.
- **The completion counters**, one `AtomicU32` per stage entry.
- **The publish word**, one `AtomicU32`: `(exec << 16) | stage`, where `exec` counts that entry's executions this
  step (1, 2, …). 0 means "nothing yet" and `u32::MAX` means END.

**Entering the region:**
- the solve opens **one** `pool.scope` and spawns k = min(W, cores) − 1 helper tasks;
- the calling worker is participant 0 and the orchestrator (Box2D's shape);
- the scope's join is what guarantees the frame's lifetime: a helper that starts after END runs nothing and
  returns.

**The orchestrator** (participant 0, the calling thread), for each stage in order:
1. If the stage has one block, or the step is on the inline path (the P2 gate at `colored.rs:4231`), it runs the
   stage itself and publishes nothing (Box2D's `blockCount == 1` rule). Serial steps that are not stages run the
   same way: `write_back`, the freeze capture and restore (§6.3), and P-a/P-b.
2. Otherwise it increments `exec` for that entry and does `publish.store((exec << 16) | stage, Release)`.
3. It claims and runs blocks through the same claim routine as the helpers.
4. It spins until `done[stage].load(Acquire) == n_blocks`, then does `done[stage].store(0, Relaxed)`.

**A helper:**
- It spins on `publish.load(Acquire)` until the value differs from the last one it saw: PAUSE ≤ 5, then
  `yield_now` (Box2D). It never parks inside the region. On END it returns.
- It reads the stage entry named by the publish value. That is a shared read of an entry nobody writes while the
  region is open.
- It claims block `b` of that entry by `claim[b].compare_exchange(exec − 1, exec, AcqRel, Relaxed)`, starting at
  its staggered start index (Box2D `GetWorkerStartIndex`) and sweeping the ring once.
- After each block it does `done[stage].fetch_add(1, Release)`.

**Why a late helper is harmless.**
- A helper still scanning entry `s` under `exec = i` after the orchestrator has moved on finds every block of `s`
  at `i`. They are all claimed, because `done == n_blocks`. Its CAS from `i − 1` therefore fails on each one.
- When `s` runs again at `i + 1`, its blocks go `i → i + 1`, and the stale expected value `i − 1` still fails.
  There is no ABA, because `exec` only grows within a step and ≤ 12 executions fit in 16 bits.
- The helper cannot pair a block with the wrong kind: the kind is read from the entry whose publish value it
  acquired, and every entry has its own blocks.
- Nobody writes the entry, so the reads are not a data race.
- The `done` reset is ordered before any later increment of that entry. The reset is sequenced before the next
  publish (Release), and a helper's increment follows its Acquire of that publish.

**Across steps.** Each step's region is one `pool.scope`. Its join returns only after every helper task has
returned. So a helper from step t never exists while step t+1 rebuilds the table and the claim words (M-R6).

**Correctness properties:**
- **Happens-before chain:** block writes → `done` (Release) → the orchestrator's Acquire → `publish` (Release) →
  the helper's Acquire → the next stage's reads.
- **Work-conserving:** a stage completes on block count, never on arrived participants, so the orchestrator alone
  can finish every stage. No deadlock is possible when helpers are busy elsewhere or never scheduled.
- **Zero cost when off:** at W = 1, with no pool, or with lanes < 2, the one existing branch per solve
  (`colored.rs:4231`) runs the stages inline with no atomics. That is today's inline path.

**Stages per step.** In order: `fill (S4 P-c)` → per substep [`gravity` → `warm c` (S2) → `biased c` →
`integrate + refresh_inertia` (S3) → `relax c` ×2] → `restitution c` → `store` (per cohort, S3) → `carry` (per
manifold range, S3). After that come the serial steps: the swap and stamp, the freeze restore, `write_back`, and
`end_step`.

**Colour thresholds:**
- a colour below the block floor is **one block**, run inline by the orchestrator with no publish; Gauss–Seidel
  order across colours is kept by the stage sequence;
- `MIN_PARALLEL_SLOTS_PER_COLOR` (256) and `MIN_SLOTS_PER_CHUNK` (64) are re-derived from ω_b, not from the scope
  cost; a colour is made multi-block only above the re-derived floor, where its parallel gain exceeds its ω_b, so
  the net narrow term is ≥ 0 by construction;
- this is the S1 "narrow" term that answers C4's ×6.5 narrow rise, if those colours are ≥ 64 slots (the histogram
  in §7 decides).

**Why it is bit-identical.**
- A colour's blocks write disjoint bodies (the colouring's invariant, the same predicate as the `*_movable` write
  guards).
- Per-row stages write their own rows.
- Every body's float-add order equals the serial walk's.
- No arithmetic depends on the block → thread mapping or on claim order.
- The existing {1,N} oracle and the W 1/2/4/8/16 pose pins are the gates.

**The rules, checked:**
- **State:** the frame (sync, done, claim words, stage table) lives in `ColoredSoftStepSolver` (a Resource, on
  VM-backed `ScratchColumn`s). No side store.
- **Heap:** about 109 scopes per step become 1. This also retires most of the ruled per-scope allocation exception
  (L5 open question 1, KC-05 / D-M2).
- **Lock-free:** atomics and spin only.
- **SAFETY, under SB and TB:**
  - While the region is open, the orchestrator holds no `build_view()` and no `&mut` to any frame column. Everyone
    reads the table through shared references derived from one raw base captured before the first publish.
  - A debug-only `region_open` flag on the frame asserts that no `build_view()` of a frame column is taken while the
    region is open.
  - Body writes keep the `CohortSolveView` / `row_ptr` raw projections (the P1/P2 structural fix). No `&`/`&mut` to
    a whole buffer exists in a helper. SAFETY comments state the disjointness under both SB and TB. Miri runs with
    `-Zmiri-tree-borrows` and with the default SB.
- **No nested scope inside a region block.** `pool.scope` and `install` debug-assert on a thread-local "inside a
  region block" flag. Without that rule, a join inside a block could steal and run another region's helper, which
  spins until that region's END. Two such regions would deadlock.
- **Inlining:** stages dispatch through a `match` on the stage kind (no `dyn`); the spin and exit paths are
  `#[cold]`.

**Loom models:**
- **M-R1:** two stages, two helpers; a stage-1 write is visible in stage 2.
- **M-R2:** a late helper after END touches nothing.
- **M-R3:** the orchestrator alone completes a region whose helpers never run.
- **M-R4:** each block is claimed exactly once.
- **M-R5, late helper across a change-over.** One orchestrator and one helper. The stages are A (2 blocks), then B
  (2 blocks), then A again at `exec = 2`. The helper is preempted between its last `done.fetch_add` on A and its
  next claim scan.
  - Asserted: every block of every execution runs exactly once; each executed block sees the kind of the entry
    whose publish value its runner acquired; and the table, a `loom::cell::UnsafeCell`, is never written
    concurrently with a read.
  - The mutation "one shared cursor reset per stage" (the rev-0 protocol) must be red.
- **M-R6, two consecutive regions.** The second region's table rebuild is not observed by any helper of the first.
  The mutation "rebuild before the join returns" must be red.

**CPU cost.** Helpers spin for the region's length, about 0.6–1.3 ms per step at W8. This is the Box2D/Jolt trade,
and on a laptop part it is a **values question** (open question 6, the owner's). For the region's length, k =
min(W, cores) − 1 workers are unavailable to any non-physics system scheduled alongside the solve; helpers do not
run foreign pool tasks while they spin (Box2D's workers do not either).

**Why Δ is only 0.015–0.184 ms on J-T (wide).**
- Today's per-wave overhead is 2.88 µs.
- The region's residual per wave is block claims (0.2–0.4 µs), ω_b (0.3–1.0 µs) and a block-grain tail (~0.7 µs):
  2.88 − (0.2..0.4 + 0.3..1.0 + 0.7) = **0.78–1.68 µs saved per wave**, × 109.32 = 0.085–0.184 ms.
- **The recruitment ramp.** Today's first solve wave pays the park→wake cost inside ω, after ≈ 0.43 ms of serial
  graph plus setup; the region's residual model drops it, so it is charged back once per step at 0–0.07 ms (the
  10–70 µs of §3's external source; **no wake latency has been measured on this machine**, KE16 names it only as a
  risk, `KE16-RESULTS.md:1217`). The region is work-conserving, so the ramp costs lost width rather than a stall,
  and 0.07 is an upper bound. W8S's ramp telemetry (instrument 2) measures today's first-wave ramp, which uses the
  same push-and-wake path as the region's recruitment.
- **Mitigation, to be priced by that reading:** when S4 is inside the region, open the region *before* P-a, under
  a predicate on the graph's widest colour that is a superset of the P2 gate. P-a and P-b then run inline while
  the helpers wake. This hides up to (1 − f) × 0.312 ≈ 0.08–0.16 ms of ramp (arith.), at the cost of the helpers
  spinning through P-a and P-b.
- The low end assumes imbalance, not dispatch, dominates L(8). §7 splits the two.

### 6.2 S2: the warm apply per colour on S1

**Why L7 can come back.** L7 was retired at ω(8) = 6.54 µs: 4 × 9.11 waves × 6.54 µs ≈ 0.24 ms, which is at least
the whole warm apply (lever rulings, "L7 RETIRED"). The same argument now prices waves at ω_b (0.3–1.0 µs) instead
of that ω (ruling 2).

**The arithmetic:**
- J-T: 0.308 ms (measured) × 0.745 − 36.44 × ω_b = **0.19–0.22 ms**; with a pessimistic 1 µs tail per wave on top,
  **0.16**.
- J-A: 0.529 × 0.824 less the same wave cost = **0.36–0.43**.

**Bit identity** is the plan's own L7 argument: slots are colour-ordered, and a dynamic body appears at most once
per colour, so each body's add sequence is the serial one (the same argument as L11 D7). The fused "warm apply
inside the first sweep" stays rejected (L11 D8: not bit-identical).

**Alternative S2′.** One per-body warm stage per substep over a colour-ordered body→slot CSR that P-b emits. That
is 4 barriers instead of about 36, the same add order, but a CSR plus random gathers. Choose between S2 and S2′ with
the ω_b microbench.

### 6.3 S3: the integrate group on S1

Gravity, integrate with `refresh_inertia`, and store are per-row or per-manifold independent. `write_back` is not
(a shared non-atomic touched mask), and it stays serial.

**Why `write_back` and the freeze are serial** (rev 1 W2). `write_back` / `write_back_awake` do
`scratch.touched.set(row)` (`colored.rs:3937`, `:3962`); `TouchedMask` is one non-atomic `BitSet256` per 256 rows in
a `ScratchColumn` (`resources.rs:4699-4717`). Both also take the whole snapshot through `snap_view.as_mut_slice()`
(`:3931-3932`). The freeze capture is an ordered `frozen.push((row, *b))` (`:4165-4177`). So the freeze capture,
the freeze restore and `write_back` stay serial and inline on the orchestrator between published stages; the
capture could later be split into count, prefix and emit if the W8S J-Son witness (`PHYS_SLEEP_FREEZE`) ever prices
it above one ω_b per step. Nothing measured suggests that.

**`store_and_swap` (`:3853-3928`) is stage-safe:** each lane writes `recs_w[cold.mi[l]]`, and every solved
manifold appears in exactly one lane of the layout; the frozen-carry loop writes `recs_w[mi]` by index;
`carry_hits` is an integer sum, exact under any partition; the swap and the cursor stamp are a serial tail. So
`store` is a per-cohort stage plus a per-manifold-range carry stage, with integer partial sums reduced in range
order.

**J-T:** (0.1216 − 0.0027 write_back) × 0.745 − 9 × ω_b = **0.080–0.086 ms** (the nine stages are gravity ×4,
integrate ×4 and store ×1). J-A stays 0.07–0.09. This also removes their interference growth (integrate 0.066 →
0.081 from W1 to W8).

**Gravity cannot fuse with warm c=0.** A body could receive a warm impulse before gravity, which changes its add
order. It stays its own stage.

**Alone S3 is about 4 % of T(8)**, under the 5 % build-if. It is built only as S1's consumer.

### 6.4 S4: the parallel setup

- **P-c (the fill)** is L11 D10's design: cohort ranges with point quotas, a pure function per cohort. It becomes
  S1's first stage, or +1 scope before S1 exists (ruling 7 permits the interim scope).
- **P-a** runs by manifold range. Each range starts its read cursor with a binary search for the **translated** key
  `ord(la, lb)` of its first translatable manifold (Lemma W holds per lookup, and the gallop only accelerates an
  exact search), and `restore_cursor` gets its own per-range start the same way. Hit counts are summed per range as
  integers. Non-strict streams (the cold sorted index) stay serial.
  - `backward_searches` becomes a per-range sum whose value differs from the serial walk's: `find` classifies
    "backward" relative to a cursor that starts at 0 (`warm_records.rs:437-453`). That is a declared **counter**
    change, not a value change. S-c's anti-vacuity (`colored_tests.rs:5272`) must be re-verified on the per-range
    code, with "no jumper in S-c" required red.
  - The write side's `strict` flag (`warm_records.rs:663`) is a stream-global AND, so per-range P-a combines each
    range's `strict` with a boundary compare at each range seam.
- **P-b** stays serial: an O(M) walk of the colour CSR.

**The build trigger has already fired:** solve_build(8) is 14 % of T(8) against D10's 5 % (window 7 FOLLOW-UP 3).

**Δ:** 0.274 ms (post-C4, arith. from J-A's on/off ratio 0.879) × fill share f (0.5–0.75, **unmeasured**; W8S's
`phys_sb_*` sub-zones measure it) × 0.745 = **0.10–0.15**. S4's pass bar is 0.6 × 0.10 = 0.060 ms at W8 on J-T.

*As built (§10.4): shape F — the solved manifolds' source search fused into the fill tasks, one scope.*

### 6.5 S5: the parallel tree query

This is the broadphase rev 2 D6 / C2 design: count per row, prefix sum, emit, in the shape of the Grid's
`build_parallel`. Its bit identity is argued in the design and not re-derived here.

**Why reopen it.** C2 was **ruled NOT BUILT on 2026-09-25** on its W1 letter (t_q(J, W=1) = 0.208 < 0.235 ms), while
D6 fired at 6.9–7.3 % of T(8). After C4, the query is **10.6 % of T(8)** (0.2088/1.97). Ruling 1 reopens C2 on a W8
letter (≥ 5 % of T(8) and above the SE bar), as S5, after F3's re-time.

**Δ:** 0.2088 × 0.745 less one wave ramp (≈ 0.01) = **0.14**; 0.17 at E = 0.6. Size it after F3's re-time.

**With L10:** the hinted query's withholding is a per-row predicate, so it is parallel-safe. L10's epilogue and the
A2.3 release stay serial between the bp and the np.

### 6.6 S6: the graph and P-b memo

**The idea.** The graph is a pure function of the manifold (a, b) stream and the dynamic/movable predicates. P-b is
a pure function of the graph and the per-manifold point counts. Identical inputs give identical outputs.

**Why P-a does not move** (rev 1 W3). P-a's tag pass calls `manifold_frozen(m, graph, s)`, which reads
`graph.island_of` (`colored.rs:1884`, `:2184-2191`); the tag pass assigns `*tag = ManifoldTag { .. }` and the source
pass then ORs in `SRC_RESTORE` (`:1885-1915`); `layout` skips `!tag.solved()` (`:1369-1372`). `physics_build_graph`
reads L10's `SleepSets` classification and `manifolds.held` (`systems.rs:1824-1878`), not `IslandSleep`. So the
graph's own inputs are covered by a held-store and classification epoch, and the IslandSleep frozen set enters only
at P-a's tags, so only at P-b. The hit test needs only the key stream, which can be read where it already is.

**The graph memo** (decided in `physics_build_graph`, before the build). All of these must hold:
1. Identity row remap: the gather rows are unchanged, which `warm_cursor.remap` already classifies.
2. M is equal, and the stream key `ord(a, b)` equals the solver's previous write-side key `keys_r[mi]` for every
   `mi`: one streaming compare, read-only on the solver Resource, ≈ 2–5 µs per step at 4,468 manifolds (arith.,
   0.5–1 ns per compare; unmeasured).
3. The dynamic/movable witness from the gather.
4. An unchanged L10 held-store and classification epoch, which covers `cls` and `manifolds.held`.
5. An unchanged `StepInputs` epoch (C3d).

**The P-b memo** (decided in `solve_build`, after `IslandSleep::begin_step` and P-a). All of these must hold:
1. The graph memo hit.
2. **No tag's `(count, FROZEN)` changed.** P-a's tag pass compares the new tag with the one already in the column
   before it overwrites it — one compare per manifold, folded into a single `changed` bit. This is exact: `layout`
   reads only the graph's colours, `tag.count` and `tag.solved()`; it covers the freeze and wake transitions and
   wake-on-merge; `SRC_RESTORE` does not reach the layout.
3. Equal widths, which follows from (2).

**What does not change.** The source pass, the tag pass and their write order stay exactly as they are. Bits are
unchanged by construction: a hit reuses outputs that are pure functions of inputs proven equal.

**Δ = hit rate × (0.117 + P-b)**, minus the compare (≈ 0.002–0.005 ms, arith.). The hit rate in J [100,500) with
reuse on is **unknown**; its build-if uses the lower of J-T's and J-Son's rates, because C1b will make sleeping-on
the default. A counter first; it is built only if the hit rate clears the build-if.

**The end state is Box2D/Rapier's persistent incremental colouring.** It needs persistent pair identity (U7
`PairCache`, refactor-last per the owner) and changes the colouring order, a value change. It is not proposed now.

### 6.7 S7: W16

**Cap region participants at the physical core count.** Read the topology once, cold, at pool start
(`GetLogicalProcessorInformationEx` on Windows, `/sys/devices/system/cpu/cpu*/topology/thread_siblings_list` on
Linux), and record it in the runner's SUMMARY. Capping the participant count does not pin cores. The W16 armed rows
sample each participant's core id once per region (armed only); affinity pinning is proposed only if that sample
shows SMT siblings co-scheduled.

**Expected at W16:** J-T back to its W8 time, **−0.175 ms** (block B, 2.363 → 2.188). J-A's W16 is not slower than
its W8 (5.578 against 5.613), so 0 there.

**Before S1** a lanes cap on the chunk count is a partial form. Whether the W16 loss is the cascade, SMT or
serial-stage pressure is decided by §7's W16 spans.

### 6.8 Deferred or rejected, with reasons

- **A fused bp+np wave (Jolt's FindCollisions shape).** Three things block it:
  1. `begin_frame_synced` sizes the hysteresis table by the step's LOGICAL pair count before any write;
  2. the carry join needs `pairs_prev` by cursor;
  3. L10's epilogue and release read the whole stream between the bp and the np, which is serial by design (L10
     E8). With C1b on, that is every step.
  Its gain over S5 is one ramp (≈ 0.01–0.05 ms) plus locality (unmeasured). Revisit after C1b settles.
- **A counted group wake at a fork.** KE16 §W eliminated the wake-gating arms `wg` and `wgc`: `max_in_flight` was
  5–6 of 16. The region needs one recruitment per solve, which the existing push-and-wake path gives.
- **Fewer ECS systems, to cut g(8) = 0.031.** Principle 0 keeps the physics logic as systems, and the prize is under
  the 5 % bar.
- **A parallel AllPairs arm.** It would be 1.46 ms on J-A (arith.), but it is a measurement-row artefact: the product
  path is the tree (C4) or Auto.
- **Value-changing parallel colouring** (Jones–Plassmann with fixed priorities, W-invariant) and **persistent
  colouring.** These are later, owner-approved-class value changes. They are not needed for parity.

### 6.9 Recommended order

1. **The W8S measurement window (§7)**, on an instrument-only commit (bit-identical). It sizes S1, S4 and S6, and it
   explains the narrow rise and W16.
2. **S4**: designed, trigger fired, lowest risk per ms (its lane may be cut and built before W8S; W8S sizes it; the
   claim is made only after the window).
3. **S5**, after F3's re-time, under ruling 1.
4. **S1 with S2 and S3 as its first consumers, as one lane** with its own research, review, loom models and the
   written ω_b microbench. S7 folds in.
5. **S6**, only if its counter clears the build-if.

Every lever is gated end to end on the P0/window protocol. Its realized gain at W8 on J-T must be ≥ 0.6 × the
predicted low end and claimed, on the wall if that bar is ≥ R_8, else on its armed stage span with that span's
canary seen. No row may be claimed slower at W 1/2/4/16, each reported with its demonstrated resolution or as
unresolved. Pose hashes must be equal across W and to the parent (`--expect-pose`).

**Interactions that later lanes must know about:**
- **S1 changes the scope census.** The S8b / G-L5-5 structural pins and L12 W4's "dispatch happened" assertion must
  be re-expressed as region and block counters (§6.10).
- **S2 and S4 follow L8's block layout** when L8 lands. The kernel math is untouched, so the order between S1 and
  L8/L12 is free.
- **C3d (`StepInputs`).** S1's stage table is built per step from `StepInputs` after P-b. S6's memo key includes the
  `StepInputs` epoch.
- **C3e (the latched warm flag).** S2 reads it once per step, when the table is built. The flag is never read inside
  a stage.
- **C1b (the sleeping-on default).** The freeze capture, the freeze restore and `write_back` stay serial and inline
  (§6.3). S6's P-b hit requires unchanged `(count, FROZEN)` tags (§6.6), and its build-if uses the sleeping-on hit
  rate. The census pins S1 moves include S1d and S8b's sleeping-on scenes, under the same counter rule (§6.10).

### 6.10 The pin ledger (rev 1 W4)

The per-frame structural assertion of the frame census, `f.scope > passes + np && (f.scope − 1 −
np).is_multiple_of(passes)` (`alloc_frame_census.rs:2149-2179`), is red on **every** frame both for S4 as its own
scope (12k + 1) and for S1 (scope − 1 − np = 1). The S1c release pin reads **122..=134 / 122..=134 / 269** since the
L9 C4 re-pin (`:439`, `:627-628`), and 134..=134 belongs to **S1e**, the reuse-off twin (`:467`, `:646`).

| lever | pins it moves | the rule that moves them |
|---|---|---|
| **S1** | S1c release 122..=134 / 122..=134 / 269 and S1e 134..=134 / 134..=134 / 269 go **down** to 3..=3 per frame (1 install + 1 np + 1 region), with dispatch MAX falling with them. The debug pins 98..=110 / 221 go down too. The structural assertion becomes `scope == 1 + Δnp + Δregion` with `Δregion ≤ 1`, read from a new `region_dispatches` counter. The fan-out mutation (`:442-452`) becomes "a scope opened inside a region block", which the §6.1 debug assert and the census must both red. S8b's structural form (`levers/00-RULINGS.md:309-314`) gains `− Δregion`. The attribution binary's row D (`chunk == scope` at W 2/4/8, and the scalar lane-growth assertion, `:395-404`) is re-expressed as blocks per stage `≤ 4 × participants`, with its lane-growth arm on the block count. G-L5-5 and L12 W4's "dispatch happened" become region and block counters. New pins S1 introduces are written as formulas (`4 × participants`, stages per step as a function of substeps and colours), never as constants, so S7 does not move them | The census's re-pin from a long-run adjudication attributed by a counter, as in the fourth re-pin form (`:662-667`: "moved by exactly the object the lane added"). A narrowing, with no upward headroom kept |
| **S4 as its own scope** (before S1) | +1 scope and +1 chunk per frame. S1c, S1e and S8b's structural forms go red on every frame unless they subtract a new `setup_dispatches` counter | The same counter rule as L5 C4's +1/+1 shift, under ruling 7, which extends L5 OQ1's per-step scope exception (`levers/00-RULINGS.md:23-27`) to one setup scope per step, which S1 then retires |
| **S4 as S1's first stage** | Nothing beyond S1's | — |
| **S5** | +1 scope per step on every Tree step at W ≥ 2, in the bp system. It **can never be a stage of the solve region**, because a region cannot span ECS system boundaries. To reach J-T at all, S5 must ship default-on with the L4/L5 lanes term: `parallel_broadphase` defaults to `false` (`resources.rs:621`). Census reach: S8b-tree at W=4 (its structural form already subtracts `Δbp_dispatches`; its Tree arm pins the exact count, `levers/00-RULINGS.md:311-313`). S1c and S1e run AllPairs and are untouched | Ruling 7 in every case; then S8b-tree's exact Tree count is re-pinned +1 by the counter rule, iff S5 increments `bp_dispatches` |
| **S2, S3** | None beyond S1's; they are stages. The G1 hash pins stay, being bit-identical | — |
| **S6** | Any test that counts graph builds or layouts per step reads fewer on hit steps. The S6 lane enumerates them with `findReferences` on `ConstraintGraph::build_with_held` and `layout`. Poses are unchanged | Each such pin's own rule, declared in the S6 lane |
| **S7** | None found. No census scene runs at W=16; S8b is W=4 and row D is W 2/4/8. S1's block-count pins are formulas in `participants` | — |

---

## 7. Measurement protocol: "W8S", one quiet window, same window as Jolt, about 50 minutes

**Binary.** Trunk plus an **instrument-only** commit (`dev` tier, bit-identical: every row's pose equals the
trunk's). The runner records the msvc host, the SHA and the logical core count in the SUMMARY; the window driver
records the physical and logical counts once per window (ruling on Q6, §10).

**Rows (ours).** `J-T` (`--cfg default --broadphase tree --sleeping off`), `J-T-off` (plus `--contact-reuse off`,
the bridge to window 7 P4) and `J-A` (`--cfg a`).
- Every row runs at W ∈ {1, 8, 16}, disarmed and armed.
- Armed rows read every span and counter over [100,500) and [0,100).
- `--sleeping off` is **pinned**, because the parity row must stay comparable with Jolt's `allow_sleep=0` after L10
  C1b flips the default.
- One witness row, `J-Son` armed at W8 over [0,100) (awake) and [100,500), prices C1b's serial prologue/epilogue at
  W8 and reads S6's sleeping-on hit rate. All L10 code is serial (L10 rev 2, E8), and its awake W8 cost is
  unmeasured.

**Rows (Jolt).**
- `H-jolt56`, timed at W1/8/16 (`-s=Pyramid -q=Discrete -f -t=W -i=500`), interleaved with ours in each pass.
- The profiled build at W1/8/16, frames 100–400, read two ways:
  1. **the job walls per frame**, which test §4.2's critical-path reading;
  2. **the lighter profile** (window 7 FOLLOW-UP 10: `JPH_PROFILE` removed from "Add Constraint From Cached
     Manifold" and the per-batch solve scope, patched in the build directory only), so that shares become times.
- **Jolt's `WaitingForBatch` wait**, measured directly: an accumulated per-thread time counter in the yield branch
  (`PhysicsSystem.cpp:1554`) emitted once per job — not a `JPH_PROFILE` scope per yield, which would inflate
  samples. Build directory only.

**Statistics.** K = 6 per cell, two blocks plus pooled. A claim needs both min–max and SE, in every block and pooled,
per the P1 rule as ruled.
- Load receipts ≤ 5 % before and after each process.
- **The during-process witness is gated:** a process whose `others_busy_pct` exceeds 2 % is re-run once, and dropped
  if the re-run is also hot (window 7 FOLLOW-UP 8).

**New instruments.** Each gets an anti-vacuity expectation: its count per step equals the structural count, else
the row is void.
1. **solve_build sub-zones P-a / P-b / P-c.** They size S4, and S6's P-b.
2. **Per-wave telemetry, armed only:**
   - wave wall (the wide colour span);
   - ramp, the time from the scope opening to the first helper task starting;
   - tail, from the caller's own last chunk to the join returning;
   - `max_in_flight` per wave (KE16's occupancy receipt) and the lanes that ran a task.
   This splits L(8) into ramp, imbalance and join, and it sizes S1. Ruling 3 admits an armed-only `Relaxed`
   atomic timestamp per task, never a zone inside a worker task.
   - **The route counter:** for every solve scope, the joiner route (worker W-d′ or external), per step, on J-T and
     J-A at W 8/16. Production's route is whatever this reads; the physics system may run on a worker or on the
     dispatcher while it helps the `install` join, so the answer may be a mix.
3. **A colour histogram per pass.** Colours and slots in the bins [1,32), [32,64), [64,128), [128,256), [256,∞), with
   reuse on and off. It explains the ×6.5 narrow rise and sizes S1's threshold term.
4. **An S6 hit-rate counter,** armed only: graph hits and P-b hits separately, read on J-T (`--sleeping off`) **and**
   on J-Son at W8 over [0,100) and [100,500), so the sleeping-on miss rate from freeze and wake transitions is
   measured.
5. **The same spans at W16**, plus the core counts.

**Resolution at W8 and W16** (rev 1 W7). The resolution ruling covers "the W=1 J-A gate only; G-TW's W ≥ 2 bars
(26–73 % in window 7) are not re-measured" (`levers/00-RULINGS.md:190`). Window 7's W8 tree cell read range 0.81 % in
quiet block B and 25.83 % in block A; its bar formula, 2·d2(K)·hypot, gives ≈ 2.3 % ≈ 0.045 ms for two block-B-quality
cells at K = 6 (arith.). The rev-1 pass bars (0.6 × low end, J-T, W8, arith.):

| gated item | bar ms | % of 1.97 |
|---|---|---|
| S1 + S2 + S3 bundle | 0.153 | 7.8 |
| S5 | 0.078 | 4.0 |
| S4 | 0.060 | 3.0 |
| S7 at W16 | 0.105 | 4.4 of 2.363 |

S4 is barely above the best-case 2.3 % bar, and only in a block as quiet as window 7's block B. So:
1. **J-T canary ladder at W8 and W16.** `--canary-frac` rungs at 0.5 / 1 / 1.5 / 2 × the smallest wall-gated effect
   at that W (W8: 0.060 ms, S4's bar; W16: 0.105 ms, S7's), run **disarmed** (ruling on Q8, §10). The injection is
   serial, on the chain, so it adds 1:1 to the wall. K = 6, in each block and pooled, claimed under the two-spread
   rule. **The demonstrated resolution R_W** is the smallest rung seen, with every larger rung also seen (the wave-2
   rule of `levers/00-RULINGS.md:190`).
2. **One rung at each of W 1/2/4 on J-T**, sized 0.060 ms, S4's own bar (ruling on Q8). "No row claimed slower" then
   has a stated resolution at every W it names. At a W where that rung is not seen, the reading is **"unresolved
   above X %"**, never "pass".
3. **Span gates for levers under R_W.** A lever whose 0.6 × low end is below R_W at W8 is gated on its armed stage
   span instead of the wall: S4 on `PHYS_SOLVE_BUILD`, whose predicted −0.10 ms is 32 % of the 0.312 ms span; S5 on
   the bp query sub-zone; S3 inside the bundle, on its stage spans. The span's resolution is demonstrated the same
   way, by an armed, dev-tier, instrument-only in-zone canary (`--canary-zone <zone> --canary-ns <n>`, a busy-wait on
   the orchestrating thread inside the named zone) at 0.5 / 1 × the lever's span bar. The wall reading is still
   reported for every lever, with its R_W beside it.
4. **Order.** No lever is gated on W8S until 1–3 have been read (ruling 8 adopts this as the W ≥ 2 resolution
   protocol).

**Microbenches, in the same window and separate from the timed passes** (each runs on every route the counter
shows, weighted by its observed share; the SUMMARY records each bench row's route):
- **ω(W, gap):** a zero-work `pool.scope` with 32 tasks at W = 8 and 16, after a serial gap of {0, 5, 20, 80} µs. It
  separates dispatch from the park→wake cost. ω₁ from L4 is the zero-gap, one-lane point. Routes: worker (the scope
  is opened by a pool task) and external (the scope is opened on the `install` frame's thread).
- **ω_b:** a bench-only copy of exactly the §6.1 protocol, at 2/4/8/16 participants: the stage table fixed before
  the first publish, the claim words with the execution-count CAS, the publish word, the per-stage `done` with its
  reset, one-block stages inline, and END. The bench prices this protocol and no cheaper one. It is the S1 build
  decision's input, and it chooses between S2 and S2′.

**Reduction:**
- The identity T(W) = S(1) + I(W) + P(1)/W + L(W) + g(W) + u(W) at W = 8 and 16, closed to ±3 % of the solve span as
  P0's closure rules require.
- ω(W) = L(W)/waves, and E(W).
- Per-stage W1/W8/W16 ratios.
- Jolt's critical path from its job walls.
- Everything per manifold on each side's own count (ours 4,467.66 with reuse on, Jolt 8,489).

**Decision rules:**
- **S1 + S2 + S3** are built as one lane if their summed low end is ≥ 5 % of T(8) on J-T and above the SE bar. The
  arithmetic says 0.255 ms, 12.9 %. The L6 fork letter is superseded (ruling 2).
- **S4** is already triggered.
- **S5** is built by the W8 letter (ruling 1).
- **S6** is built iff hit rate × (graph + P-b) ≥ 5 % of T(8).
- **S7** is built iff the W16 spans show T(16) > T(8) with serial or SMT terms, and not a W16-only parallel gain.

---

## 8. Open questions (as asked), and the review's answered

1. **Reopen tree C2 on a W8 letter?** It is 10.6 % of the post-C4 T(8). It was ruled NOT BUILT on its W1 letter
   (2026-09-25).
2. **Supersede the L6 fork letter** (ω₁·waves ≥ 0.5·L(8)). With today's numbers it reads 0.855 × 109 / 315 = 0.30,
   which selects "retune the constants"; but the fork priced the colour loss alone, before L11 made the warm apply a
   stage of its own. Proposal: decide the region primitive on the S1+S2+S3 bundle, and revive L7 as S2 on S1.
3. **The O1 rule** (no zone inside a worker task) against the per-wave imbalance reading: may the instrument add an
   armed-only `Relaxed` atomic timestamp per task?
4. **The headline metric.** Replace T1/T8 with T(8) and T(16) per manifold, plus the identity terms. T1/T8 falls when
   W1 gets faster (C4: 2.72 → ≈ 2.18, arith.).
5. **Ask the results-analyst to re-read window-7 §5** "Jolt's serial work overlaps its parallel work", against
   Jolt's job walls. §4.2 reads it as a partition artefact.
6. **(Owner, values.)** Helpers that spin, never park, for the solve region's length (about 0.6–1.3 ms per step at
   W8, on up to 7 cores) are the Box2D v3 / Jolt trade. On a laptop HS part this is a power choice.
7. Extend L5 OQ1's per-step scope exception to S4's interim setup scope (retired by S1) and to S5's bp scope. The
   alternative is S4 as S1's first stage (§6.10).
8. Adopt W8S's canary ladder and span gates (§7) as the W ≥ 2 resolution protocol, since window 7's W ≥ 2 bars
   (26–73 %) are recorded, not re-measured.

**The review's own open questions, answered (rev 1 §9):** spinning helpers do not run foreign pool tasks (stated
cost in §6.1, joining OQ6); a nested scope inside a region block is forbidden and debug-asserted (§6.1); Jolt's
`WaitingForBatch` is read by an accumulated per-thread counter, not a scope per yield (§7).

---

## 9. Rulings (orchestrator, 2026-09-26)

Recorded on the tree by the document step (PC-W8S-5 of the lane's cut); quoted here as ruled.

| ruling | closes |
|---|---|
| 1. Reopen tree C2 (parallel tree query) on a W8 letter: **YES**, as lever S5, after F3's re-time. Its W1 letter (NOT BUILT, 2026-09-25) stands for W1; the W8 decision is taken on W8S's numbers. | OQ1 |
| 2. Supersede the L6 fork letter: **YES**. The region primitive is decided on the S1+S2+S3 bundle, not on the colour loss alone; L7 is revived as S2 on S1. | OQ2 |
| 3. Armed-only `Relaxed` atomic timestamp per task in the W8S instrument: **YES** — armed-only (zero cost when disarmed, proven by a codegen/census check), never a zone inside a worker task. | OQ3 |
| 4. Headline metric: T(8) and T(16) per manifold plus the identity terms, replacing T1/T8. | OQ4 |
| 5. Window-7 §5 re-read by the results-analyst: queued for the W8S analysis. | OQ5 |
| 6. **(OWNER VALUE)** spinning helpers for the region's length (≈ 0.6–1.3 ms/step at W8 on up to 7 cores): asked of the owner; S1's design carries both a spin-only and a spin-then-park variant until answered. | OQ6 |
| 7. Extend L5 OQ1's per-step scope exception to S4's interim setup scope (retired by S1) and S5's bp scope: **YES**. | OQ7 |
| 8. Adopt W8S's canary ladder and span gates as the W ≥ 2 resolution protocol: **YES**. | OQ8 |

**The order of physics lanes** (same rulings): the W8S instrument commit → the W8S quiet window (Jolt in the same
window); S4 (cut and built before W8S, sized by it, claimed only after the window); tree F3 + `--bp-kernel` +
TREE_BRUTE_MAX_ROWS = 144 / AUTO_TREE_LO/HI = 144/152 → tree C4 → S5; S1 + S2 + S3 (+ S7) as one kernel lane; S6 only
if its counter clears the build-if; L10 C1b; L8; L12.

---

## 10. As built on `u/phys-w8s`

The lane's cut asked eleven questions of the orchestrator; the rulings, and what each became:

| Q | ruling | as built |
|---|---|---|
| Q1, S4's shape | **F**: the solved manifolds' source search fused into the fill tasks, one scope; A as the fallback; B refused | commit (4), §10.4 |
| Q2, ruling 3's letter | per task one `Relaxed` `fetch_add` (the slot) and three `Relaxed` stores (start, end, lane) into a wave-local stack record, armed only | `profiling::WaveStamps::task` |
| Q3, unparks per step | dropped from this lane; filed PC-W8S-7 for a threadpool lane | not built |
| Q4, the route counter's precision | `current_worker_id()` accepted, the blind spot stated in the SUMMARY | `phys_route_worker` / `_external`, runner `route_note` |
| Q5, two files outside the lock | `narrowphase/dispatch.rs` and `crates/boyko_physics/Cargo.toml` join the lock | the np wave's telemetry; the `omega_b_region` bench entry |
| Q6, physical cores | the runner records logical cores; the window driver records both once per window | runner `host.logical_cores` |
| Q7, ω(W, gap) | a second mode of the ω_b bench | `benches/omega_b_region.rs --mode omega` |
| Q8, the ladder rows | disarmed wall rungs; the W 1/2/4 single rung is 0.060 ms at every W | runner `--canary-frac F` now accepts any F > 0 |
| Q9, a Linux-only red of the count-feature step | a host-divergence finding for its own lane, never a reason to weaken the step | commit (3) |
| Q10, `--bp-kernel`'s parse | a STRING match with an error arm (`rowwalk`, `leaflist`, `leaflist-kd`); `leaflist-kd` "not in this build" until F3 merges; the kernel and `TreeDiag` printed with `{:?}` | commit (2) |
| Q11, S8b-tree's brute threshold | this lane owns `alloc_frame_census.rs` and adds `set_brute_max_rows(0)` to S8b-tree's Tree arm | commit (4) |

### 10.1 The instrument (commit 1)

**The armed flag is a runtime branch**, never a `cfg` the default build sets: the zone gate `zone_enabled!` (a
`const` tier test and one `Acquire` load of the arm mask), which folds away under a profile whose tier is below
`Deep`. Every new site is at stage, wave or step granularity. The colour wave reads its armed state from its own span
guard (`color_zone.is_some()`), the narrowphase wave from its dispatch span's guard, so a disarmed wave adds no load;
the disarmed spawn loops are the loops that were there.

- **Four spans**, `phys_sb_bodies` / `_pa` / `_pb` / `_pc`, split `phys_solve_build` (1 each per solving step).
- **Twenty-six counters** (the profiling module's table): per solving step the sums of the colour waves' ramp,
  tail, in-flight and lanes, the unstamped tasks, the colour scopes and their tasks, the two routes, the ten
  histogram bins and the two S6 hits; and the narrowphase wave's five. A reader's per-wave mean is the sum divided by
  the solve scopes (the two routes' total); the split of L(8) into ramp, imbalance and join is a sum to begin with.
- **The wave record** (`profiling::WaveStamps`, 3 KiB, stack-local to one armed wave): the stamped task is the
  disarmed task wrapped with one reference (104 B + 8 = 112 B, inside row D's cell budget); the record is reduced
  after the colour's span closes.
- **The spawn witness** (review W4): `profiling::wave_records_built` counts every record ever built. A stamped task
  borrows a record, so a disarmed wave that spawned the wrapper must have built one; `profiling_bit_identity` asserts
  the count unchanged across its disarmed run and risen across its armed run.
- **S6's counter without the design's epochs.** Neither a `StepInputs` epoch nor an L10 held-store epoch exists on
  this tree (`step_inputs.rs`, `sleep_sets.rs`), so the counter compares the inputs themselves, armed only, before P-a
  overwrites the tag column: the gather sequence and `rows_changed`, an FNV-1a of the stream's pair ordinals, an FNV-1a
  of every row's (dynamic, held) flags plus the sleeping arm and the held store's size, no held-store transition
  (capture or restore), and for P-b every tag's `(count, FROZEN)`. It is valid from the second armed step.
- **The in-zone canary** is a solver-held `(zone, ns)` (`ColoredSoftStepSolver::set_zone_canary`, `#[doc(hidden)]`)
  checked in the admitted arm of `phys_solve_build` and its four sub-zones.
- **The ω_b bench** (`benches/omega_b_region.rs`): §7's protocol on both routes; the worker route runs as a detached
  pool task, so the bench thread cannot steal it; each row asserts its route.
- **The sample budget, measured, and why the wave readings are sums.** §7 and the cut pushed the four readings
  once per wave. On the parity runner's armed rows at W8 (`worker_lane_max`, the most samples one worker lane held in
  a step) that read 826 on J-T and filled a `dev` region's 1024 on L10's rest pile (step 0, 192 waves; 248 of 600
  steps above 900), where the parent reads 236 and 247: the drops voided five of L10's armed runner checks
  (`armed_R-S_*_W8`). The design's "about 720 on J" was right for J and silent on the pile. As per-step sums
  the instrument adds a load that does not grow with the waves: the same rows read 266 on J-T and 277 on the pile, 1 to 55 above
  the parent's step for step, and the rest pile's armed rows are void-free again. What is lost is the per-wave
  spread, which no claim of §5 reads.

**What stays unproven by codegen, and why** (review W3): the receipt reads bodies that exist as their own symbols in
the release object (`solve_color_dispatch`, which inlines the AVX2 and scalar colour kernels; the colour and
narrowphase task closures; `WarmRecords::find`; the five `simd` kernels). `fill_cohort`, `build_columns`, the two warm
applies and the gravity dispatcher exist only inlined into `solve_colored_inner`, which the instrument edits by design
(the sub-zones and the S6 probe), so no symbol-level receipt can isolate them; they are dropped from the codegen claim.
Their source is unchanged by commit (1).

### 10.2 `--bp-kernel` (commit 2) and 10.3 the CI step (commit 3)

*Filled by those commits.*

### 10.4 S4 (commit 4)

*Filled by commit (4).*

---

## 11. Measured

*Filled by the W8S quiet window: the identity terms at W 8 and 16, the per-wave ramp / tail / in-flight / lanes by
route, ω(W) and E(W), the histograms, S6's hit rates on J-T and J-Son, R_8 and R_16 and the W 1/2/4 rungs, the span
canary's resolution, Jolt's critical path, ω_b and ω(W, gap), and S4's A/B at W 1/2/4/8/16.*

---

## 12. Relocation: the design's anchors (`d8262be1`) on the lane's base (`45acccfc`)

Physics code at `d8262be1` is byte-equal to the lane base's merge-base `75bea42e`, so the anchors moved only by
L10b's C0/C3d/C3e: about +24..+62 lines in `solver/colored.rs` after its line 1768, and +1..+53 in `systems.rs`. The
trunk sync at the lane's start (`efcda36d`) changed no physics file.

| symbol | at `d8262be1` | on `45acccfc` |
|---|---|---|
| `MIN_PARALLEL_SLOTS_PER_COLOR` / `CHUNKS_PER_WORKER` / `MIN_SLOTS_PER_CHUNK` | `colored.rs:288` / `:317` / `:371` | unmoved |
| `CohortColumns::layout` | `:1321` | unmoved |
| `build_bodies` | `:1768` | `:1792` |
| `build_columns` | `:1820` | `:1844` |
| the tag pass (`manifold_frozen` call) | `:1884` | `:1913` |
| the source pass (`plan_counts`) | `:1903` | `:1932` |
| P-b (`cols.layout`) | `:1924` | `:1953` |
| the fill loop | `:1950` | `:1979` |
| `fill_cohort` | `:2031` | `:2060` |
| `manifold_frozen` | `:2184` | `:2213` |
| `solve_all_colors` | `:3314` | `:3343` |
| `solve_color_parallel` | `:3458` | `:3487` |
| its `try_with_active_pool` / `pool.scope` | `:3523` / `:3646` | `:3552` / `:3675` |
| `store_and_swap` | `:3853` | `:3882` |
| `write_back` / `write_back_awake` | `:3928` / `:3950` | `:3961` / `:3983` |
| `solve_colored_inner` | `:4045` | `:4101` |
| `PHYS_SOLVE_BUILD` / `PHYS_SLOTS_WIDE` / `PHYS_SLEEP_FREEZE` sites | `:4098` / `:4131` / `:4165` | `:4160` / `:4193` / `:4227` |
| the P2 gate (`let parallel = !fast`) | `:4231` | `:4293` |
| gravity / warm / biased / integrate / relax / restitution / store / write_back zones | `:4243` / `:4257` / `:4263` / `:4281` / `:4299` / `:4316` / `:4327` / `:4376` | `:4305` / `:4319` / `:4325` / `:4343` / `:4361` / `:4378` / `:4389` / `:4438` |
| `systems.rs` gather / bp / `tree.step` / np / graph / solve / apply | `:229` / `:381` / `:591` / `:677` / `:1824` / `:1927` / `:2011` | `:230` / `:398` / `:626` / `:714` / `:1870` / `:1975` / `:2064` |
| `plugin.rs` `add_physics_colored_solve` / the tree's insertion | `:341` / `:625` | `:350` / `:637` |
| `dispatch.rs` `PHYS_NP_DISPATCH` / its `pool.scope` | `:590` / `:603` | unmoved |
| `warm_records.rs` `find` / `plan_sources_restored` / `restore_cursor` / `strict` | `:429` / `:619` / `:660` / `:663` | unmoved |
| `profiling.rs` `SPAN_ZONE_COUNT` / `COUNTER_ZONE_COUNT` / `counter!` | `:172` / `:175` / `:241` | unmoved |
| `broadphase_tree/mod.rs` `set_query_kernel` | `:584` | unmoved |
| the runner's `parse_args` / `validate` / `configure` / `build` / `check_step` / SUMMARY | `:574` / `:729` / `:934` / `:982` / `:1241` / `:2025` | unmoved |
| the census's gate rules / structural assertion / S1c / S1e / S8b-tree / S8b's exact scope | `:601` / `:2173` / `:2284` / `:2306` / `:3177` / `:2709` | unmoved |
| the attribution binary's `chunk == scope` | `:2376` | unmoved |
| G-L4-1 (`one_worker_parallel_solve_takes_the_inline_path`) | `default_world_worker_invariance.rs:439` | unmoved |
| G3 / {1,N} / random {1,N} / S-c's `backward_searches > 0` | `colored_tests.rs:1024` / `:1117` / `:1311` / `:5272` | unmoved |
| `ci.yml` `feature-legs` / its derived-leg check / `bench-compile` | `:330` / `:453` / `:486` | unmoved |

---

## Sources

**Repository (trunk `d8262be1`, read-only):**
- `docs/measurements/2026-09-25-physics-window7/analysis.md` §§1, 2.4, 4, 5, 6 (`:96-128`, `:164-204`) and
  `analyst/tables.txt` §§4–6;
- `docs/measurements/2026-09-19-physics-p0/ANALYSIS.md` §§2, 9;
- `docs/physics/perf-campaign/00-RULINGS.md`, `01-PLAN-REV1.md` §2–3, `levers/00-RULINGS.md` (L5 `:23-27`, broadphase,
  L9, L11, L7 retired, the window-7 rulings `:190`, `:201-207`, S8b `:309-314`);
- `levers/L11-solve-setup/02-DESIGN-REV1.md` D2, D4, D7, D8, D10;
- `levers/L10-sleeping/04-DESIGN-REV2.md` E8, C1b;
- `docs/threadpool/KE16-RESULTS.md` §0, §W, §App-2 (`:735-760`, `:1210-1225`) and `KE16-EVIDENCE.md` N28, N33;
- `crates/boyko_physics/src/systems.rs`, `plugin.rs`, `solver/colored.rs`, `solver/warm_records.rs`,
  `resources.rs` (`:586`, `:621-645`, `:4699-4717`), `benches/jolt_parity_pyramid.rs`;
- `crates/boyko_physics/tests/alloc_frame_census.rs` (`:395-404`, `:439`, `:442-452`, `:467`, `:601-700`,
  `:2045-2066`, `:2149-2179`);
- `crates/boyko_threadpool/src/worker.rs`, `scope.rs` (`:1-30`, `:541-560`, `:917-942`).

**Jolt Physics v5.6.0** (read locally at tag `v5.6.0`, `e77f1755`):
- https://github.com/jrouwe/JoltPhysics/blob/v5.6.0/Jolt/Physics/PhysicsSystem.cpp (job graph `:240-576`;
  FindCollisions `:893-1010`; split batches `:1386-1432`; sort and split `:1497-1523`; yield `:1554`)
- https://github.com/jrouwe/JoltPhysics/blob/v5.6.0/Jolt/Physics/LargeIslandSplitter.h (`:32-34`, `:170-171`)
- https://github.com/jrouwe/JoltPhysics/blob/v5.6.0/Jolt/Core/JobSystemThreadPool.cpp (`:224`, `:320`)
- https://github.com/jrouwe/JoltPhysics/blob/v5.6.0/Jolt/Core/Semaphore.cpp
- https://github.com/jrouwe/JoltPhysics/blob/v5.6.0/Docs/Architecture.md ("The Simulation Step in Detail",
  "Deterministic Simulation"); rendered at https://jrouwe.github.io/JoltPhysics/index.html
- The large-island splitter's cited source, Chen et al.:
  http://web.eecs.umich.edu/~msmelyan/papers/physsim_onmanycore_itj.pdf (cited from Jolt's header, not re-read)

**Box2D v3:**
- https://github.com/erincatto/box2d/blob/main/src/solver.c, read 2026-09-26 (stages, `atomicSyncBits`,
  `b2ExecuteStage`'s per-block `syncIndex` CAS from `previousSyncIndex` to `syncIndex`, `b2ExecuteMainStage`'s
  `blockCount == 1` run with no publish and its `completionCount` spin and reset, `b2SolverTask`'s `syncBits` decode,
  spin ≤ 5 then yield and the `UINT_MAX` exit, `b2SolverStep` building all stages and blocks before
  `enqueueTaskFcn`, `maxBlockCount = 4 * workerCount`, `GetWorkerStartIndex`, one task per worker)
- https://github.com/erincatto/box2d/blob/main/src/constraint_graph.c (persistent colouring, static bodies take no
  colour, overflow)
- https://box2d.org/posts/2024/08/releasing-box2d-3.0/
- https://box2d.org/posts/2024/08/determinism/
- https://box2d.org/posts/2024/02/solver2d/ (not re-read here)
- https://box2d.org/posts/2026/06/announcing-box3d/ ("Graph coloring for large islands"; no scaling numbers)

**Others:**
- Rapier: https://github.com/dimforge/rapier/blob/master/CHANGELOG.md (v0.35.0-beta.0: persistent contact graph,
  staged multithreaded solver)
- PhysX 5: https://github.com/NVIDIA-Omniverse/PhysX/blob/main/physx/source/lowleveldynamics/src/DyConstraintPartition.h
  (32 initial partitions, `processOverflowConstraints`);
  https://nvidia-omniverse.github.io/PhysX/physx/5.4.1/docs/Simulation.html
- Bullet: https://github.com/bulletphysics/bullet3/blob/master/src/BulletDynamics/ConstraintSolver/btBatchedConstraints.h
  (phases of batches)
- Havok: no public source on its stage scheduling was found. The Havok solver blog post
  (https://www.havok.com/blog/how-havoks-constraint-solver-works-pgs-baumgarte/) does not cover threading, so no claim
  here rests on Havok.
