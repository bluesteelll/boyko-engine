# KE16 — Pool occupancy: the design space

**Status:** synthesis, revision 2 after two refutation rounds (four refuters), 2026-09-02, branch
`feat/threadpool-ke16`, HEAD `b6c41237`, worktree `D:/wt/threadpool`. **This is a map, not a
decision.** It merges one code probe, five research lenses (theory and papers; production runtimes;
game-engine job systems; designs unlike ours and abandoned; wake protocols and idle registries) and
four refuters' findings into one axis grid, places every design found on it, names the cells nobody
occupies, and ends with a shortlist for the architect. The architect designs, the implementers build
the shortlist for measurement, the number decides. Nothing here is a verdict. What changed in each
revision is logged in §I.

The owner's acceptance criterion binds every line: **throughput, not occupancy.** *"If it needs
synchronisation heavier than the gain from maximum core loading, it should not be done."* A variant
that occupies more cores and finishes slower loses. Keeping defect B as it is is a legitimate
outcome and is carried as candidate **B0** in §G.

## 0. How to read this

| File | Holds |
|---|---|
| `KE16-DESIGN-SPACE.md` (this file) | the problem as measured (§A), the axis set (§B), our current cell on every axis (§C), the catalogue index (§D), empty cells (§F), the shortlist (§G), open questions (§H), the revision log (§I) |
| `KE16-VARIANTS-PLACEMENT.md` | group **P** P0–P16 — where worker-spawned work lands and who can reach it (defect A, mechanisms 2 and 3) |
| `KE16-VARIANTS-GRANULARITY-JOIN.md` | group **G** G1–G17 — transfer granularity and promotion; group **J** J1–J11 — what the joining thread does (defect B) |
| `KE16-VARIANTS-WAKE-LOCALITY-SCHED.md` | group **W** W0–W14 — wake protocol and idle registry (mechanism 3); group **L** L1–L10 — locality and victim selection; group **S** S0–S9 — scheduler level (mechanism 1); the cost table |
| `KE16-VARIANTS-ADDENDA.md` | **P17–P22, G18–G20, J12–J13, W15–W19, L11–L13** — round-1 additions and overturned "empty" verdicts |
| `KE16-VARIANTS-ADDENDA-2.md` | **P23–P25, G21–G24, J14, W20–W21, S10** — round-2 additions; the evidence behind axes **36** (timed-wait resolution), **37** (analytical model), **38** (contention class) |
| `KE16-VARIANTS-ADDENDA-R2.md` | round 2 re-read as data: the per-finding integration cross-walk (no new ids — every design already has one), the disputed claims, and the register rows **N59–N67** that §I promised and never wrote |
| `KE16-EVIDENCE.md` | the negative-results register (§N), the unverified-claims register (§U), the sources index |

Split across files on purpose: a monolith's line references rot on every insertion. Cross-references
are by **variant id** (`P1`, `G4`, `W2`, …) and **file**, never by line.

**Provenance legend — every number in these files carries one tag:**

| Tag | Meaning | Counts as verified? |
|---|---|---|
| `[L]` | read by the synthesiser at **this checkout** (`file:line` given), or in the installed toolchain's `std` source (path given) | yes |
| `[S]` | source code read verbatim by a lens or by this synthesis (`file::symbol` given) | yes |
| `[D]` | official design document, manual, RFC, or maintainer statement in an issue/PR | yes, as a documented claim |
| `[P]` | peer-reviewed paper, read **through a text-extraction proxy** — the number is a transcription, not a typeset value (see §U-0) | yes, with the §U-0 caveat |
| `[P*]` | paper abstract only, or the PDF could not be opened | **no** |
| `[B]` | blog post, README, vendor-run benchmark, talk slides | **no** — recorded, never relied on |
| `[I]` | inference by a lens, a refuter, or this synthesis, not a citation | **no** |
| `[R:S]` `[R:D]` `[R:P]` `[R:B]` | read by a **refuter** at the stated kind and **not re-opened by this synthesis** — listed in §U so it can be re-checked | at the stated kind, flagged |

A number without a tag is a defect in this document.

## A. The problem as measured

### A.1 The established facts (do not re-derive)

| Fact | Number | Provenance |
|---|---|---|
| `par_iter` inside a scheduled system body vs the same driver from the dispatcher thread | **1.01× vs 7.69×** (16 workers, 16384 rows, ~50 ns/row, release) | `docs/OPEN-QUESTIONS.md` KERNEL entry, three instruments, 2026-08-30 `[L]` |
| `par_for_each_chunk` in a system body: max in flight | **1** | same `[L]` |
| Inter-system parallelism, four conflict-free systems | 4 lanes, **25.1 %** top lane at W=4 and W=16 | same `[L]` |
| Dispatcher-path in-flight ceiling | "**4–5 of 16** simultaneously live" | same `[L]`; **contested** — see A.3 |
| Parallel physics sites on the defective path | **4** (`solver/colored.rs:2667`, `soft/colored.rs:1031`, `resources.rs:1688`, `:1760`) | code probe, re-read `[L]` |

### A.2 Verified at this checkout

Everything the brief asked to verify holds at `b6c41237`, read directly. Revision 1 added the
`complete_task`, pre-park-wake, Injector-pin and `pending` rows; **revision 2 adds the last four**.

| Claim | Where `[L]` | State |
|---|---|---|
| Same-pool worker spawn goes to `injector_local[wid]`, then `unpark_one_idle` unconditionally | `crates/boyko_threadpool/src/worker.rs:365-376` | confirmed |
| `injector_local[i]` is read only with the caller's own id — **within one pool** | `worker.rs:216-223` (`pop_local_injector`), `scope.rs:441-443,477-479` | confirmed — **defect A**; see the cross-pool row below |
| Sibling steal walks `inner.stealers` (worker deques) only | `worker.rs:233-257` | confirmed |
| `wake_rotor.fetch_add(1, Relaxed)` executes **before** `idle.load(Acquire)` | `worker.rs:324` precedes `:326` | confirmed — one multi-writer RMW per spawn even when nobody is parked |
| `Scope::spawn` boxes one closure per spawned task | `scope.rs:339` | confirmed |
| Joiner steals batches into `let scratch = Worker::new_fifo()` with **no** `.stealer()` registered, then `drain_scratch` runs the residue inline | `scope.rs:448`, `:479`, `:488`, `:496`, `:524-531` | confirmed — **defect B** |
| `try_steal_any` walks `inner.stealers` from index 0 with **no self-skip and no randomisation** | `scope.rs:534-541` vs `worker.rs:245-250` | confirmed — the joiner can steal its own registered deque; victim order diverges from the worker loop |
| Worker deques are `Worker::new_fifo()` — FIFO at **both** ends, no LIFO-owner/FIFO-thief split | `thread_pool.rs:595`, `worker.rs:62-63` | confirmed |
| Idle registry is one `CachePadded<AtomicU64>` + `wake_rotor`; `MAX_WORKERS = 64` | `thread_pool.rs:49,139-148` | confirmed — a registry exists; it selects a wake target, never a placement |
| `pin_workers` is a documented no-op stub | `thread_pool.rs:551-557` | confirmed — no affinity exists |
| False comment "stage 1.5" | `thread_pool.rs:127-130` and `worker.rs:355-358` | present verbatim |
| False comment "stage 2" for the global injector (it is stage 3) | `thread_pool.rs:123-124` | present |
| `lib.rs` module doc omits the own-deque source from its "4-source" list | `lib.rs:47-48` | present |
| `lanes = pool.num_threads() + 1` and its rationale | `solver/colored.rs:2628-2639`, `soft/colored.rs:990-995` | present — refuted on the production caller (A.3) |
| `if lanes < 2 { return false; }` dead branch | `resources.rs:1494-1496` | present — `num_threads() >= 1` by `thread_pool.rs:586` |
| `CHUNKS_PER_WORKER` | 4 (`resources.rs:519`), 6 (`soft/colored.rs:65`), 6 (`solver/colored.rs:255`) | read — answers the lens-1 open question on wave size (A.4) |
| ECS dispatch gate / park / drain / spawn | `schedule.rs:602`, `:683`, `:722`, `:1272` | confirmed |
| `par_iter` chunking | `par_iter.rs:73` (`MIN_ARCHETYPE_FOR_PARALLEL = 1024`), `:119-125` (`chunk_size = clamp(N/(W×bpt), 1024, ∞)`), `:331-341`, `:440` | confirmed (`crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs`) |
| Resolved `crossbeam-deque` | `Cargo.lock:1017-1018` → **0.8.7**; `MAX_BATCH = 32` at `deque.rs:18`, Injector limit `MAX_BATCH + 1` at `:1760` | corrects the probe (a lockfile exists at the worktree root) — the batch arithmetic is identical to 0.8.6 |
| `complete_task` unparks the joiner UNCONDITIONALLY before `pending.fetch_sub` | `scope.rs:157-160`, rationale `:139-151` | confirmed — two multi-writer RMWs + a conditional syscall per task completion (→W17) |
| The joiner calls `unpark_one_idle` before every `park_timeout(50 µs)` | `scope.rs:506-512` | confirmed — a third wake trigger, on the park path (→W17) |
| `Injector` never pins the epoch: `push` = two Acquire loads + SeqCst CAS on the tail index + `slot.state.fetch_or(WRITE, Release)`; an empty probe = two Acquire loads + `fence(SeqCst)` + a Relaxed load; only `Worker::resize` and `Stealer::steal*` pin | crossbeam-deque 0.8.7 `deque.rs:301,650,764,1006` (pins) vs `:1383-1440` (push), `:1795-1830` (probe) | confirmed (U44 closed) |
| `pending.fetch_add(1, AcqRel)` once per spawned task | `scope.rs:132` | confirmed — the per-task RMW that batch spawn (G20) amortises |
| **The joiner has NO pool-identity check**: `join_workers_until_drained` computes `on_worker` from `tls::current_worker_id()` alone; `tls::active_pool_ptr` is consulted only at `worker.rs:39,369` and `thread_pool.rs:253,375` | `scope.rs:441-443`; grep of `active_pool_ptr` over the crate (rev. 2) | confirmed — a pool-A worker joining a pool-B scope drains **pool B's `injector_local[own id]`** (→J14, App-6); `push_task` has the check the join lacks |
| **`park_timeout(50 µs)` and `(100 µs)` become 1 ms waits on the Windows target**: `sync/thread_parking/mod.rs:1-16` routes windows→`futex::Parker`; `futex.rs:68-85` → `futex_wait(…, Some(timeout))`; `sync/futex/windows.rs:66-67` → `WaitOnAddress(…, dur2timeout(d))`; `pal/windows/mod.rs:240-254` "Nanosecond precision is rounded up" to whole ms; no `timeBeginPeriod`/`NtSetTimerResolution` anywhere in the worktree | `C:\Users\flint\.rustup\toolchains\nightly-x86_64-pc-windows-gnu\…\library\std\src\sys\` (rev. 2); grep of the worktree | confirmed — every backstop and latency-floor number in revision 1 was a Linux number at best (axis 36, App-7, H.10) |
| **`complete_task`'s stated reason for the unconditional unpark is wrong**: "learning we are last would require reading `pending` after the sub — too late" | `scope.rs:149-151` | refuted — `fetch_sub` returns the previous value; the real constraint is the waker's lifetime (`:145-148`), which rayon and std solve by keeping the wake target outside the scope allocation (→W20, N66) |
| `active_scopes` is a counter, not a registry of live scopes | `thread_pool.rs:152` | confirmed (bears on E26) |

### A.3 What the engine path actually does — the route distinction that decides B

The probe established, and this synthesis re-read `[L]`, that there are **two routes** and the
engine takes only one of them per frame:

- **Route (a) — dispatcher spawns, dispatcher joins.** A non-worker thread inside `install` spawns a
  wave and then drops the scope with tasks still in flight. Tasks reach `injector_global`
  (`worker.rs:373`); the joiner helps via `join_workers_until_drained`. This is the shape of
  **every bench and every test** (`parallel_solve.rs`, `colored_solve.rs`, `soft_colored_sp4.rs`,
  `broadphase.rs`, `stress.rs`, `smoke.rs`) and of **no production caller found** (the probe grepped
  `boyko_ecs`, `boyko_render`, `boyko_app`, `boyko_ui`; the rest of the workspace is an open
  question, §H). Defect B lives here.
- **Route (b) — worker spawns, worker joins.** Every `par_iter` in every system body and all four
  physics sites. Tasks reach `injector_local[wid]`; the worker is inside the body, not in
  `worker_main`, so `worker.rs:221` is not running; the join drains the injector into `scratch`
  (`scope.rs:479`) and runs it serially. Ceiling **1**. Defect A, compounded by B.
- **The frame path itself.** `Schedule::run` opens ONE scope for the frame (`schedule.rs:412`) and
  drops it only after `executor_main_loop` returns, which happens only when every system completed
  (`schedule.rs:645-647`). By then `pending == 0`, so `Scope::drop` returns on its first
  `is_drained()` load (`scope.rs:463`) and never builds a batch. **Defect B does not apply to
  per-frame system dispatch.** While the frame runs the dispatcher is parked at `schedule.rs:683`
  (`park_timeout(100 µs)` — **≥1 ms on Windows**, axis 36) and steals nothing — it is not a lane.
  So `num_threads + 1` overcounts by one **even after** defect A is fixed.

Two data points about route (a) disagree and both are recorded:

| Datum | Value | Provenance |
|---|---|---|
| dispatcher-path ceiling | "4–5 of 16 simultaneously live" | OPEN-QUESTIONS 2026-08-30 `[L]` |
| `tests/diag_lane.rs` 4096-task, 4-worker dispatcher wave, ~24 runs | the joining dispatcher executed **zero** tasks | `tests/diag_lane.rs:74-80` `[L]` |
| the pool harness header | "the dispatcher route peaks at **16** in flight and still spends most of its wall-clock on **one lane**" | `tests/ke16_nested_scope_occupancy.rs:33-36` `[L]` |

The likely reconciliation is wave size and body cost (4096 trivial bodies vs 16–64 spinning
200 µs bodies), and the harness's own instrument note settles the *shape*: **defect B's signature is
per-lane work share, not max-in-flight** — a wave can peak at W and still spend most of its
wall-clock on the joiner's lane (`tests/ke16_nested_scope_occupancy.rs:87-91` `[L]`). Any future
statement about route (a) must quote `top_lane`, not `max_in_flight`. A third factor, new in
revision 2: on Windows the joiner that loses the race and parks sleeps **≥1 ms**, not 50 µs, so the
race's outcome is also a function of the timer resolution in effect (H.10).

**Bimodality (matrix U1), measured at this checkout by the harness author:** if the *outer* task is
spawned into a `Scope` from the test thread, which thread runs it is a race — on some runs the
joining thread drains it inline, the nested scope opens from the dispatcher, and the "nested" run
measures the healthy route twice (`outer_worker_id=WORKER_ID_DISPATCHER, max_in_flight=16` vs
`outer_worker_id=14, max_in_flight=1` on consecutive runs). The harness therefore dispatches the
outer task with `ThreadPool::spawn` and records `outer_worker_id` as a receipt
(`tests/ke16_nested_scope_occupancy.rs:244-256` `[L]`). This is why the two 2026-08-30 censuses
disagreed, and it is a standing hazard for every measurement in this campaign.

### A.4 A and B are coupled through the joiner's scratch

Today defect B is **subsumed** on route (b): the wave is already unreachable, so the serial drain
changes nothing. That will not survive any fix of A:

- Under **P1** (spawn to the worker's own registered deque), the joining worker's `try_steal_any`
  walks `inner.stealers` **including its own** (`scope.rs:534-541`, no self-skip `[L]`) — so the
  joiner steals half of *its own* wave into the unregistered `scratch` and runs it serially.
- Under **P2** (make `injector_local` stealable) or **P17** (push into an idle sibling's injector),
  the joiner still drains its own injector into `scratch` at `scope.rs:479` first.
- Under **P3** (route to `injector_global`), the joiner drains `injector_global` into `scratch` at
  `scope.rs:488`.

**Every A-fix promotes B from bench-only to the production path.** Therefore "keep B as is" (B0)
cannot be decided from today's numbers; it must be re-measured on route (b) *after* the A-candidate
is in place, and the B-candidates must be built to run in that configuration. The batch the joiner
takes is bounded by crossbeam's limit and the wave's block layout `[L]`: physics waves at W=16 are
`(W+1) × CHUNKS_PER_WORKER` = 102 (rigid), 102 (soft), 68 (broadphase) chunks — all above the 63-slot
Injector block, so a straddling grab can take the full **33**; `par_iter` waves are at most W chunks
(`batches_per_thread = 1`, `par_iter.rs:104` `[L]`), so at W=16 a grab takes **8**. The theory for
"a pile of independent chunks fills W workers by steal-half" is Tchiboukdjian et al.'s Theorem 3
(axis 37): `O(log2 W)` steal rounds, not `O(N)` — which is also the argument for batch spawn (G20).

### A.5 The second cause of lane idling lives in the ECS, not the pool

`running.set(i, false)` happens only in `apply_window_drain` (`schedule.rs:722` `[L]`) and on the
inline-exclusive path, and the drain fires only when
`pending > 0 && (pending == running.count_ones() || running == 0)` (`schedule.rs:602` `[L]`). With
four systems running and one finished, `pending = 1 ≠ 4`, the finished system's `running` bit stays
set, its `completed` bit stays clear, and its successors' `pred_remaining` stays `> 0`. A free
worker can only pick up a system that is conflict-free against the **whole still-marked-running
set** and has no unfinished predecessor. That is the "lanes drain at different times" the owner
named, and its cause is the **apply-window barrier**, not the pool. It is variant **S6** in the
catalogue and it must be measured separately from A (§G, mechanism 1). Revision 2 adds the
completion-side lever on the same event: when `pred_remaining[S′]` reaches zero, the completing
worker can run S′ itself instead of handing it back to the dispatcher (→S10, M1-c, axis 40).

Assignment of systems to workers is **dynamic** — the probe found no lane binding anywhere
(`SystemBox` carries `{system, kind, name}`; the ready scan indexes no worker; the task lands in
`injector_global` and the first worker to reach `worker.rs:68` takes it). So mechanism 1's
precondition is answered: it is dynamic, and an idle worker *already* takes another system whenever
the conflict bitset and the apply window allow. The lever is the gate, not a hand-out mechanism.

### A.6 The three mechanisms, placed

| # | Mechanism (owner's decomposition) | Where it lives in the catalogue | Status of the evidence |
|---|---|---|---|
| 1 | an idle worker takes **another system** | group **S** (`S0`, `S1`, `S2`, `S5`, `S6`, **`S10`**); locality cost in **L10**, **L11**, **L13** | assignment is already dynamic (A.5); the binding gate is the apply window; the cheapest hand-off is the completing worker running the released successor itself (S10, axis 40); cache-locality cost is **unmeasurable today** and the target's topology is **unrecorded** (axis 30) |
| 2 | an idle worker takes **intra-system work** from a busy peer | group **P** (`P1`–`P4`, `P18` are the fixes; `P0` is the defect) | every production runtime surveyed does this; the fix is one placement decision; the theory covering our help-first flat wave is ABP Theorem 9 (axis 37), not Blumofe–Leiserson |
| 3 | a **registry of idle workers** a spawner **pushes into** | **P6** (non-stealable placement — rejected in writing by Go and FJP, measured as a loss by XGOMP NA-RP), **P17** (stealable placement keyed on the idle mask — the buildable version), and **W6** (as wake selector — already present in boyko) | the registry exists; the non-stealable form is off the list; the stealable form is candidate **M3-s**; the completion-side wake (W17) has a shipped cheaper shape (W20) |

### A.7 Instruments at this checkout (all untracked)

The working tree holds **five** instruments plus three `[[bench]]` entries `[L]`:

| File | Lines | Measures |
|---|---|---|
| `crates/boyko_threadpool/tests/ke16_nested_scope_occupancy.rs` | 468 | both routes at the pool level; max-in-flight + wall-clock vs serial floor + **per-lane work accounting**; the RED-first gate `worker_spawned_wave_reaches_at_least_half_the_workers` (`#[ignore = "deferred: …"]`); the U1 anti-vacuity gate |
| `crates/boyko_threadpool/benches/ke16_nested_scope.rs` | 154 | criterion grid body ∈ {1 µs, 10 µs, 100 µs, 1 ms} × tasks ∈ {W, 4W, 64W}, both routes — **the grid the acceptance criterion is applied on** |
| `crates/boyko_ecs/tests/ke16_occupancy_gate.rs` | 201 | control (dispatcher) + gate (in-system) over one fixture, max-in-flight |
| `crates/boyko_ecs/benches/ke16_par_iter_in_system.rs` | 337 | the 1.01×/7.69× pair through a real `Schedule`, N ∈ {4096, 65536} |
| `crates/boyko_physics/benches/ke16_solve_in_system.rs` | 402 | the colored solve on both routes + an empty-schedule control |

No committed pool benchmark existed before these. Whether they are committed as part of this campaign
is a scope question (§H). **Every number they have produced so far was taken with ≥1 ms park
backstops** (axis 36); the grid's 1 µs and 10 µs cells are the ones this distorts most.

## B. The axis set, final

Axes 1–10 are the commissioned set. Axes 11–28 were added by the lenses and consolidated in the first
synthesis; 29–35 in refutation round 1; **36–42 in refutation round 2** (each names the refuter
finding it answers). The variant files place every design on the axes that discriminate it.

| # | Axis | Value set (who ships each value; `→id` = catalogue entry) |
|---|---|---|
| 1 | **Where a spawned task is placed** | own deque LIFO end (rayon, Cilk, FJP, libomp, Taskflow, .NET, Kotlin, Molecule, enkiTS, Eigen →P1) · own deque FIFO end (boyko deques) · own **bounded** local + LIFO slot, overflow half to global (Tokio, Go →P4, P10) · **per-worker injector polled only by owner, fed by the pool's ordinary spawn path** (boyko →P0; no precedent for *that*; thread-per-core does it by design with sender-routed SPSC inboxes →P19; rayon's **broadcast deque** is owner-only by design and fed only by `inject_broadcast` →P23) · per-worker injector in the scan set (→P2, unbuilt) · per-worker injector closed by a placeholder on the registered deque (rayon `JobFifo` →P18) · **another worker's REGISTERED queue chosen by the sender** (FJP `externalPush`, HPX hint, BEAM, CFS, PhysX →P17; by a rotating counter — Wicked →P24; rotating start + first `try_push` — stlab →P25) · mailbox of ANOTHER worker holding a proxy, task stays in origin deque (TBB, ABB, SLAW, NUMA-WS →P5) · global injector (async-executor/Bevy nested `spawn`, rayon fallback, TBB enqueue, Bitsquid →P3) · directly into a chosen idle worker, non-stealable (→P6, rejected) · every worker's inbox (id Tech 5 →P12) · random shared heap by trylock (Julia →P20) · one shared ring, per-consumer cursors (Jolt →P21) · no queue, generation broadcast + claim cursors (ForkUnion →P22) · **not queued: returned by the body as the next task** (TBB scheduler bypass →G24) · private deque + messages (Acar'13, HPX, Weave →P7) · split private/public deque (Dinan, Lace, LCWS →P8) · nothing queued until promotion (heartbeat →G10; lazy task creation →P15) · fixed partition (flecs, OpenMP static, HPX static →P13) · fixed partition + steal-from-remainder (libomp `static_steal` →G18) · central queue (Godot, Folly, GOMP →P14) |
| 2 | **Who initiates transfer** | receiver pulls (all classical) · sender pushes the WORK to a chosen thread, non-stealable (→P6, rejected by Go, FJP) · **sender pushes the work into a chosen thread's STEALABLE queue** (→P17, P24, P25) · sender pushes a HINT/proxy, work stays stealable (TBB →P5) · sender pushes only a WAKEUP (Go wakep, Tokio, FJP signalWork, boyko →W0–W2) · receiver posts a REQUEST, victim answers (Acar'13, HPX, Weave →P7) · deferred: request remembered, fulfilled later (lifelines →P9) · third-party balancer migrates out of band (BEAM, .NET →S5) · receiver-initiated pull balancing with a cache-hot threshold (Linux CFS →L13) · rate-limited sender (heartbeat →G10) · receiver steals the victim's unstarted remainder (→G18) · a running replica queues the next replica (.NET `TaskReplicator` →G22) · symmetric probabilistic exchange (Rudolph et al. `[P*]`, theory only) · none (static →P13) |
| 3 | **Transfer granularity** | one task (rayon, TBB, FJP →G1) · half the queue, capped (crossbeam 32/33, Go, Tokio, Unity, Dinan →G2) · half into an UNREGISTERED queue (boyko →G3) · half into a registered scratch (→G4, unbuilt) · 1 near / half far (HotSLAW HCS →G5) · adaptive one-vs-half (Weave, HPX →G6) · half the WORK by weight (→G7) · eager range split with grain (TBB, Molecule, boyko par_iter →G8) · split budget halved per split, RESET on migration (rayon thief-splitting →G21) · lazy split on demand (LBS, enkiTS, Weave, work-stealing tree →G9) · one promotion per beat, or a 16-job spawn batch (chili; forte →G10) · predicted-cost split (→G11) · atomic batch index, fixed (Godot, Unity, id Tech 5 →G12) · geometrically shrinking batch (libomp guided →G19) · a quarter of the victim's unstarted remainder (libomp `static_steal` →G18) · a BLOCK of the victim's deque (BWoS →G23) · whole continuation (→G13) · whole suspended deque (ProWS →J7) · whole core (Tokio block_in_place →J4) |
| 4 | **Victim selection** | uniform random (the proved model) · random start + full rotation (rayon, Go, Tokio, boyko worker loop →L2) · **fixed 0..n sweep** (boyko joiner →L2) · own id then all queues in order (Wicked →P24) · coprime stride (Eigen →L2) · sticky/last-victim/re-poll-same (Taskflow, FJP, FTL →L3) · nearest-in-hierarchy (HotSLAW, NUMA-WS, HPX radial, TBB arenas →L4) · two-choice sampling (Julia partr →P20; Mitzenmacher `[P*]`) · a random OTHER BLOCK, biased to longer queues (BWoS →G23) · steal-back from holders of my work (→L5) · lifeline hypercube (→P9) · thief-that-stole-from-me (leapfrogging →J2) · most-loaded via manager (ADM →P6) · named by affinity id (TBB →P5) |
| 5 | **What a blocked / joining thread does** | helps with ANY task (rayon, TBB, Godot, Bitsquid, boyko →J1) · helps within a restricted set (FJP theft chain, TBB isolate, **libomp tied-task descendants**, leapfrogging, own scope; Jolt's own barrier →J2, P21) · helps in its OWN pool while awaiting a FOREIGN pool's job (rayon `in_worker_cross` →J14) · steals half into a private serial scratch (boyko →G3) · blocks, no help (rayon cold, Godot non-pool, Folly →J3) · hands its core to a fresh thread (Tokio →J4) · triggers compensation/oversubscription, caller-declared (FJP, UE 5.5, TBB →J5) · replacement on scheduler-observed block (Linux cmwq, Windows IOCP →J12) · time-gated compensation (Chromium →J13) · swaps fiber (Naughty Dog, FTL; marl with stealing →J6) · suspends its deque / aborts and requeues (ProWS, X10 →J7) · no join exists — continuations (Molecule, Frostbite →J8) · retracts the awaited task (UE →J9) · wait-free counter (Nowa →J10) · runs the child, continuation stolen (Cilk →G13) · **parks and polls a completion queue, never helps** (boyko dispatcher on the frame path →J11) · spins/yields (id Tech 5 →P12) |
| 6 | **Idle policy** | spin forever (theory) · bounded backoff then park, with a full O(W) scan per round (crossbeam Backoff 127 pauses + 4 yields →W11; rayon 32 yields likewise) · **spin on ONE signal word, no queue access until acquisition** (.NET `WaitSlow`, ~35 µs calibrated →W21) · spinning/searching cap with wake-only-if-none (Go, Tokio →W2) · 2PC prepare/commit wait (Taskflow →W4) · idle bitmap + rotor wake-one (boyko →W6) · Treiber stack / versioned stack / LIFO semaphore (FJP, Kotlin, .NET, Folly →W6) · wake-all (rayon pre-RFC5 abandoned; flecs barrier →W7) · single spinner relay (Eigen, "whipping boy" →W8) · user-selectable per dispatcher (PhysX `eWAIT_FOR_WORK`/`eYIELD_*` →P17) · no registry: block on the OWN queue's condvar (stlab →P25) · advertise self and wait for a deal (Acar sender-initiated →P6) · register a deferred request and go quiescent (lifelines →P9) · parallelism feedback shrinks the worker count (A-STEAL, BWS, CLR →W12) · yield core to a peer (BWS →W12) |
| 7 | **Locality** | LIFO owner / FIFO thief (universal; **not** boyko →L1; FJP ships the FIFO-local mode as `asyncMode` for tasks "that are never joined") · one-slot LIFO cache (Tokio, Go →P10) · time-window before stealable (Kotlin, 100 µs →P11) · affinity mailbox (→P5) · last-victim (→L3) · places / cache-sharing groups (SLAW →L4) · socket-biased steal (NUMA-WS →L4) · L3/cluster-scope preference with a non-strict escape (Linux workqueue `cache_shard` →L13) · constructive sharing of one LLC by depth-first priority (PDF →L11) · footprint-to-cache anchoring (space-bounded →L6) · cache-sized split (Molecule 32 KiB →L7) · stable entity→thread partition across sync points (flecs →L8; + steal fallback →G18) · **the completing CPU runs the dependent successor** (Nanos6 →S10; TBB bypass →G24) · LIFO wake order for cache warmth vs rotating vs topology-nearest (→W6, W18) · chunk-size heuristics (→G8) · pinning / static (→P13) |
| 8 | **Level of the fix** | data-structure (deque protocol, fences →G15–G17; **per-block synchronisation** →G23) · pool (placement, granularity, wake →P, G, W) · runtime/compiler (two-clone, heartbeat, return barriers →G10, G13, P15) · scheduler (system partition, hand-out, apply window, migration, **inline successor** →S) · application (chunk size, splitting, lane count, batch spawn →G8, S9, G20, G21) · OS/hardware (BWS yield, ADM; Linux CFS placement and pull balancing →P17, L13; WAITPKG →W19; **OS timer resolution** →axis 36) |
| 9 | **Nested parallelism model** | fully strict fork-join (Cilk) · terminally strict async-finish (X10/HJ) · nested scopes on a stealable deque (rayon) · **nested scopes on an unreachable queue** (boyko →P0) · nested `spawn` to the pool-wide executor, `spawn_on_scope` pinned to the scope thread (Bevy) · futures (deviation bounds) · heartbeat promotion (→G10) · lazy task creation / lazy splitting (→P15, G9) · inlining below a threshold (par_iter < 1024 rows; spice join) · nested spawn FORBIDDEN (Unity →S4; ForkUnion →P22) · continuations, no nesting (→J8) · the body returns its successor (TBB bypass →G24) |
| 10 | **Synchronisation cost model** (how the criterion is applied — refined by axis 38) | owner push: plain store (ABP, Chase-Lev) · +1 store-load fence (THE, Chase-Lev take) · fence-free by TSO argument (→G16) · fence-free by at-least-once (→G15) · zero atomics on a private region (split/private deques →P7, P8) · owner and thieves synchronise per BLOCK (BWoS →G23) · one CAS per chunk (work-stealing tree →G9) · **spawn = alloc + `pending` RMW (multi-writer) + Injector CAS + slot `fetch_or` (single-writer under P0) + rotor RMW (multi-writer) + idle load** (boyko →W0; no epoch pin) · spawn = local push + one LOAD as the wake gate (Go, rayon, Tokio →W1–W3) · spawn deliberately omits the fence for spawned work (TBB →W5) · one RMW per WAVE instead of per task (batch spawn →G20) · steal = one CAS per task (→G1) or per ≤32 tasks (→G2), a `Stealer` probe pins the epoch, an `Injector` probe pays a full fence · idle = O(W) probes per round (boyko →W11; rayon likewise) vs one word (.NET →W21) · park path carries the lost-wakeup burden (→W4) · **complete = two multi-writer RMWs + a conditional syscall per task** (boyko →W17) vs **one RMW per task, one swap per scope** (rayon/std →W20) · zero atomics on fork, amortised over a ~100 µs beat (→G10, W10) |
| 11 | **Reachability closure** — is every queue in some thread's scan set? | closed by construction for `spawn`/`join` — only registered deques (rayon), **except** its owner-only broadcast deque, by design (→P23) · closed by putting submission queues in the SAME scan array (FJP even/odd) · closed by a placeholder on the registered deque (rayon `JobFifo` →P18) · closed by bounded local + overflow (Tokio, Go) · closed by a shared fallback queue with no stealing (PhysX →P17) · closed because every consumer sees every slot (Jolt →P21; Wicked →P24) · closed by leaving the task in the origin deque, mailing a proxy (TBB) · closed by design partition, nothing expected to cross (HPX pools, TBB arenas →P16; thread-per-core →P19) · deliberately unreachable for one mandatory task (TBB bypass →G24) · **broken by accident** (boyko `injector_local`; boyko `scratch`) · **cross-pool: see axis 42** |
| 12 | **Wake trigger condition** | every push (boyko →W0) · **every task completion** (boyko `complete_task` →W17) · **at the join, before parking** (boyko `scope.rs:511` →W17) · **the LAST completion of a scope only** (rayon `CountLatch`, std `thread::scope` →W20) · push into an EMPTY queue (FJP, rayon, TBB →W1) · no searcher exists (Tokio, Go →W2) · idle worker AND no searcher (Go wakep) · searching→not-searching transition (Tokio compensating wake) · at most once per interval, eager publication (Folly `ThrottledLifoSem` →W15) · the scheduler running a replica (.NET `TaskReplicator` →G22) · heartbeat (→W10) · barrier/sync point (flecs →W7) · never, for worker-generated work (cbloom rule →W9) · relay: wake one, it wakes the rest (→W8) |
| 13 | **Idle-registry structure, maintainer, and wake ORDER** | `AtomicU64` bitmap + rotor, rotating order (boyko) · Treiber stack packed in one word with version, LIFO (FJP `ctl`, Kotlin) · LIFO blocker stack under a lock (.NET) · LIFO semaphore (Folly; Windows IOCP releases LIFO →J12) · `Mutex<Vec<id>>` (Tokio) · per-worker `Mutex<bool>+Condvar` + global packed counters (rayon) · `pidle` list under `sched.lock` + `nmspinning` counter (Go) · `(id, Waker)` list + `AtomicBool` fast path (async-executor/Bevy) · `EventCount` 64-bit packed waiter stack (Eigen) · semaphore + waiting count (enkiTS) · idle-first then awaiting (Godot) · two-tier A/B team with a target size (Halide →W16) · **topology-nearest / recently-spinning** (Linux `select_idle_sibling`, marl →W18) · distributed per-victim (lifelines) · **none — the worker blocks on its own queue** (stlab →P25) · **none** (flecs, id Tech 5) |
| 14 | **Missed-wakeup semantics and backstop** | correctness-critical, closed (rayon external jobs; Go StoreLoad both sides) · throughput-only, tolerated (rayon internal; TBB "allows parallelism, never promises it") · closed by a timeout backstop (boyko `park_timeout(50 µs)` — **≥1 ms on the Windows target**, axis 36) · closed by a park-path re-check (boyko post-`mark_idle` re-poll; Tokio; FJP rescan) |
| 15 | **Deque exposure and extraction semantics** | fully concurrent (Chase-Lev, ABP, THE) · block-partitioned: owner's block private, other blocks stealable (BWoS →G23) · split public/private (Dinan, Lace, LCWS →P8) · fully private + messages (→P7) · the call stack is the deque (→P15) · shared progress counter (work-stealing tree →G9) · one ring, per-consumer cursors, slot claimed by exchange (Jolt →P21) · exactly-once (all) vs at-least-once (idempotent →G15) |
| 16 | **Task / continuation representation and spawn-path allocation** | **heap `Box` per spawn** (boyko `scope.rs:339`; rayon `spawn`) · job in the caller's frame (rayon `StackJob`, Cilk, chili) · pooled cell allocated once (Tokio, Go) · proxy in addition to task (TBB) · indirection job in addition to the task (rayon `JobFifo` →P18) · pooled fixed-size job, 64→128 B (Molecule) · fixed ring buffers, no runtime alloc (Our Machinery, enkiTS) · a bare pointer to a thunk, zero allocation (GHC sparks, axis 35) · two-clone / TLMM / stacklets / coroutine frames (Cilk-5, Cilk-M, libfork) · type-erased `JobRef` fn+data pointer (rayon) |
| 17 | **Promotion trigger and run-time adaptivity** | eager at every spawn · eager to a grain (→G8) · **my job was observed to have migrated** (rayon thief-splitting →G21) · local deque empty, with hysteresis (LBS, DF2-LS →G9) · a thief arrived / a request is pending (→G9, P8, G18) · a timer beat (→G10) · predicted cost > κ (→G11) · a shrinking schedule as the index drains (→G19) · adapted quantity: nothing / grain / policy work-first↔help-first (SLAW →G14) / steal size (HCS →G5) / victim distance (→L4) / worker count (→W12) |
| 18 | **Local-queue capacity and overflow policy** | unbounded growable (crossbeam, rayon, libomp, **boyko**) · fixed 256 + spill half to global (Tokio, Go →P4) · fixed 512 (async-executor) · fixed 32 per worker (forte →G10) · fixed blocks of 8 × 32 (stdexec BWoS →G23) · fixed ring + fail (Vyukov MPMC) · fixed 2..64K, remote end under a mutex (Eigen) · bounded + **drop on overflow** (GHC sparks, axis 35) · run inline on overflow (splitting, heartbeat) |
| 19 | **Helper's permitted set** | anything (rayon, TBB default, boyko — carries the documented TLS-corruption hazard →J1) · own theft chain (FJP) · own isolation region (TBB) · **task-tree descendants of the last suspended TIED task, untied exempt — binds thieves too** (libomp TSC, default on →J2) · own barrier (Jolt →P21) · priority floor (enkiTS) · dispatch-depth gated classes (TBB enqueue stream) · **own pool only, while awaiting a foreign pool** (rayon `in_worker_cross` →J14) · **anything, including a FOREIGN pool's owner-only slot** (boyko cross-pool joiner →J14) · nothing (rayon non-member →J3) |
| 20 | **Termination / quiescence and steal budget** | latch/counter + unpark-before-decrement + timeout backstop (boyko `ScopeShared`) · count-gated latch, last completer only (rayon, std →W20) · wait-free split counters (Nowa →J10) · distributed voting tree (Dinan) · lifeline quiescence (→P9) · barrier (flecs) · dummy pinned task to another worker (enkiTS `WaitforAll`) · steal budget: unbounded sweep then park (rayon, boyko) / bounded attempts then yield (Taskflow) / 4 passes with `runnext` late (Go) / one `try_pop` rotation then block on the own queue (stlab →P25) / ~1 ms of 256×32-nop loops (marl) |
| 21 | **Caller context: who may spawn, from where, with what determinism** | any thread (Molecule, enkiTS, Bevy, UE, rayon, boyko) · main thread only, nested forbidden (Unity →S4) · nobody at runtime — graph built before the frame (Frostbite, Destiny, flecs) · **boyko's routing key**: `WORKER_ID_DISPATCHER` / same-pool worker / other-pool worker / `WORKER_ID_UNATTACHED` (`tls.rs:24-28`, `worker.rs:369`) — the key `push_task` branches on, the key the physics lane count ignores (→S9), **and the key the joiner does not consult** (→J14) · determinism: frame-to-frame (Unity) / up to sync points (flecs) / none (Bevy, boyko) |
| 22 | **Thread-count elasticity** | fixed (rayon, Tokio, Taskflow, boyko; most game engines) · elastic during waits (UE 5.5 standby threads →J5; Chromium →J13) · elastic up to 4×+1 (stlab →P25) · compensation on block (FJP) · demand-driven allotment (TBB arena) · closed-loop feedback (CLR hill climbing/DFT; A-STEAL; BWS →W12) · kernel-managed with overcommit (libdispatch; IOCP concurrency value →J12) · idle-timeout retirement (.NET, Folly, Kotlin 60 s) · discontinued without a stated reason (Intel GTS, N65) |
| 23 | **Priority classes and fairness injection** | none (rayon, boyko) · priority tiers with promotion and a low-tier thread cap (Godot) · foreground/background local queues (UE) · three pools with OS thread priorities (Wicked →P24) · high/bound/normal/low + shared low (HPX) · enqueue FIFO stream checked only at the outermost level (TBB) · priority heaps (Julia →P20) · periodic forced global poll (Go 61 ticks, Tokio `global_queue_interval`, async-executor 64) · LIFO-slot poll cap (Tokio 3) · **boyko probes the shared injectors FIRST on every acquisition** (`worker.rs:195-211`) |
| 24 | **Mutation model constraining who may run what** | per-thread command buffers merged at sync points (flecs) · precomputed access-conflict bitsets (Bevy; boyko `conflict_bits`) · user-declared dependencies (specs) · read/write component tracking into a job graph (Unity) · data-dependency graph consulted at completion (Nanos6 →S10) · none (generic pools) |
| 25 | **What crosses between threads** | the task itself (Go, Tokio, rayon, crossbeam, boyko; into a stealable foreign queue →P17, P24, P25) · a proxy, task stays (TBB) · only a wakeup (Go, Tokio, FJP, boyko) · a request toward the work (HPX, lifelines) · a whole core/queue (Tokio block_in_place, ProWS, BEAM) · a whole BLOCK (BWoS →G23) · a bare thunk pointer that may fizzle (GHC) · nothing — the successor is returned or run inline (TBB bypass →G24, Nanos6 →S10) |
| 26 | **Blocking primitive** | `std::thread::park/unpark` — atomic swap, syscall **only if the target was PARKED** (boyko, rayon latches) · raw futex · Win32 `WaitOnAddress` (no kernel object, spurious wakes permitted; **timeouts in whole milliseconds** →axis 36) · `Mutex+Condvar` per worker (rayon sleep) · LIFO semaphore (.NET, Folly) · `LockSupport` (FJP) · `note`/futex (Go) · parking_lot bucket lock on every park AND unpark · **user-mode monitor/wait, TPAUSE/WFET** (WAITPKG; ForkUnion →W19, empty cell E22) |
| 27 | **Load shape the pool is tuned for** | continuous server load, queues rarely empty (Tokio, Go, FJP, .NET) · **frame-locked bursty waves that drain to empty between systems** (all game engines; boyko) — workers are parked at almost every wave boundary, so an ungated wake almost always pays the syscall, and the tail-latency floor is one chunk (unless the chunk shrinks →G19) · batch/HPC: one big job, barrier (OpenMP, flecs) |
| 28 | **Verifiability under the crate's own harness** | loom M1–M3 over the fork/join counter and the idle bitmap (`tests/loom_pool.rs`, transport abstracted away) and Miri over the `'scope→'static` transmute. A variant that changes only *which registered queue* a task lands in (P1, P2, P3, P17, P18, G4) is outside what loom models and inside what Miri covers; a variant that changes the wake protocol (W1, W2, W9, W15, W17, **W20**) changes exactly what M1/M2/M2b model and must extend them — **W20's count gate is precisely M1's subject**; a variant that adds a TLS pointer to the worker's deque (P1, P18) adds a Tree-Borrows obligation the Miri suite must exercise; the cross-pool joiner (J14/App-6) needs a join-side row in `tests/cross_pool_routing.rs` |
| 29 | **Completion / join-signal path**: who is notified per task COMPLETION, gated or not, at what cost | unconditional unpark of the joiner before the decrement — two multi-writer RMWs + a conditional syscall per task (boyko →W17) · **count-gated: one `fetch_sub` per task; only the LAST completer swaps the latch and wakes, and only if the owner was SLEEPING; the wake target outlives the scope** (rayon `CountLatch` + `CoreLatch` UNSET/SLEEPY/SLEEPING/SET; std `thread::scope` `ScopeData` in an `Arc` →W20) · no per-task signal — the continuation holder reaches the join (Cilk →G13) · unstolen job run inline at join, no signal (chili/spice →G10) · barrier counter per sync point (flecs) · own-barrier counter (Jolt →P21) · gated by a joiner-published parked flag INSIDE the scope allocation (→W17 design, the fallback) |
| 30 | **Intra-socket topology and core heterogeneity**: SMT pairs, L2 clusters, per-CCD L3s, P/E core classes | unrecorded for the target (**the standing gap**) · sub-LLC clusters exist on single-socket parts (Linux `CONFIG_SCHED_CLUSTER` `[R:S]`) · measured on a 12-core / 4-L3 Ryzen (Linux workqueue affinity scopes →L13) · partition skewed 60/40 by core class (libomp →G18, L12) · the L4/G5 verdicts ("collapses on a single socket") were conditioned on this axis being flat |
| 31 | **Spawn batching granularity**: shared RMWs per WAVE vs per TASK | per task (boyko `pending.fetch_add` + a queue op per chunk; rayon) · per wave (Tokio `push_batch`, Go `runqputbatch` →G20) · per wave by construction (one descriptor: Godot, Unity, Jolt, ForkUnion →G12, P21, P22; one replica at a time →G22) |
| 32 | **Block-detection source** | none (boyko, rayon) · caller-declared (FJP `ManagedBlocker`, Chromium `ScopedBlockingCall`, Tokio `block_in_place` →J4, J5) · time-inferred (Chromium MAY_BLOCK threshold + poll →J13) · scheduler-observed, delivered to the kernel's own pool (Linux cmwq `wq_worker_sleeping`, Windows IOCP runnable count →J12) · scheduler-observed, delivered to a USER-SPACE scheduler — **shipped and withdrawn / never merged** (Windows UMS, NetBSD SA, Linux UMCG — N59–N61) |
| 33 | **Wake fan-out COUNT per event** | one (Go, Tokio, boyko, Julia) · `min(new_jobs − idle, sleeping)` (rayon `spawn`) · **all, for a broadcast** (rayon `inject_broadcast` →P23) · `min(jobs, threads)` (Jolt) · a tier up to a target (Halide A-team) · cascade: each activated worker activates one more (FJP, O(log W)) · **task cascade: as many replicas as actually start, +1 outstanding** (.NET `TaskReplicator` →G22) · **conditioned on push kind: one for a task, all for a wave** (Wicked →P24) · all (flecs; Unity before its 1.15× fix) — →W16 |
| 34 | **Sender-side placement KEY when the sender chooses** — the counterpart of axis 4 | spawner identity (P1) · idleness registry (P6 non-stealable; P17 stealable) · data ownership (Seastar shard →P19; flecs entity partition →L8) · first-accepting bounded queue in index order (PhysX →P17) · **rotating counter** (Wicked →P24) · **rotating start + first lock-acquirable queue** (stlab →P25) · explicit core id per job list (id Tech 5 →P12) · random shared heap by trylock (Julia →P20) · affinity id from a previous run (TBB →P5) · **the CPU that completed the predecessor** (Nanos6 →S10; TBB bypass →G24) · last scheduler it ran on (BEAM →P17) · idle CPU in the waker's LLC domain (CFS →P17, W18) |
| 35 | **Task obligation** | mandatory exactly-once (every runtime in the catalogue; boyko chunks) · mandatory and deliberately unreachable by thieves (TBB bypass →G24) · advisory — may be dropped on overflow, fizzled if the owner did the work first, pruned by GC (GHC sparks `[S]`) · inline-executed by the owner on overflow (splitting policies) |
| 36 | **Timed-wait RESOLUTION of the target OS** *(rev. 2 — refuter 3)*: what a `park_timeout(d)` actually waits | Windows: `d` rounded UP to whole milliseconds by `dur2timeout` (`std`), then bounded by the system timer resolution, which Windows 10 2004+ does not raise for a process that never called `timeBeginPeriod` — **≥1 ms** for 50 µs and for 100 µs `[L]`/`[D]` · Linux: `futex(2)` timeouts carry the default 50,000 ns timer slack; real-time policies exempt `[D]` · bought resolution (`timeBeginPeriod(1)`: documented power and scheduler cost) · spin-until-deadline (no wait) · **boyko's cell**: `50 µs` at `scope.rs:512`, `100 µs` at `schedule.rs:683` are source constants, not waits; nothing in the tree raises the resolution; the actual expiry latency on the bench machine is unmeasured (H.10) |
| 37 | **Analytical model whose hypotheses the workload satisfies** *(rev. 2 — refuter 3)* | Blumofe–Leiserson (fully strict, work-first — NOT our shape) · **Arora–Blumofe–Plaxton Theorem 9: arbitrary DAG, "either choice" of push/execute, `O(W/p + D)`** — covers the help-first flat wave (→P1/A1) · **Tchiboukdjian–Gast–Trystram: W unit independent tasks, steal-half, random victim — `E[Cmax] ≤ W/m + 3.24·(log2 W + 1/(2 ln 2)) + 1`; DAG `W/m + 5.5·D + 1` vs ABP's `32·D`** (→G2, G20, G12, G18, B0–B2) · **Gast et al. with steal latency λ: `W/p + 16.12·λ·log2(W/2λ) + 3λ`; `E[R] ≤ 2pγ·log2(W/λ)`** (→P17/A5, W-f) · **Karlin et al.: spin for the block cost C is 2-competitive and optimal deterministic; random spin length on [0, C] is e/(e−1)** (→W11/W-c/W21) · Kruskal–Weiss FSC, GSS, factoring, trapezoid, Hagerup's "Bold" (→G8/G19; `[R:P]`) · **boyko's cell before rev. 2: none applied** — every shortlist prognosis was `[I]` |
| 38 | **Contention class of each RMW** *(rev. 2 — refuter 3)*: single-writer line (stays Modified in the owner's L1, ~10 ns class) vs multi-writer line (cache-line transfer, 40–400 ns class) vs a write to a FOREIGN line | boyko SPAWN: `pending.fetch_add` **multi**; `Injector::push` CAS + slot `fetch_or` on `injector_local[wid]` **single-writer under P0 route (b)**; `wake_rotor.fetch_add` **multi** · boyko COMPLETE: parker swap **multi** (every completer writes the joiner's line); `pending.fetch_sub` **multi** · P3 moves the two single-writer ops to a multi-writer line; P17/P24/P25 write a FOREIGN line; P1's Chase-Lev push has no RMW at all · classification `[I]` over verified single-writer facts `[L]`; anchors `[B]` (U50) |
| 39 | **Placement-policy SELECTION POINT** *(rev. 2 — refuter 4)* | fixed per pool (boyko `push_task` keys on caller identity only; Go; Tokio) · chosen per CALL by the spawner (rayon `spawn` vs `spawn_fifo` vs `broadcast` →P18, P23; FJP `externalSubmit` "added to a scheduling queue for submissions to the pool even when called from a thread in the pool" `[D]`; TBB `spawn` vs `enqueue`) · per task CLASS (Wicked High/Low/Streaming pools →P24) · **boyko's cell**: one discipline for a joined `par_iter` wave and for a fire-and-forget `ThreadPool::spawn` alike |
| 40 | **Successor hand-off on COMPLETION** *(rev. 2 — refuter 4)* — the counterpart of axis 29: what the completing worker does with the successor it just enabled | enqueue it as a new task (Bevy S1; Molecule/UE continuations →J8) · **the dispatcher enqueues it after its next poll** (boyko: `pred_remaining` hits zero, the dispatcher spawns S′ to `injector_global` after a `park_timeout` of 100 µs → ≥1 ms on Windows →S0, J11) · run it inline on the completing thread with probability p (Nanos6 `immediate_successor`, default 0.75 →S10) · returned by the body as the next task, no queue op (TBB bypass →G24) |
| 41 | **Deque synchronisation GRANULARITY** *(rev. 2 — refuter 4)* | per task — every owner take and every steal touches the shared indices (Chase-Lev, ABP, THE, crossbeam; boyko) · per BLOCK — owner and thieves synchronise only at block boundaries, thieves sample random non-owner blocks (BWoS →G23; stdexec) |
| 42 | **Cross-POOL reachability and helper set** *(rev. 2 — refuter 4)*: which pool's queues a thread may drain while joining a FOREIGN pool's scope | its own pool only (rayon `in_worker_cross` →J14) · **the target pool's `injector_local[own id]` — a slot it does not own** (boyko `scope.rs:441-443`, no `active_pool_ptr == inner` check →J14, App-6) · n/a — single-pool designs |

## C. Our cell on every axis

| Axis | boyko today `[L]` | Nearest shipped neighbour | What separates us |
|---|---|---|---|
| 1 placement | worker spawn → `injector_local[wid]`; dispatcher/foreign spawn → `injector_global` | rayon `inject_or_push` (identity check, then push to own deque); rayon `JobFifo` (same queue type, reachable via a placeholder); rayon's broadcast deque (owner-only, but fed only by `broadcast`) | rayon's `spawn` targets have a reader; ours has none but the owner |
| 2 initiator | receiver pulls; sender pushes a wakeup | Go, Tokio, FJP | none — same discipline |
| 3 granularity | `steal_batch_and_pop`: ≤32 from a deque, ≤33 from an Injector, into the thief's deque (worker loop) or into `scratch` (joiner) | crossbeam users (Go, Tokio semantics) | the joiner's destination is unregistered |
| 4 victim | worker loop: random start + rotation, self skipped; joiner: fixed 0..n, self included | rayon (random, full sweep) | the joiner's order is biased to worker 0 and reaches its own deque |
| 5 blocked thread | joiner helps with anything, batch-drains serially; **dispatcher on the frame path parks and polls, never helps** | rayon `wait_until_cold` (helps, one job at a time) | serial drain of an unregistered batch; the frame path is a third shape no runtime has |
| 6 idle policy | Backoff (127 pauses + 4 yields), each round a full O(W) scan; `mark_idle` (RMW) → re-poll → `park` | rayon (32 yields, each round a full `find_work` scan); .NET (spin on one word, W21) | same per-round scan shape as rayon; the decoupled form is shipped (.NET) and unbuilt here |
| 7 locality | FIFO both ends; no LIFO slot; no affinity; rotating wake order | — | none of the field's locality levers; the cheapest (LIFO owner end) is a constructor argument; FJP ships FIFO-local only for tasks "that are never joined" — ours are joined |
| 8 level | pool | — | scheduler-level (S6, S10), application-level (S9, G20, G21), OS-level (axis 36) levers unexamined |
| 9 nested model | help-first child stealing on an unreachable queue; `par_iter` inlines below 1024 rows | rayon (help-first on a reachable deque); Bevy (help-first on the pool-wide executor) | reachability |
| 10 sync cost, spawn | `Box` + `pending.fetch_add(AcqRel)` + `Injector::push` (SeqCst CAS + slot `fetch_or`, **no epoch pin**) + `wake_rotor.fetch_add` + `idle.load` — **1 alloc, 4 lock-prefixed RMWs, unconditional: 2 on single-writer lines, 2 on multi-writer lines** (axis 38) | Go: local store + one load; rayon: Chase-Lev push + one load, JEC CAS only if a sleeper exists | the only surveyed spawn path with an allocation and with an unconditional multi-writer RMW that is not the join counter (`wake_rotor`); the P0→P1 delta on the spawn path is two *uncontended* ops plus the wake, not two transfers |
| 10 sync cost, steal | one CAS per ≤32 tasks; a `Stealer` probe pins the epoch, an `Injector` probe pays a `SeqCst` fence | crossbeam users | same |
| 10 sync cost, idle | ~11 rounds × (2 Injector probes, each a full fence + W−1 Stealer probes, each an epoch pin) | rayon (32 rounds, full scan each); .NET (one word); Go (4 passes, then park) | our O(W) scan runs inside the spin budget; rayon's does too; .NET's does not |
| 10 sync cost, complete | `waker.unpark()` swap on the joiner's parker line + futex if the joiner is parked + `pending.fetch_sub` — **2 multi-writer RMWs + a conditional syscall per task, unconditional by design** | rayon `CountLatch`: **1 RMW per task, one swap + conditional wake per SCOPE** (W20); Cilk (no per-task signal) | the only row paying an unconditional per-task RMW pair on completion; the stated reason for it is refuted (A.2, N66) |
| 11 reachability | **broken twice**: `injector_local[i]` and the joiner's `scratch`; **and cross-pool in the other direction** (axis 42) | HPX pools / thread-per-core (broken by *design*, nothing expected to cross); rayon's broadcast deque (owner-only by design, dedicated feeder) | ours is fed by the ordinary spawn path |
| 12 wake trigger | **three**: every push; every completion; at the join before parking | Go rejected approach #3 (the first); rayon/std fire the completion wake once per scope (W20) | — |
| 13 registry | `AtomicU64` bitmap + rotor; rotating order; 64-worker cap | FJP `ctl` (Treiber, LIFO); Linux/marl (nearest / recently spinning) | we have O(1) "anyone idle?"; we lack recency and topology |
| 14 missed wakeup | timeout backstop (50 µs constant → **≥1 ms on Windows**) + post-`mark_idle` re-poll | boyko is at the "closed by backstop" value | a lost wake costs ≥6 % of a 16 ms frame per occurrence on the target, not 0.3 % |
| 15 exposure | fully concurrent, exactly-once, per-task synchronisation | crossbeam | BWoS's per-block form exists (G23) |
| 16 representation | `Box<dyn FnOnce>` per spawn; `TaskHandle` word-sized | rayon `spawn` | rayon `join` uses a caller-frame job; we have no join primitive |
| 17 promotion | eager at every spawn; static grain 1024 rows | Bevy `BatchingStrategy`; rayon thief-splitting (G21) | same shape as Bevy; rayon resets its split budget on migration |
| 18 capacity | unbounded Injector (63-slot linked blocks) + unbounded deque | crossbeam | — |
| 19 helper set | anything, including fire-and-forget tasks (abort guard) — **and a foreign pool's owner-only slot** | rayon, TBB default; rayon `in_worker_cross` (own pool only) | same hazard, documented at `scope.rs:469-476`; plus the cross-pool one (J14) |
| 20 termination | `pending` counter, unpark-before-decrement, 50 µs (→≥1 ms) backstop | rayon latch (count-gated, W20) | — |
| 21 caller context | four-way key in TLS; physics lane count ignores it; the joiner does not consult the pool half of it | rayon (three latch types by membership) | our key is right; two consumers ignore it (S9, J14) |
| 22 elasticity | fixed | rayon, Tokio | UE 5.5 and Chromium are elastic during waits; not a lever for a CPU-bound frame |
| 23 priority/fairness | none; shared injectors probed first, per task | rayon `find_work` (local first) | inverted order vs rayon, Go, Tokio |
| 24 mutation model | conflict bitsets at schedule build (`schedule.rs:1006`) | Bevy | same; Nanos6 consults the dependency graph at completion (S10) |
| 25 what crosses | the task; a wakeup | Go, Tokio | — |
| 26 primitive | `std` park/unpark → `WaitOnAddress` with whole-ms timeouts on Windows | rayon latches | the timeout unit, not the primitive |
| 27 load shape | frame-locked waves, drained to empty between systems | game engines | wake gating matters more here than in a server pool |
| 28 verifiability | loom M1–M3 + Miri; queues abstracted away | — | P/G changes are Miri-visible, loom-invisible; W changes must extend M1/M2/M2b; W20 is M1's subject |
| 29 completion path | unconditional, 2 multi-writer RMWs + conditional syscall per task | rayon/std: count-gated, 1 RMW per task | a per-task cost the first cost model did not see, and a shipped cheaper shape the second did not see |
| 30 topology | **unrecorded** (`lscpu`/`cpuid` never captured for the bench machine) | — | every L-verdict is conditioned on it |
| 31 spawn batching | per task | Tokio/Go (per batch) | N−1 shared RMWs per wave on the table; Tchiboukdjian's theorem prices the fill |
| 32 block detection | none | — | the joiner parks inside a system body undetected |
| 33 fan-out count | one | Go, Tokio | an O(W) wake chain per wave boundary; Wicked/.NET ship wave-conditioned and replica-cascade counts |
| 34 placement key | spawner identity (worker) / n/a (dispatcher → global) | rayon | the idle mask is loaded after placement is committed |
| 35 obligation | mandatory exactly-once | all | — |
| 36 timed-wait resolution | source constants 50 µs / 100 µs; waits ≥1 ms on Windows; no `timeBeginPeriod` | — | every backstop and latency-floor figure in revision 1 was wrong by ≥10× on the target (App-7, H.10) |
| 37 analytical model | none applied | — | ABP Theorem 9 covers A1; Tchiboukdjian/Gast cover the pile and the latency; Karlin covers the spin |
| 38 contention class | not distinguished | — | two of the "four shared RMWs" are single-writer; the criterion was applied on the wrong class |
| 39 selection point | fixed per pool | rayon (per call) | a joined wave and a fire-and-forget spawn share one discipline |
| 40 successor hand-off | via the dispatcher, after its next poll (≥1 ms on Windows) | Nanos6 (inline, p = 0.75); TBB (returned) | the released successor takes a queue op, a park and a wake to start; S10 removes all three |
| 41 deque granularity | per task (crossbeam) | BWoS (per block) | recorded; not a candidate this round |
| 42 cross-pool helper | drains the target pool's `injector_local[own id]` | rayon (own pool only) | one missing pointer comparison (App-6) |

## D. Variant catalogue — index

One hundred and twelve variants in six groups (P0–P25, G1–G24, J1–J14, W0–W21, L1–L13, S0–S10).
`applies_to` uses the brief's tags: **A** defect A, **B** defect B, **M1/M3** the owner's
mechanisms, **Loc** cache locality, **Sched** scheduler-level. Entries marked *(rev. 1)* live in
`KE16-VARIANTS-ADDENDA.md`; *(rev. 2)* in `KE16-VARIANTS-ADDENDA-2.md`.

**Group P — placement and reachability** (`KE16-VARIANTS-PLACEMENT.md`; P17–P22 rev. 1; P23–P25 rev. 2)

| Id | Variant | Applies | Shortlist |
|---|---|---|---|
| P0 | per-worker injector polled only by its owner — **boyko today** (and, cross-pool, by a foreign joiner) | A, M3 | defect cell |
| P1 | spawn to the spawner's own registered deque | A, B, Loc | **A1** |
| P2 | per-worker injector kept, made stealable (in the scan set) | A, M3 | **A2** |
| P3 | every spawn to the global injector | A, Loc | **A3** (control) |
| P4 | bounded local queue, overflow half to global | A | fallback for A1 |
| P5 | affinity mailbox with dual-residency proxy | A, M3, Loc | not shortlisted |
| P6 | direct handoff / push-to-idle, **non-stealable** — the owner's mechanism 3 as rejected by Go/FJP | M3, A, Loc | recorded, not shortlisted |
| P7 | private deques + explicit steal requests (channels) | A, M3 | not shortlisted (and abandoned by Weave's author for shared memory — N63) |
| P8 | split deque, private tail / public head, lazy release | A, B | not shortlisted |
| P9 | lifelines: dormant thief, deferred sender push | M3, A | not shortlisted |
| P10 | non-stealable single LIFO slot (P0 at capacity 1) | A, Loc | not shortlisted |
| P11 | time-window stealability (Kotlin, 100 µs) | A, Loc | not shortlisted |
| P12 | sender pushes to every worker's inbox, per-list atomic index, no stealing | M3, B | not shortlisted |
| P13 | static partition, no transfer | M1, Loc, Sched | measured baseline only |
| P14 | single central queue (lock-based and lock-free MPMC) | A, B | null hypothesis |
| P15 | nothing queued: lazy task creation / stack-as-deque / return barriers | A | not buildable in stable Rust |
| P16 | arena / pool isolation, stealing forbidden across the boundary | A, Loc | the per-pool designed twin of P0 |
| P17 *(rev. 1)* | **sender-chosen placement into a REGISTERED per-worker queue, keyed on the idle mask** — mechanism 3 in its buildable form | M3, A, Loc | **M3-s** |
| P18 *(rev. 1)* | per-worker Injector closed by a placeholder on the registered deque (rayon `JobFifo`) | A | **A2′** |
| P19 *(rev. 1)* | thread-per-core: owner-only queue by design, data-owner-routed SPSC inboxes (Seastar, glommio) | — | the per-thread designed twin of P0 |
| P20 *(rev. 1)* | random shared priority heaps, two-choice take (Julia partr) | — | not a candidate |
| P21 *(rev. 1)* | one shared ring, per-consumer cursors, claim by exchange; own-barrier helping (Jolt) | B | not a candidate |
| P22 *(rev. 1)* | no queue: generation broadcast + claim cursors; nesting banned (ForkUnion) | — | not a candidate |
| P23 *(rev. 2)* | **an owner-only per-worker queue inside a stealing pool, by design** — rayon's broadcast deque (fed only by `inject_broadcast`, wake-all) | — | not a candidate; sharpens P0's record and corrects axes 11/33 for rayon |
| P24 *(rev. 2)* | round-robin sender placement into registered per-thread queues; wake one for a task, all for a wave (Wicked Engine) | M3 | not a candidate; supplies E27's value for W-f and a ninth axis-34 key |
| P25 *(rev. 2)* | rotating start + first-accepting `try_push`; one `try_pop` rotation then block on the own queue, no registry (stlab) | — | not a candidate |

**Group G — granularity and promotion; Group J — the joining thread** (`KE16-VARIANTS-GRANULARITY-JOIN.md`; G18–G20, J12–J13 rev. 1; G21–G24, J14 rev. 2)

| Id | Variant | Applies | Shortlist |
|---|---|---|---|
| G1 | steal one | B | **B2** (joiner only) |
| G2 | steal half into a registered queue | B, A | the worker loop already does this; Tchiboukdjian's theorem prices it |
| G3 | steal half into an unregistered private scratch, drained serially — **boyko today** | B | **B0 keep as is** |
| G4 | steal half into a registered scratch / push residue back | B | **B1** |
| G5 | hierarchical chunk: one near, half far | B, Loc | not until axis 30 is recorded |
| G6 | adaptive one-vs-half per request | B | not shortlisted |
| G7 | steal half the WORK by weight | B | not needed (chunks are already balanced) |
| G8 | eager range split with a grain — `par_iter` and physics chunking today | A, B | application lever |
| G9 | lazy splitting on demand | A, B | not shortlisted |
| G10 | heartbeat promotion (chili; forte, 5 µs beat) | A, B | measurable as a pool replacement; not this round |
| G11 | oracle-guided granularity | B | not shortlisted |
| G12 | batch claiming by a shared atomic index | A, B | alternative shape for `par_iter` |
| G13 | continuation stealing / work-first | A, B | not buildable without coroutines |
| G14 | child stealing / help-first, and SLAW's adaptive switch | A, B | what we are |
| G15 | idempotent, at-least-once extraction | — | disqualified |
| G16 | fence-free by bounded TSO | — | disqualified |
| G17 | weak-memory fence placement (crossbeam lineage) | — | already have |
| G18 *(rev. 1)* | static partition + steal a quarter of the victim's unstarted remainder (libomp `static_steal`) | A, B, Loc | design note for the `par_iter` rework |
| G19 *(rev. 1)* | geometrically shrinking batch claim (libomp guided) | B | `par_iter` knob |
| G20 *(rev. 1)* | **batch spawn: one queue op + one `pending` RMW per WAVE** (Tokio `push_batch`, Go `runqputbatch`) | A, B | **App-4** — now with Tchiboukdjian's O(log2 W) fill behind it |
| G21 *(rev. 2)* | **thief-splitting: split budget halved per split, RESET on migration** (rayon `Splitter`) — a new axis-17 trigger | A, B | **App-5′** design note for the `par_iter` rework |
| G22 *(rev. 2)* | self-replicating task: one queued, each replica that STARTS queues one more (.NET `TaskReplicator`) — a task cascade on axis 33 | M3, B | a **W-f** grid value |
| G23 *(rev. 2)* | block-based work-stealing deque: owner and thieves synchronise per BLOCK (BWoS OSDI'23; NVIDIA stdexec) | — | recorded (axis 41); not a candidate this round |
| G24 *(rev. 2)* | task-scheduler BYPASS: the body returns the next task, never queued (oneTBB preview) | M1, Loc | the zero-sync extreme of axis 40; not a pool candidate |
| J1 | joiner helps with any task | B | what we are, minus the batch sink (libomp moved to J2) |
| J2 | joiner helps within a restricted set (+ Jolt own-barrier; + **libomp Task Scheduling Constraint**, default on) | B | not shortlisted |
| J3 | non-worker caller blocks, no help | B | **B3** (measure) |
| J4 | core handoff on block | B | not applicable |
| J5 | compensation / oversubscription, caller-declared | B, Sched | not applicable |
| J6 | fiber swap (+ marl: fibers with random stealing) | A, B, Loc | not buildable safely |
| J7 | suspend the whole deque | B, M1 | not shortlisted |
| J8 | continuations instead of joins | B, Sched | not shortlisted |
| J9 | task retraction | B | not shortlisted (Godot's retraction of an unstarted task not confirmed — U) |
| J10 | wait-free join counter | B | not applicable (we are help-first) |
| J11 | dispatcher parks and polls, never helps — **boyko's frame path** | B, Sched | established fact; its park is ≥1 ms on Windows |
| J12 *(rev. 1)* | scheduler-observed block → replacement, LIFO release (Linux cmwq, Windows IOCP) | B, Sched | not applicable without a kernel hook — and the user-space form was shipped and withdrawn three times (N59–N61) |
| J13 *(rev. 1)* | time-gated compensation (Chromium MAY_BLOCK threshold + poll) | B, Sched | not applicable |
| J14 *(rev. 2)* | **cross-pool helper set**: rayon `in_worker_cross` helps in its own pool; boyko's joiner drains the target pool's `injector_local[own id]` with no identity check | A (correctness) | **App-6** — one pointer comparison |

**Group W — wake and idle; Group L — locality; Group S — scheduler** (`KE16-VARIANTS-WAKE-LOCALITY-SCHED.md`; W15–W19, L11–L13 rev. 1; W20–W21, S10 rev. 2)

| Id | Variant | Applies | Shortlist |
|---|---|---|---|
| W0 | unconditional wake on every push, rotor RMW before the idle load — **boyko today**; plus two more triggers (W17) | M3 | defect cell |
| W1 | wake gated on empty→non-empty | M3 | **W-b** |
| W2 | wake gated on "no searcher", searcher cap | M3 | **W-b** |
| W3 | jobs-event counter / sleepy handshake | M3 | not shortlisted |
| W4 | park-path re-check / 2PC notifier | M3 | what we are |
| W5 | tolerated lost wakeup + backstop | M3 | what we are — with a ≥1 ms backstop on the target (axis 36) |
| W6 | idle-registry structure and wake order — **boyko's bitmap is here** | M3, Loc | **W-a** (move the rotor RMW) |
| W7 | wake-all / broadcast | M3 | abandoned (rayon, Julia, Unity); by design for rayon `broadcast` (P23) |
| W8 | single spinner / relay | M3 | not shortlisted |
| W9 | silent worker-generated spawn (no notify) | M3, A | with A1/A2, worth measuring |
| W10 | heartbeat-gated publication | M3, A, B | see W15 for the wake-only form |
| W11 | spin budget policy | M3 | **W-c** — now an OCCUPIED cell upstream (.NET, W21), unbuilt here; Karlin prices the length |
| W12 | worker-count feedback | M1, Sched | not applicable |
| W13 | OS blocking primitive | M3 | what we are — timeouts in whole ms on Windows |
| W14 | spawn-path allocation | A, B | with A1, caller-frame jobs become possible |
| W15 *(rev. 1)* | wake-rate throttling on the wake path only (Folly `ThrottledLifoSem`) | M3 | only if W-b leaves no-op wakes |
| W16 *(rev. 1)* | wake fan-out COUNT per event | M3 | **W-f** — grid extended by G22's cascade and P24's wave-conditioned count |
| W17 *(rev. 1)* | the completion-path signal: boyko's unconditional per-task unpark + the joiner's pre-park wake | M3 | round 1's **W-d**, now the fallback to W-d′ |
| W18 *(rev. 1)* | topology-nearest / recently-spinning wake target | M3, Loc | empty cell E21; needs axis 30 |
| W19 *(rev. 1)* | user-mode monitor/wait as the idle primitive (WAITPKG; ForkUnion) | M3 | empty cell E22 |
| W20 *(rev. 2)* | **count-gated completion: only the LAST completer signals; the wake target outlives the scope** (rayon `CountLatch`/`CoreLatch`; std `thread::scope`) | M3 | **W-d′** |
| W21 *(rev. 2)* | decoupled idle spin on ONE signal word, queues scanned after acquisition (.NET) — the occupant of E18 | M3 | W-c's shipped form |
| L1 | LIFO owner / FIFO thief | Loc, A | **App-2** — strengthened by FJP's `asyncMode` condition |
| L2 | victim order: random+rotation vs fixed 0..n vs coprime | Loc, B | **App-3** |
| L3 | sticky / last-victim | Loc | not shortlisted |
| L4 | hierarchical / NUMA | Loc | not until axis 30 is recorded |
| L5 | steal-back / localized WS | Loc | proven price |
| L6 | space-bounded / footprint anchoring | M1, Loc | not shortlisted |
| L7 | cache-sized split | Loc | application lever |
| L8 | stable entity→thread partition across sync points | M1, Loc | see G18 |
| L9 | cache-line padding / false sharing | Loc | already have |
| L10 | working-set migration cost of handing over a system | M1, Loc | **unmeasurable today** |
| L11 *(rev. 1)* | parallel depth-first scheduling / constructive sharing of one LLC | M1, Loc | theory for axis 30's shared-LLC case |
| L12 *(rev. 1)* | heterogeneous-core awareness (libomp 60/40 skew; Bender–Rabin) | Loc | record axis 30 first |
| L13 *(rev. 1)* | sub-LLC affinity scopes with a non-strict escape; 500 µs cache-hot threshold (Linux) | M1, Loc | the shape to measure once axis 30 is recorded |
| S0 | dynamic hand-out via the global injector race + apply-window barrier — **boyko today** | M1, Sched | established |
| S1 | ready-set executor spawning each system as a task | M1, Sched | what we are |
| S2 | cost-weighted static partition of systems to lanes | M1, Sched | empty cell |
| S3 | systems are not the scheduling primitive | M1, Sched | contrary evidence |
| S4 | main-thread-only graph scheduling, nested forbidden | Sched | contrary evidence |
| S5 | periodic migration / load compaction (BEAM; CFS pull) | M1, Sched | not shortlisted |
| S6 | apply-window early release | M1, Sched | **M1-b** (measure first) |
| S7 | heartbeat at the system level | M1, Sched | empty cell |
| S8 | emit a DAG, host schedules | Sched | not applicable |
| S9 | caller-aware lane count for the physics sites | A (application) | **App-1** |
| S10 *(rev. 2)* | **immediate successor: the completing CPU runs the released dependency successor with probability p** (Nanos6, default 0.75) | M1, Loc, Sched | **M1-c** design note — the cheapest answer to the owner's locality objection; presupposes M1-b |

## F. Empty cells of the axis grid

Cells no lens or refuter found a shipped or published occupant for, and the cells whose "empty"
verdict a revision overturned (kept under their original numbers so the log stays readable). Each
is either worth building for measurement, or is not, with the reason.

| # | Cell | Nearest occupant | Worth building? |
|---|---|---|---|
| E1 | **per-worker injector in the steal scan set** (axis 1 × 11) — `P2` | occupied in the reachability sense by rayon `JobFifo` (→P18); FJP's interleaved scan array is the scan-set precedent; rayon's broadcast deque (→P23) is the owner-only twin *by design* | **Yes** as A2, with P18 as A2′ — the extra idle-path cost of P2 is W−1 `SeqCst` fences per round (not epoch pins); P18 has none but needs P1's TLS pointer |
| E2 | **steal-half into a registered scratch** (axis 3 × 11) — `G4` | every steal-half runtime lands the batch in the thief's *registered* queue (crossbeam `dest: &Worker<T>`, Go `runqsteal`, Tokio `steal_into`) | **Yes** — nobody publishes "steal into a private queue and run serially" because it is not a design, it is a bug; the fix is a `.stealer()` registration or a push-back of the residue |
| E3 | **wake gating applied to the WAKE only, eager publication** (axis 12) | **OCCUPIED — Folly `ThrottledLifoSem`** (→W15) | **Maybe** — measure only if W-b leaves no-op wakes on the table |
| E4 | **idle-registry-driven chunk count** (axis 3 × 13) | every splitter uses a static grain, an empty-deque test, a timer — or a migration signal (rayon →G21) | **No** — A-STEAL and BWS measured that counting idle workers and feeding them loses on throughput (§N); the registry read is a shared-line load on the spawn path; G21's "reset on migration" gets the adaptivity without the read |
| E5 | **bitmap idle registry + most-recently-parked hint** (axis 13 × 7) | FJP/Kotlin/Folly wake LIFO; boyko rotates | **Only if** wake order is shown to matter (W-f measures the count first) |
| E6 | **dual-residency proxy mailbox in Rust** (axis 1 × 25) — `P5` | TBB `task_proxy` | **No** — double bookkeeping and a CAS per extraction; the motivation is unfalsifiable without L10 |
| E7 | **heartbeat promotion at the ECS system level** (axis 8 × 17) — `S7` | HBC (C loops), TPAL (assembly); forte at the pool level | **Not now** — needs a promotable representation of a running system body (opaque closures); the *pool-level* form is buildable (G10) |
| E8 | **cost-weighted static partition of systems to lanes** (axis 8 × 17) — `S2` | HEFT/CPOP; no game engine | **Not before an instrument** — needs per-system timing history and a locality model |
| E9 | **stable entity→thread partition with a steal fallback** (axis 7 × 2) | **OCCUPIED at loop level — libomp `kmp_sch_static_steal`** (→G18) | **Design note for the `par_iter` rework** — presupposes reachability (P1–P4) |
| E10 | **lane affinity keyed on component-column identity** (axis 7) | TBB `affinity_partitioner` (loop identity), NUMA-WS (data placement), Linux `cache_shard` scope (topology); Nanos6's inline successor is the cheapest relative (→S10) | **Instrument first** — the only design that would answer the owner's cache objection to mechanism 1 with data; `pin_workers` is a stub and no residency instrument exists |
| E11 | **steal half the WORK by archetype row count** (axis 3) — `G7` | Configurable Strategies | **No** — chunks are already equal rows / slot-balanced |
| E12 | **time-window stealability with batch stealing** (axis 7 × 3) — `P11` + `G2` | Kotlin (single tasks, 100 µs window) | **No** — for µs chunks the window would be sub-µs |
| E13 | **work-requesting over channels in a frame-locked pool** (axis 2) — `P7` | HPX, Weave — and Weave's author replaced it with shared-memory stealing for the non-distributed case (N63) | **No** — the victim polls on its working path; transfer is a round trip |
| E14 | **continuation stealing without compiler support** (axis 9 × 16) — `G13` | libfork (C++20 coroutines) | **No** — coroutines or segmented stacks |
| E15 | **apply-window early release** (axis 8, scheduler) — `S6` | Bevy's executor (no such barrier) | **Yes, measure** — independent of the pool; may be the larger share of the observed lane drain (A.5); S10 builds on it |
| E16 | **move the rotor RMW behind the idle load** (axis 10) — `W6` | Go `wakep` (load first) | **Yes, trivially** |
| E17 | **push-to-idle as NON-stealable placement** (axis 1 × 2 × 13) — `P6` | rejected by Go (three reasons) and FJP; NA-RP measured >100 ns per pushed task | **Recorded as the owner's proposal in its strong form; not shortlisted.** The *stealable* form is occupied (→P17) — candidate M3-s |
| E18 | **spin budget decoupled from the steal scan** (axis 6 × 10) — `W11` | **OCCUPIED — .NET `LowLevelLifoSemaphore.WaitSlow`** (→W21): the spin reads one signal word, no queue; `Dispatch()` runs after the semaphore is acquired. Round 1's "no occupant" was wrong — the file was already in the `[S]` list for its 35 µs constant | **Yes, as W-c in its shipped form** — presupposes a push-side signal (every enqueue posts), so it is W1/W2 + a decoupled spin; Karlin's spin-block theorem sets the length (axis 37) |
| E19 | **LIFO owner end on the pool's deques** (axis 7) — `L1` | universal; FIFO-both-ends is rayon's deprecated `breadth_first` (N51) and FJP's `asyncMode` for tasks "that are never joined" | **Yes, one constructor argument** — ours are joined, which is FJP's exclusion |
| E20 | **the joiner's victim sweep randomised and self-skipped** (axis 4) — `L2` | worker loop already does it | **Yes, trivially** |
| E21 *(rev. 1)* | **bitmap idle registry + topology-nearest wake order** (axis 13 × 30) — `W18` | Linux `select_idle_sibling` (kernel), marl (recently-spinning) | **After axis 30 is recorded** — one AND on the wake path, no new RMW |
| E22 *(rev. 1)* | **user-mode monitor/wait as the idle primitive** (axis 26) — `W19` | ForkUnion (TPAUSE/WFET, `[R:B]`); no surveyed pool | **Not before the wake protocol is settled**; needs a `cpuid` gate and a fallback |
| E23 *(rev. 1)* | **batch spawn in boyko** (axis 31) — `G20` | Tokio `push_batch`, Go `runqputbatch` (occupied upstream; unbuilt here) | **Yes** — App-4; N−1 shared RMWs per wave, independent of placement; Tchiboukdjian's Theorem 3 says a pile fills in O(log2 W) steal rounds |
| E24 *(rev. 1)* | **two-choice victim sampling for a work-stealing scan** (axis 4) | Julia partr samples two heaps (→P20); Mitzenmacher `[P*]`. Round 1 also called steal-granularity theory "measurements only" — wrong: Tchiboukdjian/Gast give closed forms for steal-half with random victims (axis 37) | **No** for two-choice — a full random-start sweep already finds a non-empty victim in O(W) probes at W=16; the steal-half theorems are now attached to G2/G20/G12/G18/P17/W-f |
| E25 *(rev. 1)* | **an advisory task class** (axis 35) | GHC sparks (`[S]`); no pool with mandatory tasks | **No for `par_iter` chunks**; design note for a future speculative-split or prefetch spawn |
| E26 *(rev. 2)* | **a per-SCOPE queue registered in the scan set** (axis 1 × 11 × 19 × 29): the wave lives in a queue owned by the `Scope` object, reachable through a registry of live scopes; own-scope-first helping and the completion counter share one owner | none shipped; nearest: G12's single descriptor, Jolt's per-barrier job list over a shared ring (P21), boyko's `active_scopes` — a counter, not a registry (`thread_pool.rs:152` `[L]`) | **Design note, not this round** — it is the only reachability closure that needs neither A1's TLS deque pointer nor A2's scan-set growth; its price is a live-scope registry probed on the steal path, i.e. one more shared structure per steal. Recorded as theory-only |
| E27 *(rev. 2)* | **wave-conditioned wake fan-out in a stealing pool with batch spawn** (axis 33 × 31) | Wicked ships "one for a task, all for a wave" in a non-stealing pool (→P24); .NET's replica cascade (→G22) | **Yes, inside W-f** — the wave size N is known at the one push App-4 leaves, so the count can be `min(N, sleepers)` for a wave and 1 for a single spawn at no extra cost |
| E28 *(rev. 2)* | **count-gated completion with a registry-owned wake target in boyko** (axis 29) — `W20` | occupied upstream (rayon `CountLatch`, std `ScopeData`); unbuilt here; boyko's own comment claims it is impossible (refuted, N66) | **Yes — W-d′**: one RMW per task instead of two plus a conditional syscall; the memory-safety order is replaced by a lifetime decision (the joiner's parker lives in `WorkerHandle`/a dispatcher slot, not in `ScopeShared`) |
| E29 *(rev. 2)* | **per-call placement selection** (axis 39) in a pool with one joined and one fire-and-forget spawn kind | rayon (`spawn`/`spawn_fifo`/`broadcast`), FJP (`externalSubmit`), TBB (`spawn`/`enqueue`) | **Design note** — a joined `par_iter` wave and `ThreadPool::spawn` (no production caller, H.5) need not share a discipline; A1–A5 currently force one; FJP's `asyncMode` condition ("never joined") is the documented criterion |
| E30 *(rev. 2)* | **inline successor at the ECS-system level** (axis 40): the worker that finishes system S runs the successor it just released, no queue op, no dispatcher round trip | Nanos6 at the task level (→S10, p = 0.75); TBB bypass at the body level (→G24); nobody at the system level | **Design note M1-c** — presupposes M1-b (readiness released per completion) and the conflict check; removes a `park_timeout` (≥1 ms on Windows) + a queue op + a wake from every dependency edge; keeps S's outputs on the core that wrote them — the cheapest answer to the owner's locality objection |
| E31 *(rev. 2)* | **a calibrated spin budget** (axis 6 × 37): spin for the measured park+wake round trip, or a random length on [0, C] | .NET (calibrated to ~35 µs); Karlin's theorems; boyko's 127-pause budget is calibrated to nothing | **Yes, as part of W-c** — but the round trip on Windows includes the ≥1 ms timer granularity only for timed waits; an untimed park's wake latency is the number to calibrate against (H.10) |

## G. Shortlist for design — NOT a decision

What the architect should design and the implementers should build for measurement on the harness
grid (§A.7). Each row is a candidate with the reason it is on the list; nothing here ranks them. The
number from the real consumers (`ke16_solve_in_system`, `ke16_par_iter_in_system`) decides, under the
criterion **throughput, not occupancy**. **Precondition for every number (rev. 2):** App-7 — the
harness so far ran with ≥1 ms park backstops on Windows; the 1 µs and 10 µs grid cells must be
re-taken after the timer resolution is either measured and accepted or raised.

**Defect A — reachability (mechanism 2).** Build A1–A3; A3 is the control; A2′ and A5 are optional rows.

| Id | Variant | Why it is on the list | What to watch |
|---|---|---|---|
| **A1** | `P1` — same-pool worker spawn goes to the worker's **own registered deque** | every production runtime; rayon does exactly this after the same identity check we already perform; spawn path becomes a Chase-Lev push (plain store, no RMW); the sibling reaches it through the existing `stealers[wid]`; opens the door to W14 (caller-frame jobs). **Theory (rev. 2):** ABP Theorem 9 covers this shape — arbitrary DAG, "either choice" of push/execute, `O(W/p + D)` — so the expected direction at every grid cell is a theorem, not an inference; the 1 µs × 64W cell is still a measurement (the constant, not the direction) | needs a TLS pointer to the worker's `Worker<TaskHandle>` (`scope.rs:17-21` becomes false); a Tree-Borrows obligation for Miri (axis 28); **promotes B to the production path** (A.4) |
| **A2** | `P2` — keep `injector_local`, add it to the steal scan set | smallest structural delta; makes the false doc comment true; FJP's interleaved-scan precedent | W−1 extra `SeqCst` fences per idle round (not epoch pins); the joiner still drains its own injector into `scratch` first; the spawn-path ops stay on a single-writer line only until a sibling probes it (axis 38) |
| A2′ *(rev. 1)* | `P18` — keep `injector_local`, close it by a placeholder `TaskHandle` on the worker's registered deque (rayon `JobFifo`) | zero extra idle probes; the same crossbeam types; rayon measured it "equivalently" | needs A1's TLS pointer anyway; one extra `Injector::steal` per executed task; buys FIFO-for-spawns only if the deques go LIFO (App-2) |
| **A3** | `P3` — same-pool worker spawn goes to `injector_global` | one-line change; the reachability floor; async-executor/Bevy ship it; the control against which A1/A2's locality claim is tested | the two Injector ops move from a single-writer line to a MULTI-writer line (axis 38) — the class change, not a new instruction; zero producer-consumer affinity |
| A4 | `P4` — bounded local + overflow half to global | the shape Tokio and Go converged on; removes the unbounded Injector | larger change; only if A1's TLS pointer is judged unsound |
| A5 = **M3-s** *(rev. 1)* | `P17` — push into an idle sibling's `injector_local[target]` chosen from the idle mask `push_task` already loads; own injector when the mask is zero; siblings scan injectors (A2's scan set) | **the owner's mechanism 3 in the only form the record does not reject**; the wake and the placement finally agree; costs P0's push on a FOREIGN line (axis 38) and nothing else. **Theory (rev. 2):** Gast et al.'s latency term `16.12·λ·log2(W/2λ)` is what "beats A1 by one steal latency" is measured against | NA-RP's >100 ns per pushed task predicts a loss at 1–10 µs bodies; possible win at ≥100 µs when siblings are parked at a wave boundary; the joiner's `scope.rs:479` drain applies (A.4) |

**Defect B — the joiner's batch (route (a) today, route (b) after any A-fix).** Build B0–B3 and
measure them **in the A-fixed configuration**, not today's.

| Id | Variant | Why it is on the list | What to watch |
|---|---|---|---|
| **B0** | `G3` — **keep as is** | owner-required candidate; half-batching is universal and amortises one CAS over ≤32 tasks; on the engine frame path B never fires (A.3); the only production caller that will hit it after A is the route-(b) joiner; **theory:** Tchiboukdjian Theorem 3 bounds what steal-half buys the *other* thieves | the joiner's serial residue is up to 33 tasks on physics waves, 8 on `par_iter` waves at W=16 (A.4); `shutdown.rs:22-31` already documents the liveness hazard; **rev. 2:** the alternative to running the residue inline is parking, and a park is ≥1 ms on the target — B0's comparison point moved |
| **B1** | `G4` — register a `Stealer` for `scratch` (or push the residue back to the joiner's registered deque under A1) | the only change that keeps the amortisation and removes the sink; every steal-half runtime does this by construction | registering a `Stealer` is what *adds* epoch pins to sibling probes of the scratch; a stack-lived deque's stealer must be unregistered before the frame returns; under A1 `scratch` can be deleted |
| **B2** | `G1` on the joiner only — `Stealer::steal` / `Injector::steal` (one task) | nothing is ever parked privately; rayon's helper takes one job at a time | one CAS per task on the joiner only; the worker loop keeps batching |
| **B3** | `J3` — the non-worker joiner does not help (parks, rayon `in_worker_cold`) | removes route (a)'s oversubscription-by-one and the serial sink at once; `diag_lane.rs` says the joiner rarely wins the race anyway | on the bench path the helper is a lane; the frame path is unaffected either way; a parked joiner sleeps in ≥1 ms quanta on Windows unless the wake is reliable (W-d′) |

**Mechanism 3 — the wake protocol.** Independent of A and B and of each other. W-d′ is new in
revision 2 and supersedes W-d.

| Id | Variant | Why it is on the list | What to watch |
|---|---|---|---|
| **W-a** | `W6` — move `wake_rotor.fetch_add` inside the `mask != 0` branch | one multi-writer RMW per spawn removed; zero behavioural change; `worker.rs:320-324`'s own comment says the rotor carries no data | none |
| **W-b** | `W1` or `W2` — gate the push-side wake on `queue_was_empty` (FJP/rayon) or on a searching count (Go/Tokio) | Go rejected our current shape by name; Tokio measured +14 %/+25 % on its official harness from removing no-op wakes `[D]`; on a frame-locked load shape (axis 27) an ungated wake almost always hits a parked worker | the last-searcher race (Tokio's compensating wake; Go's StoreLoad on both sides); must extend loom M2/M2b; keep the backstop — **which is ≥1 ms on Windows, so a lost wake now costs ≥6 % of a frame** (App-7); gating only the push-side wake leaves W17's two per-task wakes ungated |
| **W-c** | `W11`/`W21` — spin on ONE word (the idle bitmap's line, or a "work exists" word), run the O(W) scan every k-th round or on a change | our spin budget is coherence traffic, not spinning; **the decoupled form is shipped** (.NET `WaitSlow`, W21) — round 1 called this cell empty | .NET's form presupposes a push-side signal on every enqueue, so W-c composes with W-b rather than replacing it; the spin LENGTH has a theorem (Karlin: spin for the block cost; random on [0, C] is e/(e−1)) — calibrate against the measured untimed wake latency (E31, H.10) |
| **W-d′** *(rev. 2)* | `W20` — count-gated completion: `if pending.fetch_sub(1) == 1 { wake }`; move the joiner's wake target OUT of `ScopeShared` into a registry-owned slot (the worker's `WorkerHandle`; one slot for the dispatcher) so the last completer can wake after the count hits zero | rayon `CountLatch` and std `thread::scope` ship exactly this: one RMW per task instead of boyko's two multi-writer RMWs plus a conditional syscall; boyko's own comment ("learning we are last… too late") is refuted — the counter returns the previous value, the constraint was the waker's lifetime (N66); for a flat wave the completion path runs as often as the spawn path | must extend loom M1 (the count gate IS M1's subject); the wake target's lifetime must be independent of the scope allocation; the round-1 flag design (W17) stays as the fallback if the target must remain inside `ScopeShared`; the pre-park `unpark_one_idle` at `scope.rs:511` is reviewed under the same gate |
| W-e *(rev. 1)* | `W15` — throttle the wake to at most one per interval (Folly `ThrottledLifoSem`) | shipped; bounds the syscall rate with eager publication | a knob; only if W-b/W-d′ leave no-op wakes |
| **W-f** *(rev. 1, grid extended rev. 2)* | `W16` — wake count ∈ {1, FJP cascade, **.NET replica cascade (G22)**, `min(N, sleepers)` (rayon/Jolt), **wave-conditioned: 1 for a task, `min(N, sleepers)` for a wave (P24, E27)**} on a wave push | with W−1 siblings parked, wake-one is an O(W) serial wake chain per wave boundary (Bevy #10064's "one thread at a time"); `min(N, sleepers)` pays W syscalls on the spawner up front; the wave-conditioned count is free once App-4 batches the spawn; Gast's λ term prices the chain | measure on the grid at W ∈ {4, 16}; interacts with W-b's gate |
| — | `P6` push-to-idle as non-stealable placement | **not on the list**; recorded in E17; the stealable form is A5 = M3-s above | — |

**Mechanism 1 — another system for an idle lane.**

| Id | Variant | Why it is on the list | What to watch |
|---|---|---|---|
| **M1-a** | instrument the split between the apply-window barrier (A.5) and the pool | mechanism 1 cannot be sized without it; assignment is already dynamic, so there is no hand-out to build until the binding gate is known | the harness's inter-system test gives 25.1 % top lane for conflict-free systems — the measurement needs a *conflicting* schedule with unequal system costs; **the dispatcher's park between rounds is ≥1 ms on Windows**, which may itself be a large share of the observed drain (H.10) |
| **M1-b** | `S6` — apply-window early release: clear a finished system's `running` bit and decrement successors' `pred_remaining` without waiting for the whole round | the only scheduler-level variant that addresses the owner's observation directly; Bevy's executor has no such barrier | the barrier exists for a reason (SCH7: the drain holds `&mut world` exclusively for deferred application); early release must not touch the apply, only the readiness bits |
| **M1-c** *(rev. 2)* | `S10` — inline successor: the worker that finishes system S and releases S′ runs S′ itself when S′ is conflict-free against the running set, with no queue op, no dispatcher round trip and no wake | Nanos6 ships it at the task level (`immediate_successor`, p = 0.75) for "cache data reutilization between successor tasks"; TBB's bypass is the zero-sync extreme; **it is the cheapest possible answer to the owner's locality objection** — S′'s inputs are what S just wrote, on this core | presupposes M1-b; a chain of inlined successors is a serial lane, hence Nanos6's probability; the conflict check against the running set is the same test the dispatcher runs; deferred-command visibility (SCH7) applies; nobody does it at the ECS-system level (E30) |
| **M1-0** *(rev. 1)* | **record axis 30** for the bench machine (`lscpu`/`cpuid`: SMT pairs, L2 clusters, L3 domains, core classes) | every locality verdict in this document is conditioned on a topology nobody captured | one command; no design |
| — | `L10` / `E10` locality instrument | the owner named cache locality as the axis that decides mechanism 1; nothing in the tree can measure it; Linux's 500 µs cache-hot threshold (L13) is the only shipped numeric proxy | build before deciding L-anything |

**Application level.**

| Id | Variant | Why it is on the list |
|---|---|---|
| **App-1** | `S9` — lane count from caller context: `W` on a worker (after A), never `+1`; delete the dead `lanes < 2` guard or make it ask who is calling | three sites carry the same wrong arithmetic; the guard was written for the condition that is exactly the production condition and cannot see it |
| App-2 | `L1` — `Worker::new_lifo()` for the worker deques | one constructor argument; the field's free locality lever; FIFO-both-ends is rayon's deprecated `breadth_first` (N51) **and** FJP's `asyncMode`, which FJP documents as "for forked tasks that are never joined" — boyko's `Scope` tasks are joined |
| App-3 | `L2` — randomise and self-skip the joiner's sweep | restores the worker loop's discipline in `scope.rs:534-541` |
| **App-4** *(rev. 1)* | `G20` — batch spawn: `pending.fetch_add(N)` once per `par_iter`/physics wave and one queue insertion per wave | N−1 shared RMWs per wave removed on the spawn path, independent of which A-candidate wins; Tokio and Go both batch injection; **theory (rev. 2):** a wave published as a pile is Tchiboukdjian's model — W workers fill in O(log2 W) steal rounds |
| App-5 *(rev. 1)* | `G18`/`G19` — for the `par_iter` rework: owner-range partition with steal-from-remainder, and a shrinking batch | design notes; occupied upstream by libomp; presuppose reachability |
| App-5′ *(rev. 2)* | `G21` — thief-splitting for the `par_iter` rework: a split budget of W, halved per split, reset on "my chunk migrated" | rayon's shipped `par_iter` policy; the trigger is free (the worker id at run time); no deque-emptiness proxy (which defect A inverts) and no timer |
| **App-6** *(rev. 2)* | `J14` — add `std::ptr::eq(tls::active_pool_ptr(), inner)` to `on_worker` in `join_workers_until_drained` | the join side lacks the pool-identity check `push_task` has; a pool-A worker joining a pool-B scope drains B's `injector_local[own id]` today `[L]`; one pointer comparison; extend `tests/cross_pool_routing.rs` to the join side |
| **App-7** *(rev. 2)* | axis 36 — measure the real `park_timeout(50 µs)` latency on the bench machine; then choose: `timeBeginPeriod(1)` at boot (documented power/scheduler cost), a spin-until-deadline backstop, or accept ≥1 ms and make lost wakes rarer (W-b, W-d′) | every backstop and latency-floor figure in revision 1 was a Linux number; `dur2timeout` rounds 50 µs and 100 µs both to 1 ms on this toolchain `[L]`; nothing in the tree raises the resolution; the harness numbers were taken this way |

**Doc corrections owed by whichever variant lands** (from A.2): `thread_pool.rs:123-130`,
`worker.rs:355-358`, `lib.rs:47-48`, `scope.rs:17-21`, **`scope.rs:149-151`** (the "too late"
rationale), `solver/colored.rs:2628-2639`, `soft/colored.rs:990-995`, `resources.rs:1466-1496`, the
`50 µs` / `100 µs` doc comments at `scope.rs:430-431,512` and `schedule.rs:683` (they describe a
constant, not a wait), and the stale line references in `tests/loom_pool.rs:22,24,140,343` and
`tests/miri_scope.rs:6-7` (probe's list).

## H. Open questions and blocked items

1. **Route (a) in production.** The probe found no non-worker thread that opens a scope with tasks
   in flight outside benches and tests, but grepped only four crates. A workspace-wide grep is
   owed before "B is bench-only" is relied on.
2. **B after A.** Every A-candidate (including A5) makes B reachable on route (b) (A.4). The B
   measurement must be scheduled after the A candidate is chosen, or the B candidates must be built
   against each A candidate. An ordering constraint on the implementers, not a design choice.
3. **Cache locality is unmeasurable.** No affinity, no residency instrument. Mechanism 1 and every
   L-variant are blocked on an instrument nobody has specified; M1-0 (record axis 30) is the cheap
   precondition; M1-c (S10) is the one M1 shape whose locality claim is structural rather than
   measured (the successor reads what the predecessor wrote).
4. **The apply-window share.** Unmeasured. It may dominate the owner's observation (A.5) — and on
   Windows the dispatcher's between-round park is ≥1 ms, which M1-a must separate out.
5. **`ThreadPool::spawn` has no production caller** in the four crates grepped; if that holds
   workspace-wide, the fire-and-forget arm (and its abort-on-panic policy) can change freely, and
   axis 39 (per-call placement) has only one production caller kind to serve.
6. **Instrument scope.** Five untracked files and three `Cargo.toml` hunks carry the red-first gate
   and every number this campaign will produce; a fresh clone of the branch has none of them.
   Whether they are committed is an owner scope call.
7. **The two 2026-08-30 censuses disagreed** because of the U1 bimodality (A.3). Every future
   measurement on route (b) must record `outer_worker_id`.
8. **W-d′'s lifetime decision is a soundness question first.** Moving the wake target out of
   `ScopeShared` removes the reason for unpark-before-decrement; the loom M1 model must be extended
   to the count gate before it is built (axis 28). The round-1 flag design (W17) remains the
   fallback.
9. **Model routing** for the KE16 phases is recorded in `docs/aether-v2/KERNEL-BACKLOG.md` §KE16
   (Fable where a decision is made; Opus elsewhere). This document makes no decision.
10. **The timer resolution in effect on the bench machine is unmeasured** (axis 36). The rounding
    to whole milliseconds is verified in the toolchain `[L]`; the default system resolution's value
    is not on the Microsoft page (commonly quoted 15.625 ms — `[B]`, U79); whether another process
    on the machine had raised it during the 2026-08-30 census is unknown. App-7 measures it; until
    then every 1 µs / 10 µs grid cell is suspect.
11. **Cross-pool joins in production.** Whether any production caller joins a foreign pool's scope
    (J14) is unknown — the same class as H.1. App-6 is correct regardless.
12. **BWoS / stdexec** (G23): whether stdexec's per-thread `remote_queue` inbox is stealable was
    not read; it decides where that design sits on axis 11 (U71). Not a candidate this round.

## I. Revision log

### Refutation round 2 (2026-09-02) — two refuters, both accepted in full

**Refuter 3 (theory, empty cells, and the target OS).** Accepted, all seven wrong claims and all
five missing variants/cells, plus three axes. *Wrong claims:* (1) P1/A1's "the Blumofe–Leiserson
bound does not cover a help-first flat wave; the applicable analysis is Guo et al." — the applicable
theorem is **ABP Theorem 9** ("arbitrary multithreaded computations as opposed to the special case
of 'fully strict'"; "The bounds proven in this paper hold for either choice") — re-read via proxy
`[P]`, P1 and A1 rewritten, axis 37 added. (2) `scope.rs:149-151`'s "learning we are last would
require reading `pending` after the sub — too late", quoted approvingly in W17/W-d/§H.8 — refuted:
`fetch_sub` returns the previous value; rayon `CountLatch::set` and std `ScopeData::
decrement_num_running_threads` gate on `== 1` and keep the wake target alive independently of the
scope (both re-read verbatim `[S]`); **W-d′ = W20** supersedes W-d; N66 recorded. (3) The cost
table's rayon COMPLETE column ("swap + wake when SLEEPING", `[R:S]`) understated rayon by the wave
size — per task rayon pays one `fetch_sub`; the swap and conditional wake are once per scope; and the
"unbuilt parked flag" cell is occupied by `CoreLatch`'s UNSET/SLEEPY/SLEEPING/SET — corrected, now
`[S]`. (4) Every backstop and latency-floor figure (axis 14 "50 µs", W5 "0.3 % of the frame", J11
"100 µs floor", W-d "covered by the 50 µs backstop") was a Linux number: on
nightly-x86_64-pc-windows-gnu `park_timeout` → `WaitOnAddress(dur2timeout(d))` rounds to whole
milliseconds (`[L]`, toolchain source), Windows 10 2004+ does not raise the resolution for a process
that never calls `timeBeginPeriod` (`[D]`, re-read), and nothing in the tree does — **≥1 ms, ≥6 % of
a frame per occurrence**; axis 36, App-7, H.10 added; A.2, A.3, A.7, W5, J11, G3, B0, B3, W-b, M1-a
corrected. (5) E18 "no occupant" — .NET `WaitSlow` spins on one signal word and touches no queue
(`[S]`, re-read); W21 added, W-c re-verdicted "occupied upstream, unbuilt here". (6) "4 shared
RMWs, the most expensive spawn path in the survey" conflated lock-prefixed instructions with
cache-line transfers: the two `Injector` ops on `injector_local[wid]` are single-writer under P0 —
axis 38 added, every cost row annotated. (7) Steal-half / two-choice theory recorded as
"measurements only" — Tchiboukdjian–Gast–Trystram (Theorems 2, 3, 6) and Gast et al. (Theorem 4.1,
Lemma 4.3) re-read via proxy `[P]` and attached to G2, G20, G12, G18, P17, W-f; E24 re-verdicted.
*Missing variants/cells:* W20 (count-gated completion), G21 (rayon thief-splitting, `[S]`), G22
(.NET `TaskReplicator`, `[S]`), W21 (.NET decoupled spin, `[S]`), E26 (per-scope queue, theory-only).
*Notes accepted:* Karlin (Theorems 6, 7 re-read via proxy `[P]`) attached to W11/W-c/E31; Hagerup's
"Bold" recorded (N67, `[R:P]`); the "no runtime uses a bitmap" claim in W6 narrowed to user-space
pools (U80).

**Refuter 4 (shipped runtimes and negative results).** Accepted, all five wrong claims, all
fourteen items, and all five axes. *Wrong claims:* (1) J1 listed libomp `taskwait` as an
unrestricted helper — `kmp_global.cpp` `int __kmp_task_stealing_constraint = 1; /* Constrain task
stealing by default */` re-read `[S]`; libomp moved to J2 with a fourth restriction kind (tied-task
descendants, untied exempt, binds thieves too). (2) Axis 11 "rayon = closed by construction" and
axis 33 "rayon = min(new_jobs − idle, sleeping)" — rayon's **broadcast deque** is owner-only by
design with a wake-all (`inject_broadcast` re-read `[S]`); P23 added, both axes corrected. (3) A.2's
"`injector_local[i]` is read only with the caller's own id" holds within one pool only: the joiner
has no `active_pool_ptr == inner` check (`scope.rs:441-443`; grep of the crate `[L]`) — a pool-A
worker joining a pool-B scope drains B's slot of its own index; J14, axis 42, App-6 added. (4) N51
presented FIFO-both-ends as the field's deprecation verdict — FJP ships it as `asyncMode` under a
stated condition ("forked tasks that are never joined", `[D]` re-read); L1/App-2 strengthened, not
weakened. (5) J12's "not available to a user-space pool" stated as a design fact — it was shipped
and withdrawn (Windows UMS `[D]` re-read), removed (NetBSD SA `[D]` + `[R:D]`), never merged (Linux
UMCG, LWN `[D]` re-read); N59–N61 added, axis 32 extended. *Placed:* BWoS/stdexec (G23, axis 41,
`[P]` via proxy + `[R:S]`), Wicked (P24, `[S]`), stlab (P25, `[S]`), Nanos6 immediate successor
(S10, axis 40, `[D]`; default 0.75), TBB scheduler bypass (G24, `[D]` uxlfoundation mirror — Intel's
page 403), rayon `in_worker_cross` (J14, `[S]`), FJP `externalSubmit`/`asyncMode` (axis 39, `[D]`);
negative results ConcRT (N62, `[D]` re-read), Constantine (N63, `[D]` re-read), GTS (N65, `[R:D]`).
*Axes:* 39 (selection point), 40 (successor hand-off), 41 (deque granularity), 42 (cross-pool
helper); axis 33 extended with the push-kind-conditioned count rather than a new axis.

**Re-verified locally this round `[L]`:** `scope.rs:120-180` (`register_task`, `complete_task`,
`is_drained`), `scope.rs:425-545` (the join loop), `thread_pool.rs:118-165` (`PoolInner` fields incl.
`active_scopes`), `worker.rs:350-380` (`push_task`), the `active_pool_ptr` call sites, the
`timeBeginPeriod` grep, and the toolchain's `std` parking path.

**Not re-opened this round** (carried at the refuters' kind, listed in §U): NVIDIA stdexec
`static_thread_pool.hpp`; NetBSD's stated SA reason (rmind PDF); lore.kernel.org UMCG posts; ConcRT
Scheduler Policies page; oneTBB issue #1523; Intel GTS; Kruskal–Weiss, GSS, Hagerup.

### Refutation round 1 (2026-09-01) — two refuters

**Refuter 1 (theory and empty cells).** Accepted, all nine wrong claims and all ten missing
variants: the Injector-epoch error (every cost row for P0/P2/P3/W11 and the cost table corrected;
verified at crossbeam-deque 0.8.7 `[L]`); E3 occupied (W15); E9 occupied (G18); E18's nearest
occupant withdrawn (rayon scans every round — verified `[S]`; **round 2 found the real occupant**);
P6's "no production runtime" narrowed to the non-stealable form, the stealable form catalogued as
P17 and shortlisted as M3-s; A-STEAL "≈2·T1" corrected to Theorem 12's parametric form (N58);
L4/G5's "collapses on a single socket" downgraded to an unrecorded assumption (axis 30, M1-0); axis
12 given boyko's two missing triggers and the cost table a COMPLETE column (W17); P1's Lemma 12 /
Theorem 13 attribution corrected (**and round 2 corrected the correction**: the applicable theorem
is ABP's). Added axes 29–31 and variants P17, G18, G19, G20, W15, W18, W19, L11, L12.

**Refuter 2 (shipped systems).** Accepted: rayon `JobFifo` (P18); forte (G10); Jolt (P21); PhysX
(P17); Julia partr (P20); cmwq/IOCP (J12); Chromium (J13); Linux affinity scopes (L13); CFS pull
balancing (L13, S5); Halide A/B teams (W16); GHC sparks (axis 35, E25); marl (W18, J6);
Seastar/glommio (P19); ForkUnion (P22); Bevy's axis-9 cell corrected (U45 closed); axis 22's "all
game engines" corrected; G10's "needs a promotable representation" narrowed to the loop level. Added
axes 32–35, negative results N51–N57, U30 closed. Declined as a separate variant: O3DE.

Registers: negative results in `KE16-EVIDENCE.md` §N (67 entries); unverified claims in
`KE16-EVIDENCE.md` §U (87 entries plus the U-0 method caveat, never silently dropped; three closed
in round 1, none in round 2); sources by kind in §S.
