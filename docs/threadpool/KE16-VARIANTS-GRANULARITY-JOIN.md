# KE16 — Variants, group G: transfer granularity and promotion; group J: the joining thread

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`.
Negative results `N…` and unverified claims `U…` are in `KE16-EVIDENCE.md`. Cost anchors as in
`KE16-VARIANTS-PLACEMENT.md`. **G18–G20** (static partition + steal-from-remainder, geometric batch
claim, batch spawn) and **J12–J13** (scheduler-observed and time-gated compensation), added in
refutation round 1, are in `KE16-VARIANTS-ADDENDA.md`.

Group G is where defect B's *mechanism* lives (a half-batch amortises one CAS) and where the
nested-parallelism model is decided (help-first vs work-first, eager vs lazy vs heartbeat). Group J
is where defect B's *site* lives (what the joiner does with the batch). Read A.4 in the index first:
every A-fix makes group J's behaviour reachable on the production path.

---

## Group G — granularity and promotion

### G1 — Steal one

**Axis cell.** 3: one task · 10: one CAS per task transferred · 7: the stolen task is the oldest =
the largest remaining subtree in a fork-join tree.

**Implemented by.** rayon-core `registry.rs::WorkerThread::steal` (random start, `victim.stealer
.steal()`, one job) `[S]`; oneTBB `task_dispatcher.h::steal_or_get_critical` `[S]`; Java FJP
(polls one, then re-polls the same queue "which also reduces bookkeeping, cache traffic, and
scanning overhead") `[S]`; LLVM libomp `__kmp_steal_task` (body outside the fetched window — U);
Cilk `[P]`.

**Documented rationale.** In a recursively split tree one old task is a whole subtree, so batching
buys nothing (TBB's breadth-first-steal rationale) `[D]`. Batch stealing is the answer for FLAT task
sets — which is what `par_iter` chunks and physics chunks are.

**Evidence.** rayon's full sweep from a random start costs P−1 probes on a miss `[S]`. Dinan: steal-one
"degrad[es] past 128 processors" while steal-half sustains >95 % at 256 `[P]`. HotSLAW: on structured
graphs (fib, nqueens, UTS T1L/T2L) steal-one/small chunks win; "performance degrades severely with
ChunkSize ≥ 8"; unstructured T3L wants 16 `[P]`.

**Failure modes.** Under a flat wave of N equal tasks, N CASes on the victim's line to fill the pool
(vs ~4 with halving); with random victims, O(P) failed probes per success when the pool is mostly
empty.

**Cost.** STEAL: one CAS + one cache-line transfer per task.

**Fit.** B: **candidate B2 on the joiner only** — nothing is ever parked privately; the worker loop
keeps batching. The cost is one CAS per task on the *joiner's* path, which after an A-fix is the
route-(b) worker that opened the scope. Prognosis: pays at 1 µs bodies (≥33 CASes for a physics
wave), free at 100 µs.

---

### G2 — Steal half into a REGISTERED queue

**Axis cell.** 3: half the victim's queue, capped · 10: one CAS amortised over ≤32 (Stealer) / ≤33
(Injector) · 11: the residue is re-stealable because the destination is registered.

**Implemented by.** crossbeam-deque `steal_batch_with_limit` — `min(len.div_ceil(2), limit)`,
`MAX_BATCH = 32`, Injector limit `MAX_BATCH + 1` `[S]`/`[L]` (0.8.7, `deque.rs:18,705,946,1760`);
Go `runqgrab`/`runqsteal` (`n = n - n/2`, into the thief's own runq) `[S]`; Tokio `steal_into2`
(`n - n/2`, into the thief's registered queue) `[S]`; async-executor `steal` (`(len+1)/2` bounded by
destination capacity, into the runner's registered local queue) `[S]`; Unity job system ("steals
half of a native job's remaining batches at a time, to ensure cache locality") `[D]`; Dinan SC'09
(steal-half "to maximize the number of work sources") `[P]`; Hendler & Shavit PODC'02 (O(log k) CAS
for k ops; no experiments) `[P]`; boyko's **worker loop** (`worker.rs:222,228,252` → the worker's
registered deque) `[L]`.

**Documented rationale.** crossbeam: "around half of the tasks in the queue, but also not more than
some constant limit"; the count "is considered an implementation detail" `[D]`. Go/Tokio: amortise
the atomic. Unity: cache locality. Dinan: more work sources.

**Evidence.** Dinan UTS: steal-half "transferred an overall average of 3.8 tasks per load balancing
operation" `[P]`. Go's global pop is batched too (`runqsize/gomaxprocs + 1`) `[S]`. Every steal-half
runtime read lands the batch in the thief's registered queue — none lands it somewhere unstealable
`[S]`/`[I]`.

**Failure modes.** Half of a small queue is one task, so amortisation vanishes at the contended tail.
Half the COUNT can be a small fraction of the WORK when tasks are unequal `[I]` (→G7). A half moved to a thief that
immediately runs out of parallelism is worse than one task and destroys the victim's LIFO locality.

**Cost.** STEAL: one CAS per ≤32 tasks.

**Fit.** B: this is what the amortisation argument *actually* supports — batching into a place other
thieves can still reach. Our worker loop is here; our joiner is at G3. **Theory (rev. 2, axis 37):**
steal-half against a uniformly random victim has a closed form — Tchiboukdjian, Gast, Trystram
Theorem 3: for W unit independent tasks on m processors, `E[Cmax] ≤ W/m + 3.24·(log2 W + 1/(2 ln 2))
+ 1`, "optimal up to a constant factor in log2 W"; with a steal latency λ, Gast et al. Theorem 4.1:
`E[Cmax] ≤ W/p + 16.12·λ·log2(W/2λ) + 3λ` `[P]` (both re-read via proxy). Round 1 called this
granularity "measurements only" (Dinan, HotSLAW) — it is not.

---

### G3 — Steal half into an UNREGISTERED private scratch, drained serially (boyko today; candidate B0)

**Axis cell.** 3: half, capped 32/33 · 5: joiner runs the residue inline, one at a time, no
`is_drained` re-check · 11: **broken** — `scratch` has no `Stealer`.

**Implemented by.** boyko `scope.rs::join_workers_until_drained` (`let scratch = Worker::new_fifo()`
at 448, "Not exposed"; `steal_batch_and_pop(&scratch)` at 479/488/496; `drain_scratch` 524-531)
`[L]`. **No upstream adherent**: crossbeam's `steal_batch_and_pop(dest: &Worker<T>)` takes a
registered `Worker` precisely so the batch stays stealable; Go, Tokio, async-executor all land in a
registered queue `[S]`.

**Documented rationale.** `scope.rs:427-439`: the joiner "steals work from any stealable source… and
runs the stolen tasks inline"; the batch is the pool's normal batch. `shutdown.rs:22-31` already
documents the consequence as a liveness hazard: a blocking task in the batch "would wedge the
dispatcher while its sibling tasks sit unrun in the same scratch deque → deadlock" `[L]`.

**Evidence.**
- Magnitude: up to 33 per Injector grab, 32 per Stealer grab `[L]`; physics waves at W=16 exceed the
  63-slot block (A.4), `par_iter` waves are ≤W.
- Route (a) datum conflict: "4–5 of 16 simultaneously live" vs `diag_lane.rs`'s zero joiner tasks
  over ~24 runs vs the harness's "peaks at 16 in flight and still spends most of its wall-clock on one
  lane" `[L]` — the signature is **per-lane work share**, not max-in-flight (index A.3).
- On the ECS frame path this variant never fires (`is_drained()` true on the first iteration) `[L]`.
- Under any A-fix it fires on route (b) (index A.4) `[L]`/`[I]`.

**Failure modes.** Converts a parallel wave into a serial run of ≤33 on the joiner; a blocking task
in the batch deadlocks (documented); priority inversion (the joiner runs unrelated work while its own
scope is one task from done).

**Negative results.** none — nobody publishes this; it is a bug's shape, not a design's.

**Cost.** STEAL (joiner): one CAS per ≤33 tasks — the cheapest steal path in the file. IDLE: n/a.
The price is paid in **wall-clock**, not in atomics.

**Fit.** B: **candidate B0 — keep as is.** The owner requires it reportable. The case for it: the
amortisation is real; the frame path never hits it; `diag_lane.rs` says the joiner rarely wins the
race on cheap bodies; route (a) has no production caller found. The case against it: after any
A-fix the route-(b) joiner is the worker that opened the scope, it *always* reaches its own wave
first (`scope.rs:479`, or `stealers[wid]` under P1), and a 200 µs × 33 serial residue is 6.6 ms
on one lane. **Rev. 2 — the comparison point moved:** the alternative to running the residue inline
is parking, and on the Windows target a `park_timeout(50 µs)` is a **≥1 ms** wait (axis 36), so B0
is measured against a joiner that sleeps in millisecond quanta whenever it loses the race, not one
that re-polls every 50 µs. **Decidable only by measurement in the A-fixed configuration** (index §G,
§H.2), after App-7 settles what the backstop really is.

---

### G4 — Steal half into a registered scratch / push the residue back (the minimal fix)

**Axis cell.** 3: half, capped · 5: joiner runs one, publishes the rest · 11: closed.

**Implemented by.** **Nobody, because nobody has G3.** The mechanism is crossbeam's own contract:
`steal_batch_and_pop` into a `Worker` that has a `.stealer()` registered in `inner.stealers`. Two
concrete shapes: (i) register a `Stealer` for the joiner's `scratch` for the duration of the join
(a stack-lived deque whose stealer must be unregistered before the frame returns); (ii) under P1 the
joining *worker* already owns a registered deque — delete `scratch` and steal into that deque via
the TLS pointer P1 introduces. Lens 1's empty cell (2); lens 5's fix note.

**Documented rationale.** Inherited from G2.

**Evidence.** crossbeam signature `[S]`; Go/Tokio destinations `[S]`.

**Failure modes.** (i) a dynamic stealer registry (today `Arc<[Stealer]>` is built once at
`thread_pool.rs:594-599`) — a slot per worker for "the joiner's scratch" is the cheap version; the
dispatcher thread would need its own slot. (ii) only exists once P1 exists.

**Cost.** STEAL: unchanged (one CAS per batch). The residue costs one more CAS per re-steal by a
sibling — span-proportional, not work-proportional.

**Fit.** B: **candidate B1** — keeps the amortisation, removes the sink; strictly smaller than
removing the batch. Prognosis: dominates G3 whenever the residue is larger than the steal cost, i.e.
at every body size ≥ 10 µs; at 1 µs the re-steal CAS is the whole task.

---

### G5 — Hierarchical chunk selection: steal one near, half far

**Axis cell.** 3: 1 intra-domain, half inter-domain · 4: hierarchical · 7: cache-domain aware.

**Implemented by.** HotSLAW HCS (Min, Iancu, Yelick PGAS'11) `[P]`.

**Documented rationale.** "HCS method shows the highest average performance for all chunk sizes" —
the parameter-free default `[P]`.

**Evidence.** Geomean at 256 cores: hand-tuned Fixed-Chunk 116.4×; HCS 106.1×; StealHalf 94.9×;
Random+StealHalf 83.3× `[P]`; HCS vs Random+StealHalf +27 % (a second figure of 122 % in the same
extraction is inconsistent — U).

**Failure modes.** 9 % below hand-tuned; needs a topology model; on a single socket the hierarchy is
L1/L2/L3, not nodes.

**Fit.** Loc/B: the prior text said the hierarchy "collapses on our target" — that is unrecorded,
not measured: the target class has sub-LLC clusters and the kernel measured the trade-off there
(→L13, axis 30). Not shortlisted until axis 30 is recorded.

---

### G6 — Adaptive steal-one vs steal-half, re-decided periodically

**Axis cell.** 3: adaptive · 17: adapted quantity = steal granularity.

**Implemented by.** Weave (`WV_StealAdaptativeInterval=25`) `[D]`; HPX work-requesting
(`stealhalf_` flag per request) `[S]`.

**Documented rationale.** Neither granularity wins across workloads (Weave README `[D]`; HotSLAW
`[P]`).

**Evidence.** Weave's "3×–10× less overhead" — author's README, no harness `[B]`.

**Fit.** B: an adaptive knob over G1/G2 that presupposes G4 (a stealable destination). Not
shortlisted; the harness grid (body × tasks) is the static version of the same decision.

---

### G7 — Steal half the WORK, not half the tasks

**Axis cell.** 3: half the estimated work via a transitive weight · 17: task-weight metadata.

**Implemented by.** "Configurable Strategies for Work-stealing" (arXiv:1305.6474, 48-core Opteron)
`[P]`.

**Evidence.** ~2× on graph bipartitioning; no gain on quicksort `[P]`.

**Fit.** B: not needed here — `par_iter` chunks are equal rows (`par_iter.rs:393-397` `[L]`) and
physics chunks are slot-balanced (`colored.rs:2640-2648` `[L]`), so half the count is already half
the work (E11).

---

### G8 — Eager range splitting with a grain (what `par_iter` and the physics sites do)

**Axis cell.** 3: split-in-half or equal chunks decided eagerly at spawn · 8: application · 17: eager
to a fixed threshold.

**Implemented by.** boyko `par_iter.rs::BatchingStrategy::chunk_size` — `clamp(N / (W ×
batches_per_thread), 1024, ∞)`, `batches_per_thread = 1`, `MIN_ARCHETYPE_FOR_PARALLEL = 1024`
(archetypes under 1024 rows never spawn) `[L]`; physics `n_chunks = clamp(lanes × CHUNKS_PER_WORKER,
1, n)` with `CHUNKS_PER_WORKER` 4/6/6 `[L]`; Bevy `BatchingStrategy` (`batch_size =
max_items.div_ceil(thread_count × batches_per_thread)`) `[S]`; oneTBB partitioners `[D]`; Molecule
`CountSplitter(256)` / `DataSizeSplitter(32 KiB)` `[B]`; rayon `with_min_len` `[D]` — rayon's
actual split policy is **thief-splitting** (a budget of W splits, halved per split, reset when the
job is observed to have migrated), which is neither eager-to-a-grain nor lazy (→G21, rev. 2 `[S]`);
Unity `innerloopBatchCount` `[D]`.

**Documented rationale.** boyko `par_iter.rs:68`: "a single `scope.spawn` costs ~120 ns (plan
§10.3)" `[L]` — the grain amortises the spawn. Unity: "Starting at 1 and increasing the batch count
until there are negligible performance gains is a good strategy" `[D]`. Bevy: "assumes each entity
has roughly the same amount of work" `[S]`.

**Evidence.** Tzannes et al. PPoPP'10: the optimal stop-splitting threshold depends on thread count,
iteration count AND calling context; LBS beat `auto_partitioner` by 38.9 % (default) / 16.2 %
(platform-tuned), `simple_partitioner` by 56.7 % (sst=1) / 19.5 % (trained on another dataset) `[P]`
(lens 4 could not open the PDF; lens 1 did via proxy). Su et al. ASPLOS'24: static chunking on
irregular data is unbalanced `[P]`.

**Failure modes.** A per-call-site constant is wrong on another machine; over-splitting floods, under-
splitting starves; **under defect A the grain is irrelevant because no chunk leaves the spawner**.

**Negative results.** N39.

**Cost.** SPAWN: `ceil(N/chunk)` spawns, each the full spawn path.

**Fit.** A/B: the application-level lever; the chunk *count* is what the joiner's batch is measured
against (≤W chunks for `par_iter` at `bpt=1`; 68–102 for physics at W=16). After an A-fix,
`batches_per_thread` and `CHUNKS_PER_WORKER` are the knobs the harness can sweep. S9 fixes the lane
count these formulas take.

---

### G9 — Lazy splitting on demand

**Axis cell.** 3: split-in-half only when a proxy says someone is hungry · 17: local deque empty
(LBS), a thief arrived (work-stealing tree, enkiTS split-on-steal, Weave), hysteresis (DF2-LS).

**Implemented by.** XMT LBS (Tzannes et al. PPoPP'10) and Lazy Scheduling DF-LS/BF-LS/DF2-LS
(TOPLAS'14) `[P]`; enkiTS `SplitTask` (`gc_MaxStolenPartitions`: a thief takes a coarse slice so it
is not immediately re-stolen from) `[S]`; Weave "adaptative lazy loop splitting" `[D]`; Prokopec's
lock-free work-stealing iterator (owner CASes a `progress` field per chunk; thief `markStolen()`)
`[P]`.

**Documented rationale.** LBS: "checks if the local deque is empty and only then splits"; the
decision is revocable — "lazy scheduling does not make irrevocable serialization decisions" `[P]`.

**Evidence.** Deque-check cost light (no fence) but count linear in iterations, hence a profitable-
parallelism threshold (ppt, T = 1000 cycles) `[P]`. DF-LS "fails to scale beyond ~8 workers" with
fine-grained code; BF-LS and DF2-LS fix the deep-first bias `[P]`. Prokopec vs TBB: step workload
TBB 25 % slower, exponential 2× slower `[P]`.

**Failure modes.** The empty-deque proxy is wrong under nesting (DF2-LS adds hysteresis). **Under
defect A the proxy is inverted: the deque is empty while the injector is full.** One CAS per chunk on
the owner's path (work-stealing tree). Assumes an indexable domain — true for a dense column, false
for a fragmented archetype set.

**Negative results.** N39.

**Cost.** SPAWN: one local read per split point (LBS) or one CAS per chunk (tree).

**Fit.** A/B: an alternative shape for `par_iter` that never creates a task object until a thief
arrives; presupposes reachability (a thief must be able to *see* the splittable range). Not
shortlisted; noted for the `par_iter` redesign that KE15 (dense chunk runner) will force.

---

### G10 — Heartbeat promotion

**Axis cell.** 1: nothing queued until promoted · 2: sender, rate-limited · 3: one promotion per
beat · 9: latent parallelism promoted on a timer · 10: fork = a few plain stores, zero atomics; all
sync amortised over ~100 µs · 17: timer.

**Implemented by.** Acar, Charguéraud, Guatto, Rainey, Sieczkowski PLDI'18 (theory; PDF not
readable by lenses 2, 4, 5 — lens 1 read it via proxy) `[P]`; TPAL PLDI'21 `[P]`; HBC ASPLOS'24
`[P]`; spice (Zig) `[B]`/`[D]`; chili (Rust) `src/lib.rs` — dedicated heartbeat thread, per-scope
`AtomicBool` checked 1-in-64, `shared_jobs` BTreeMap under a global Mutex touched only on a beat
`[S]`; **forte** (Rust, Bevy-motivated) README, re-read this session: "Approximately every 5us
(gated by the CPU's instruction counter) if there's space available, each worker pushes a small
number of jobs into this queue"; "Each worker also has a small fixed-capacity work-stealing queue
(currently each has space for 32 jobs)"; "Jobs created by `join` are executed in LIFO order… the
oldest `join` job is promoted"; "Jobs created by `spawn` are executed in FIFO order… the newest
`spawn` jobs are grouped into small batches (16 jobs each)"; "Up to 32 threads can participate in a
pool"; "a lower-overhead, lower-latency alternative to `rayon_core`"; **no benchmark numbers** `[D]`.
Named by Bevy maintainers as the bevy_tasks 0.17 direction (#18510 comment — search snippet only,
`[R:D]`).

**Documented rationale.** "The cost of creating and managing parallelism can be amortized over the
useful work done between heartbeats"; promote the oldest promotable frame when ≥ N cycles elapsed
`[P]`. spice: fork = "one memory store to a pointer to somewhere on the stack, one memory store to
the current stack frame, one register store"; the global mutex is uncontended because "only a single
thread is executing a heartbeat" `[B]`. spice's reasons for abandoning deques: "Every piece of work
is a dynamic dispatch"; "The local work queue isn't really local"; "Spinning works great… until it
doesn't" `[B]`.

**Evidence.**
- Theorems: `work(d_h) ≤ (1 + τ/N)·work(d_s)`, `span(d_h) ≤ (1 + N·τ)·span(d_p)` `[P]` (via proxy —
  the extraction that lens 4 saw was degraded; numbers from that extraction are excluded, U).
- PBBS: thread-creation overhead "sometimes over 25 %"; heartbeat overhead "always less than 5 %";
  ≥ one order of magnitude fewer threads `[P]`. TPAL: 13.8× lower task-creation overhead vs Cilk Plus
  `[P]`. HBC: 21.7× geomean vs OpenMP dynamic 14.2× at 64 cores; polling ≈50 cycles/software poll,
  ≈3800/interrupt; +58.46 % on spmv-arrowhead `[P]`; **"HBC underperforms on balanced workloads;
  OpenMP static scheduling remains superior for non-irregular cases"** `[P]`.
- spice/chili numbers (sub-ns/node; chili 3.51× vs rayon on M1; chili *slower than serial* at 1023
  nodes) — authors' READMEs, single benchmark kind `[B]`.

**Failure modes.** A promotable *representation* is needed only for the chunk LOOP (G9/G12
territory); for the pool itself a not-yet-stolen join job or a spawn batch IS the promotable unit —
chili and forte ship heartbeat behind closure-based fork-join APIs with no resumable splitter, so
the prior text's "needs a promotable representation… in Rust" conflated the two levels. Real costs:
♥ tuned per machine; a wave shorter than a few beats never goes parallel; a regular dense `par_iter`
IS the balanced case heartbeat loses on; spice self-declares "zero testing coverage".

**Negative results.** N49.

**Cost.** SPAWN: a local counter check (1-in-64 in chili). STEAL: a global mutex, at ≤10 kHz.
IDLE: condvar.

**Fit.** A/B: the principled alternative to a task queue; the beat interval vs a 16 ms frame with
~µs-to-ms systems is unexplored (E7). Measurable on the harness grid as a *pool replacement*
(forte's `join`/`spawn`/scope is rayon_core-shaped); not shortlisted this round because it replaces
the pool rather than fixing A, and forte publishes no numbers. The wake-only gating sub-variant is
W10; its shipped occupant is Folly `ThrottledLifoSem` (→W15).

---

### G11 — Oracle-guided granularity

**Axis cell.** 3: sequentialise below `α·κ` predicted cost · 17: learned online.

**Implemented by.** Acar, Aksenov, Charguéraud, Rainey PPoPP'19 `[P]`.

**Evidence.** κ = 25–500 µs on modern hardware, α ∈ [1.2, 5]; overhead "close to 5 % or smaller"
except one 13 %; nested BFS up to 84 % faster `[P]`.

**Failure modes.** Needs a per-loop abstract cost function, cycle counters, per-machine κ; excludes
ill-balanced right-leaning trees.

**Fit.** Not shortlisted; `par_iter`'s 1024-row floor is the static version.

---

### G12 — Batch claiming by a shared atomic index (no per-item queue traffic)

**Axis cell.** 1: one descriptor per parallel-for in a queue; the iteration space is never queued ·
2: receiver claims batches by `fetch_add` · 3: fixed batch · 10: one atomic per batch per worker.

**Implemented by.** Godot `worker_thread_pool.cpp::_process_task` (`group->index.postincrement()`)
`[S]`; Unity `IJobParallelFor` `innerloopBatchCount` ("the job queue steals 32 iterations and then
performs them in an efficient inner loop") `[D]`; id Tech 5 per-list `currentJob.Increment()` `[S]`;
Narkowicz's ParallelFor `[B]`.

**Documented rationale.** Keep the shared queue free of per-item traffic; "a simple atomic increment
is enough to safely pick the next batch" `[B]`; Godot distributes to all workers by default `[D]`.

**Evidence.** Godot's collaborative wait: a pool thread keeps processing tasks; a non-pool thread
blocks on a semaphore; `_notify_threads` wakes idle first `[S]`. Godot refuses a wait from inside a
task when it could deadlock (`ERR_BUSY`) `[D]` (N30).

**Failure modes.** One contended line (the index) at high core counts and small batches; no
frame-to-frame affinity; tail latency floor = one batch.

**Cost.** SPAWN: one descriptor + one wake. STEAL: one `fetch_add` per batch. IDLE: unchanged.

**Fit.** A: an alternative *shape* for `par_iter` — one task object per wave instead of `ceil(N/1024)`
boxes; presupposes the descriptor is reachable (i.e. any of P1–P4). Worth a design note for the
KE15 chunk-runner rework; not a pool variant. Shipped relatives: per-consumer claim cursors on one
ring (Jolt →P21; ForkUnion →P22), a stable owner range with steal-from-remainder (libomp
`static_steal` →G18), a geometrically shrinking batch (guided →G19), a self-replicating claimer
that queues one more replica each time one starts (.NET `TaskReplicator` →G22, rev. 2). Theory for
the claim race: Tchiboukdjian et al. Theorem 3 (axis 37).

---

### G13 — Continuation stealing / work-first

**Axis cell.** 1: the CONTINUATION goes on the deque; the child runs now · 5: whoever holds the
continuation reaches the join · 9: fork-join with a cactus stack · 16: two-clone / TLMM / stacklets
/ coroutine frames.

**Implemented by.** Cilk-5 (THE protocol; PLDI'98) `[P]` (lens 2 abstract-only, lens 1 via proxy);
Cilk-M TLMM stacks `[P]`; Nowa (wait-free join) `[P]`; libfork (C++20 coroutines) `[P]`; X10
`[P]`; Fibril (not opened — U).

**Documented rationale.** N3872: continuation stealing keeps extant tasks ≤ P; child stealing "spawns
all n tasks before executing any of them… space proportional to n" `[D]`. libfork: better cache
locality ("the child task is likely to use data the parent task has loaded") and preserves serial
order `[P]`. Cilk-5 work-first principle: move overhead off the work onto the critical path `[P]`.

**Evidence.** Space `M_P ≤ P·M_1` `[P]`. libfork at 112 cores: 7.2× faster than OpenMP, 2.7× than
TBB; `T1/T_S` on fib: libfork 8.8, OpenMP 41, TBB 57, Taskflow 180 `[P]`. Nowa vs Cilk Plus 1.62×,
vs TBB 3.84×, knapsack regresses to 0.36× `[P]`. Cilk-5 spawn "2 to 6 times the cost of a C
function call"; 27–115 ns/spawn `[P]`. Guo et al.: for a FLAT loop from one worker, work-first needs
"P−1 steals and these steals must occur sequentially", `O(t_steal·P)`; FJ(1024) fixed work-first
4.6× slower than help-first `[P]`.

**Failure modes.** Needs a cactus stack or coroutines — not in stable Rust; deep recursion overflows
(SLAW: spanning tree beyond 62.5K nodes failed); **flat waves are its bad case**.

**Negative results.** N40.

**Fit.** Not buildable here (E14). The transferable fact: **our workload is flat, which is
help-first's good case** — the policy is right, the queue is wrong.

---

### G14 — Child stealing / help-first, and SLAW's adaptive switch

**Axis cell.** 1: the CHILD task object goes on a queue; the spawner continues · 5: parent joins its
children · 9: async-finish · 16: a heap closure (rayon `HeapJob`; boyko `Box`).

**Implemented by.** rayon `Scope::spawn` → `inject_or_push` `[S]`; oneTBB, PPL, OpenMP tied tasks
`[D]` (N3872); Habanero help-first `[P]`; SLAW (adaptive work-first↔help-first per spawn site,
INT = 64, hard stack/task budgets) `[P]`; boyko `Scope::spawn` `[L]`.

**Documented rationale.** Guo et al.: "stealing can be performed in parallel"; removes the stack
depth limit `[P]`. SLAW: neither fixed policy is safe; start help-first, switch by observed steal rate,
"overhead under 5 %" `[P]`.

**Evidence.** Help-first beat work-first on FJ(1024) by 4.6×, SOR, CG, LUFact; lost on fib(35) fine
threshold by 10.2× `[P]`. SLAW tracks the better fixed policy 0.98×–9.2× / 0.97×–4.5× on Niagara 2
`[P]`. "Space bound is not guaranteed" `[P]`. Kumar: heap frames ≈ half of X10's 4.1× overhead `[P]`.

**Failure modes.** Every spawn allocates a task object (rayon `HeapJob`, boyko `Box`) — the
allocation principle 5 forbids in the hot path; unbounded live children if spawn rate > steal rate;
buried joins in distributed settings.

**Negative results.** N40, N49.

**Fit.** This is what we are. A: correct policy for flat waves. W14 addresses the allocation.

---

### G15 — Idempotent, at-least-once extraction

**Implemented by.** Michael, Vechev, Saraswat PPoPP'09 `[P]`.

**Evidence.** LIFO 1.55–4.92× vs Chase-Lev on microbenchmarks; redundant tasks avg 2 %, max 6 %
`[P]`.

**Fit.** **Disqualified**: a system body writing components is non-idempotent; double execution breaks
change-detection ticks and every `&mut` aliasing argument.

---

### G16 — Fence-free by bounded TSO

**Implemented by.** Morrison & Afek ASPLOS'14 (FF-THE/THEP) `[P]`.

**Evidence.** Fence is up to ≈25 % of single-threaded CilkPlus time (Figure 1); THEP +11 %/+13 %
average on Westmere-EX/Haswell, dropping to 3.6 %/7 % with SMT `[P]`.

**Fit.** **Disqualified**: correctness rests on a microarchitectural constant (store-buffer depth)
that is not an architectural contract and cannot be expressed in Rust.

---

### G17 — Weak-memory fence placement (the crossbeam lineage)

**Implemented by.** Lê, Pop, Cohen, Zappa Nardelli PPoPP'13 `[P]` (lens 5 could not open; lens 1 via
proxy); crossbeam-deque is the Rust stand-in `[S]`.

**Evidence.** Optimised orders ≥1.5× vs SC everywhere at low contention; gains shrink with grain
(fib 1.19–1.3×, matmul 1.03–1.1×) `[P]`. The fence on the owner's take path is the expensive
instruction on x86 (relative throughput peaks >50 % vs >85 % on ARM) `[P]`.

**Fit.** Already have. Relevant only if the target grows aarch64 (x86 testing does not exercise the
orders).

---

## Group J — what the joining or blocked thread does

### J1 — Joiner helps with ANY task in the pool

**Axis cell.** 5: helps, unrestricted · 19: anything · 4: same as a worker.

**Implemented by.** rayon `latch.rs::Latch::wait(owner)` → `wait_until_cold` (`take_local_job` →
steal → `pop_injected_job`, "finish what we started before we take on something new") `[S]`; oneTBB
`local_wait_for_all` → `receive_or_steal_task` `[S]`; ~~libomp `__kmpc_omp_taskwait`~~ — **moved
to J2 in rev. 2**: it spins calling `execute_tasks` *under the Task Scheduling Constraint*, which is
on by default (`kmp_global.cpp` `int __kmp_task_stealing_constraint = 1; /* Constrain task stealing
by default */` `[S]`), so it is a restricted helper, not an unrestricted one; Godot
`_wait_collaboratively` `[S]`; Bitsquid `[B]`; enkiTS `WaitforTask`
with a priority floor (`priorityOfLowestToRun_`) `[S]`; boyko `join_workers_until_drained` `[L]`.

**Documented rationale.** rayon grants helping by membership — a member executes inline, a member of
another pool helps (`SpinLatch`), a non-member blocks (`LockLatch`) `[S]`. Molecule: "wait until the
job has completed. in the meantime, work on any other job" `[B]`.

**Evidence.** oneTBB documents the correctness hazard with `ets.local() = i; parallel_for(...);
assert(ets.local()==i); // May fail!` — a waiting thread "might execute other available tasks" so
two outer iterations can share one thread `[D]`. boyko `scope.rs:469-476` documents the same class
(fire-and-forget tasks in the same queues; an unwind out of the helper abandons the join → UAF from
safe code; hence `run_task`'s abort guard) `[L]`.

**Failure modes.** Unbounded stack growth (helper frames nest under the joined frame); priority
inversion; TLS corruption; **boyko-specific: the private serial batch (G3)**.

**Negative results.** N21, N22 (Epic's four reasons — three are correctness/latency, one waste).

**Cost.** No extra cost over normal work-finding.

**Fit.** B: we are here minus the batch sink. J1 + G4 = "rayon's helper with batching".

---

### J2 — Joiner helps within a restricted set

**Axis cell.** 5: helps, restricted · 19: own theft chain (FJP) / own isolation region (TBB) /
**task-tree descendants of the last suspended TIED task, untied exempt** (libomp) /
deeper-than-awaited (leapfrogging) / own scope only / own barrier (Jolt).

**Implemented by.** Java FJP `helpJoin` ("linear helping": each worker records `source` = the queue
it last stole from) and `helpComplete` `[S]`; oneTBB `this_task_arena::isolate` (per-thread, opt-in)
`[D]`; **LLVM libomp Task Scheduling Constraint** (rev. 2) — `kmp_global.cpp`: `int
__kmp_task_stealing_constraint = 1; /* Constrain task stealing by default */` `[S]` (re-read);
`kmp_tasking.cpp::__kmp_task_is_allowed`: "Check if the candidate obeys the Task Scheduling
Constraints (TSC) only descendant of all deferred tied tasks can be scheduled, checking the last one
is enough"; `__kmpc_omp_taskwait` passes `__kmp_task_stealing_constraint` to `flag.execute_tasks`
`[R:S]` — the permitted set is defined by the task TREE and tiedness, applies to steals as well as
to the helper, and untied tasks are exempt (round 1 listed libomp under J1 as unrestricted — wrong);
Wagner & Calder leapfrogging (PPoPP'93) `[P]`; Wool/Lace (leapfrogging default) `[P]`; Sukha
SPAA'09 depth-restricted lower bound (could not open — U); Jolt Physics `BarrierImpl::Wait` runs only
the barrier's own jobs — a shipped game-physics occupant (→P21) `[R:S]`.

**Documented rationale.** FJP: "avoid context switching or adding worker threads when one task would
otherwise be blocked… just by running that task or one of its subtasks" `[S]`. Wagner & Calder: the
depth rule buys deadlock freedom and a stack bound `[P]`.

**Evidence.** FJP pays one recorded integer per steal `[S]`. Lace uts T3L: leapfrogging alone 20×,
plus random stealing 36× on 48 cores `[P]`. Wagner & Calder: >90 % efficiency above ≈750 instructions
per task `[P]`.

**Failure modes.** Strictly less reachable work; cores idle while the pool has work off the permitted
path (measured 20× vs 36×); provenance bookkeeping on the steal path.

**Negative results.** N43.

**Fit.** B: would fix the TLS/re-entrancy hazard and REDUCE occupancy — under the throughput criterion
that is a measurement, not an assumption. Not shortlisted. Note boyko is on the *unrestricted* end
already (`try_steal_any` walks every stealer).

---

### J3 — Non-worker caller blocks, contributes nothing

**Axis cell.** 5: parks on a mutex+condvar · 19: nothing.

**Implemented by.** rayon `Registry::in_worker_cold` ("This thread isn't a member of *any* thread
pool, so just block", `LockLatch`) `[S]`; Godot non-pool threads (semaphore) `[S]`; Folly; .NET.

**Documented rationale.** A non-member has no worker context (no deque, no index, no RNG); letting it
steal means synthesising one `[S]`.

**Evidence.** rayon encodes the policy in the latch *type* — no runtime branch on the hot path `[S]`.
boyko's dispatcher-path 7.69× includes the helper as a lane `[L]` — but `diag_lane.rs` says it
executed zero of 4096 tasks across ~24 runs `[L]`; the two data are at different body costs.

**Failure modes.** Wastes a core if the caller is the only thing scheduled; but helping from a
non-worker oversubscribes the pool by one.

**Cost.** One condvar wait instead of a spin/steal loop.

**Fit.** B: **candidate B3** — removes route (a)'s serial sink and its oversubscription at once;
the frame path (J11) is unaffected either way. Measure: at 200 µs × 4W the helper-lane's share
(`top_lane`) says what is lost — and (rev. 2) a parked joiner on Windows sleeps in ≥1 ms quanta
unless its wake is reliable (axis 36, W20), so B3 and W-d′ are measured together. The third
membership case — a member of ANOTHER pool — is rayon `in_worker_cross` (helps in its own pool) and
boyko's unguarded joiner (drains the target pool's owner-only slot) (→J14).

---

### J4 — Core handoff on block

**Implemented by.** Tokio `block_in_place` ("We are taking the core from the context and sending it
to another thread"; a `Reset` guard tries to steal it back) `[S]`.

**Fit.** Not applicable — our joiner blocks on its own children, not an external event; and a fixed
pool has no spare thread to hand the core to. Recorded to mark the axis value.

---

### J5 — Compensation thread / oversubscription

**Implemented by.** Java FJP `tryCompensate` (conservative by design) `[S]`; UE 5.5 standby threads
(busy-wait replaced by oversubscription) `[D]`; oneTBB mandatory concurrency / `request_workers`
`[S]`; .NET hill climbing `[S]`.

**Documented rationale.** FJP, verbatim: "the vast majority of blockages are transient byproducts of
GC and other JVM or OS activities that are made worse by replacement by causing longer-term
oversubscription" `[S]`. Epic: busy-waiting "caused deadlocks regularly… picking unrelated tasks to
run that could themselves have a dependency on the task currently waiting"; "prone to stack overflow";
"special care was needed to exclude long running tasks" `[D]`.

**Evidence.** UE 5.5 documents the replacement and that standby threads are parked "as soon as the
oversubscription period is finished" `[D]` (API page rendered empty — U).

**Failure modes.** More runnable threads than cores — the outcome the criterion penalises.

**Negative results.** N7, N22.

**Fit.** Not applicable to a CPU-bound frame where nothing truly blocks. The transferable datum is
**Epic's list of what unrestricted helping did to a shipped engine** — it applies to J1 as we have it.
Two further shapes on axis 32 (how the block is detected): scheduler-observed (Linux cmwq, Windows
IOCP →J12) and time-gated (Chromium →J13). Note for axis 22: UE 5.5 and Chromium are *elastic* pools,
so "fixed (all game engines)" in the prior index was false by this file's own evidence.

---

### J6 — Fiber swap: the wait never blocks a thread

**Implemented by.** Naughty Dog (GDC 2015; slides not extractable — U) `[B]`; FiberTaskingLib
`task_scheduler.cpp` (`GetNextHiPriTask`, `SwitchToFreeFiber`, `AddReadyFiber`; last-successful-
steal hint "to improve cache locality"; deferred fiber cleanup) `[S]`; Our Machinery `[B]`.

**Evidence.** ND: 160 fibers, 3 priority queues, no stealing — secondary summaries only `[B]`. Our
Machinery's lock-free MPMC "failed miserably" → spin-lock `[B]` (N29).

**Failure modes.** Stacks (tens of MiB); resuming a fiber on another core moves its whole stack
working set (FTL's `pinToCurrentThread` exists for this); not expressible in safe Rust; TLS and
`!Send` state break on migration.

**Fit.** Not buildable safely. Marks the axis-5 value "wait costs a stack swap".

---

### J7 — Suspend the whole deque; a worker adopts it later

**Implemented by.** ProWS / Cilk-F (Singer, Xu, Lee PPoPP'19) `[P]`; X10 suspension scheduler
(abort-and-requeue) `[P]`.

**Evidence.** ProWS `O(T1/P + T∞·lg P)`; deviations imply `Ω(C·P·T∞ + C·t·T∞)` extra misses for
general futures `[P]`. X10 without stealing: fib(40) 700× slower `[P]`.

**Failure modes.** Deque adoption moves an entire working set to another core — the mechanism-1
objection in miniature (→L10).

**Fit.** M1: the only theory result that prices "adopt another lane's queue" in cache misses.

---

### J8 — Continuations instead of joins

**Implemented by.** Molecule Part 5 (`Job::continuations[15]`, job 64→128 B) `[B]`; Frostbite / Destiny
job graphs (slides, third-party transcription) `[B]`; UE Tasks prerequisites / nested tasks `[D]`.

**Documented rationale.** "as soon as this job finishes, run all its continuations immediately";
DICE: "Build big job graphs. Batch, batch, batch." `[B]`.

**Failure modes.** Fan-out capped by the inline array; the whole frame must be a DAG up front; work
discovered mid-job is the nested-spawn case again.

**Fit.** Sched: an engine-wide architectural commitment, not a pool feature. Our schedule already is
a DAG at the system level (axis 24); this variant would push it into system bodies.

---

### J9 — Task retraction

**Implemented by.** UE `FTaskBase::TryRetractAndExecute(FTimeout, RecursionDepth)` — "Tries to pull
out the task from the system and execute it"; recurses into prerequisites `[D]`.

**Fit.** Only helps if the task has not started; the recursion-depth parameter exists because
unbounded inline execution overflowed stacks. Not shortlisted.

---

### J10 — Wait-free join counter

**Implemented by.** Nowa IPDPS'21 (`Nτ = α − ω`, non-atomic fork counter, atomic decrement per
completion) `[P]`.

**Evidence.** vs Fibril 1.17× at 256 threads (0.99–1.64×) `[P]`; the "486.93× vs libgomp" figure is
implausible — U.

**Fit.** Applies only to continuation stealing. Our `ScopeShared::pending` is already a single atomic
counter with unpark-before-decrement (`scope.rs:158-159`) — the help-first equivalent.

---

### J11 — Dispatcher parks and polls a completion queue; never helps (boyko's frame path)

**Axis cell.** 5: parks (`park_timeout(100 µs)`), re-checks a completion queue, dispatches ready
systems; steals nothing · 21: `WORKER_ID_DISPATCHER`.

**Implemented by.** boyko `schedule.rs::executor_main_loop` (`:543-686`; park at `:683`; drop of the
frame scope at `thread_pool.rs:238` after `pending == 0`) `[L]`. No surveyed runtime has this exact
shape: rayon's cold caller blocks on a latch (J3); Bevy's executor is itself a task that may run on
the pool or be polled once inline (#11801) `[D]`; Godot's main thread is a full worker; Unity's main
thread completes jobs inline.

**Documented rationale.** SCH7: the apply window needs the dispatcher to hold `&mut world` exclusively
during `apply_window_drain` `[L]`.

**Evidence.** The frame scope's `Scope::drop` returns on its first `is_drained()` `[L]`; the
dispatcher is not a lane during a frame `[L]`; Bevy measured 10–70+ µs per OS wake in the analogous
loop and moved to polling the executor once inline before spawning it `[D]`.

**Failure modes.** The dispatcher is a parked core for the whole frame (one of the machine's cores
never runs a system); the 100 µs timeout is a latency floor on the apply window — **≥1 ms on the
Windows target**, because `park_timeout` rounds to whole milliseconds and nothing raises the timer
resolution (axis 36, rev. 2), so a released successor can wait a millisecond for the dispatcher to
notice it (→S10 removes that round trip); and the apply-window gate is the second cause of lane
idling (index A.5, →S6).

**Fit.** Established fact, not a candidate. It is why route (a) is bench-only and why B0 is cheap for
the engine today; and it is where mechanism 1's binding gate lives.
