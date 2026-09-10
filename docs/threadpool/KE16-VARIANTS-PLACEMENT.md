# KE16 — Variants, group P: where worker-spawned work lands, and who can reach it

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`.
Negative results `N…` and unverified claims `U…` are in `KE16-EVIDENCE.md`. **P17–P22** (added in
refutation round 1: sender-chosen placement into a stealable queue, rayon's `JobFifo` indirection,
thread-per-core, Julia partr, Jolt, ForkUnion) are in `KE16-VARIANTS-ADDENDA.md`.

Every subsection: axis cell · implemented by (verification kind) · documented rationale · evidence
(tagged) · failure modes · negative results · **cost on the SPAWN / STEAL / IDLE paths** · **fit**
for defect A, defect B, mechanism 1, mechanism 3 and cache locality under the throughput criterion.

Cost anchors used throughout, all `[B]` (Downs, "A Concurrency Cost Hierarchy", published harness,
named hardware, not peer-reviewed): uncontended atomic ~10 ns; contended atomic / cache-line transfer
40–400 ns (~110 ns at two threads); syscall ~1 µs; forced context switch ~10 µs. Bevy measured
10–70+ µs per OS wake inside its executor `[D]` (discussion #8304).

---

### P0 — Per-worker injector polled only by its owner (boyko today; the defect cell)

**Axis cell.** 1: per-worker `Injector`, private by construction · 2: none — no transfer path
exists · 5: owner drains it in `worker_main` and in `Scope::drop`; nobody else can · 11: **broken** ·
12: wake on every push · 25: the task, plus a wakeup to a worker that cannot reach it.

**Implemented by.** boyko_threadpool `worker.rs::push_task` (365-376), `::pop_local_injector`
(216-223), `::try_steal_random` (233-257) `[L]`. Tokio's `lifo_slot` is the same property at
capacity 1 (→P10) `[S]`. No work-stealing pool feeds a queue of this shape from its ordinary spawn
path; thread-per-core runtimes (Seastar, glommio) do it **by design**, with sender-routed SPSC inboxes
and nothing expected to cross (→P19); rayon's broadcast deque is an owner-only per-worker queue
*inside* a stealing pool, fed only by `inject_broadcast`, never by `spawn` (→P23, rev. 2) `[S]`.

**Documented rationale.** `worker.rs:351-364` and `thread_pool.rs:127-130`: "cache locality",
gated on pool identity so a cross-pool spawn cannot be stranded (FIX-2) — the first clause is correct
and implemented; the second ("siblings reach it via the local-injector steal in stage 1.5") describes
code that does not exist `[L]`.

**Evidence.**
- `injector_local` is indexed in exactly two places, both with the caller's own TLS id
  (`worker.rs:221`, `scope.rs:479`) `[L]` — **within one pool**. The joiner at `scope.rs:441-443`
  has no `active_pool_ptr == inner` check (only `worker.rs:39,369` and `thread_pool.rs:253,375`
  consult it `[L]`, rev. 2), so a pool-A worker joining a pool-B scope drains **pool B's slot of
  its own index** — the owner-only invariant fails in the cross-pool direction (→J14, App-6).
- On route (b) the owner is inside the body, not in `worker_main`; the only reader is the joiner,
  which moves the batch into an unregistered `scratch` and runs it serially (`scope.rs:477-484`,
  `:524-531`) `[L]`. Ceiling **1**; measured **1.01×** vs **7.69×** `[L]` OPEN-QUESTIONS.
- Additional: the push also performs `wake_rotor.fetch_add` before `idle.load` — one shared RMW
  per spawn regardless of idleness (`worker.rs:324-326`) `[L]` (→W0).
- Additional: `pop_any` probes the two shared injectors *before* the thread-private deque on every
  acquisition (`worker.rs:195-211`) `[L]`; rayon's `find_work` is the inverse with the comment
  "finish what we started before we take on something new" `[S]`.

**Failure modes.** Nested parallelism degenerates to serial on the spawning worker. The wake is pure
cost (Go's rejected approach #3, N3). The 50 µs `park_timeout` becomes load-bearing.

**Negative results.** N3, N12 (Tokio's capacity-1 twin is an acknowledged defect with an opt-out).

**Cost.** SPAWN: `Box` + `pending.fetch_add(AcqRel)` + `Injector::push` (two Acquire loads + a
SeqCst CAS on the tail index + `slot.state.fetch_or(WRITE, Release)` — two RMWs, **no epoch pin**:
in crossbeam-deque 0.8.7 `epoch::pin()` occurs only in `Worker::resize` and the three
`Stealer::steal*` functions, `deque.rs:301,650,764,1006`; nothing in `impl Injector` pins `[L]`) +
`wake_rotor.fetch_add` + `idle.load` — **1 alloc, 4 lock-prefixed RMWs, unconditional** `[L]`; of
the four, the two on `injector_local[wid]` act on a **single-writer** line under route (b) (the
owner is the only writer and nobody reads until the join — the ~10 ns class) and the two on
`pending` and `wake_rotor` act on **multi-writer** lines (the transfer class) — axis 38, rev. 2.
STEAL: never happens for this queue. IDLE: the woken worker scans W−1 deques, finds nothing, parks
again. COMPLETE: `waker.unpark()` (an atomic swap on the joiner's parker line + `futex_wake` when the
joiner is parked) then `pending.fetch_sub(AcqRel)` — two shared RMWs per task, unconditional by
design (→W17).

**Fit.** A: this *is* the defect. B: subsumes it on route (b). M3: the registry is read after the
placement is committed — the pusher learns who is idle only after it has put the task where the idle
worker cannot reach it. Loc: perfect locality of work that never leaves one core, which is the
degenerate case of the objective.

---

### P1 — Spawn to the spawner's own registered deque (rayon shape)

**Axis cell.** 1: own deque, LIFO end (rayon, Cilk, FJP) or FIFO end (boyko's deques) · 2: receiver ·
3: one (rayon) or half (crossbeam users) · 4: random + rotation · 5: helps · 7: LIFO owner / FIFO
thief · 11: closed by construction · 16: caller-frame job possible (rayon `join`).

**Implemented by.** rayon-core `registry.rs::inject_or_push` — verbatim `if !worker_thread.is_null()
&& (*worker_thread).registry().id() == self.id() { (*worker_thread).push(job_ref); } else {
self.inject(job_ref); }`; `scope/mod.rs::Scope::spawn` calls it `[S]`. LLVM libomp
`kmp_tasking.cpp::__kmp_push_task` ("Find tasking deque specific to encountering thread") `[S]`.
Taskflow `executor.hpp::_schedule` (`worker._wsq.try_push`, spill on failure) `[S]`. .NET
`ThreadPoolWorkQueue.cs::WorkStealingQueue.LocalPush` `[S]`. Java FJP (internal fork → own WorkQueue)
`[S]`. Kotlin `CoroutineScheduler.kt` (worker dispatch → local queue head) `[S]`. Molecule Job System
2.0 `[B]`. enkiTS `TaskScheduler.cpp::AddTaskSetToPipeInt` (`SplitAndAddTask(threadNum_, …)`) `[S]`.
Eigen `NonBlockingThreadPool.h::ScheduleWithHint` ("Worker thread of this pool, push onto the thread's
queue", `PushFront`) `[S]`; UE 5 `LocalQueue.h` is documented as Eigen-derived `[D]`. Cilk-5 `[P]`.

**Documented rationale.** oneTBB "How Task Scheduler Works": run the youngest local task
(depth-first, cache-hot, linear space), steal the oldest (breadth-first, converts potential
parallelism into actual) `[D]` (wording near-verbatim via search index, page returned 403 — U). rayon
FAQ: `join(a,b)` "places b into W's queue while W starts executing a" `[D]`. Blumofe & Leiserson:
work stealing is provably good — expected steal attempts `O(P·T∞)` (**Lemma 12**), expected running
time `T1/P + O(T∞)` (**Theorem 13**) — both proved for the WORK-FIRST rule ("If the thread Ga spawns
a child Gb, then Ga is placed on the bottom of the ready deque, and the processor commences work on
Gb") over FULLY STRICT computations ("all join edges from a thread go to the thread's parent") `[P]`
(re-read via proxy in round 1; the first synthesis cited Theorem 13 for the steal bound). **Arora,
Blumofe, Plaxton TOCS'01** removes both of those restrictions and is the theorem that covers our
shape: "we consider arbitrary multithreaded computations as opposed to the special case of 'fully
strict' computations"; "Of the two ready threads (the assigned thread and the newly ready thread),
the process pushes one onto the bottom of its deque and continues executing the other … The bounds
proven in this paper hold for either choice."; Theorem 9: "The expected execution time is O(W/p +
D)" with dedicated processors `[P]` (re-read via proxy, rev. 2; axis 37).

**Evidence.**
- rayon's push body: `let queue_was_empty = self.worker.is_empty(); self.worker.push(job);
  self.registry.sleep.new_internal_jobs(1, queue_was_empty);` `[S]` — a Chase-Lev push and one
  load in the common case.
- Chase-Lev owner push is a plain store + store to bottom, no CAS; CAS only in steal and the emptying
  pop `[P]` (Chase & Lev SPAA'05); ABP Figure 5 likewise `[P]`.
- Every design in the theory corpus that puts spawned work in a per-worker structure also gives it a
  reader — the deque itself, a marked duplicate, a lower-priority poll, or a polling obligation
  (lens 1 notes) `[I]`.
- Guo et al. IPDPS'09: for one worker distributing N flat tasks, help-first lets P−1 thieves steal
  concurrently; FJ(1024) fixed work-first 4.6× slower than help-first `[P]` — boyko's `par_iter` is
  the flat case and is already help-first in shape; only the queue is wrong.

**Failure modes.** Owner's deque is now contended by thieves (one CAS per successful steal, span-
proportional). Deque is unbounded (rayon, libomp) — burst growth. The owner's LIFO end is the hot
end; a thief takes the FIFO end (fine). Requires a way for `push_task` to reach the worker's
`Worker<TaskHandle>`, which lives on `worker_main`'s stack — a TLS pointer, as rayon's
`WorkerThread` TLS does; `scope.rs:17-21` currently states the deque is "not accessible here".

**Negative results.** N17 (rayon constrained the push behind an identity check — the same check we
already have; the opposite resolution). N40 (a *fixed* policy in either direction fails somewhere;
we are help-first and flat, which is help-first's good case).

**Cost.** SPAWN: Chase-Lev push (Acquire load of front + Release store of back, **no RMW**), plus
whatever wake gate is chosen (→W0–W2); the `Box` and the `pending` RMW remain unless W14 is taken.
STEAL: one CAS on the victim's front per batch (≤32) + epoch pin; residue lands in the thief's
registered deque (re-stealable). IDLE: unchanged (the scan set is unchanged: `inner.stealers`).

**Fit.** A: **the cheapest fix in the design space by the cost model** (lens 1) and what rayon does.
B: **promotes B to route (b)** — the joiner's `try_steal_any` includes `stealers[wid]` with no
self-skip (`scope.rs:534-541` `[L]`), so the joining worker will steal half its own wave into the
unregistered `scratch` (A.4). M1: n/a. M3: compatible with any wake gate. Loc: restores the free
LIFO/FIFO discipline if the deques are switched to `new_lifo()` (→L1); the spawner's own chunks stay
on its core until stolen. Throughput prognosis (rev. 2, now a theorem plus one measurement):
round 1 said the Blumofe–Leiserson bound "does not cover this case" and fell back to Guo et al.'s
flat-loop argument — wrong theorem. **ABP Theorem 9** covers help-first over an arbitrary DAG
("either choice"), with the serial spawn chain inside the critical path D (at N = 64W = 1024 chunks
and ~120 ns per spawn, `par_iter.rs:68` `[L]`, `D ≥ 123 µs`); the steal-request constant on a DAG
of depth D is Tchiboukdjian et al.'s `5.5·D` (vs ABP's `32·D`) `[P]`; and once the wave is
published as a pile (App-4), Theorem 3's `W/m + 3.24·(log2 W + 1/(2 ln 2)) + 1` says W workers
fill in O(log2 W) steal rounds `[P]` (axis 37). So the *direction* at every grid cell is proved;
what the grid measures is the constant — at 1 µs × 64W the contended-deque CAS per steal
(a transfer per `≤32` tasks, axis 38) against a 1 µs body.

---

### P2 — Per-worker injector kept, made stealable (in the scan set)

**Axis cell.** 1: per-worker `Injector`, now polled by siblings on the steal path · 2: receiver ·
3: ≤33 per `Injector::steal_batch_and_pop` · 4: random + rotation over injectors and deques · 11:
closed by scan-set membership · 12/13: unchanged.

**Implemented by.** **Nobody, exactly.** Nearest: Java FJP interleaves external submission queues
(even indices) with worker queues (odd indices) in ONE array so one scan samples both — "cheaper
than alternatives" despite "many wasted probes (null slots)" `[S]`. Acar/Blelloch/Blumofe's affinity
mailbox is polled by its owner first and duplicated in the deque with a test-and-set mark `[P]`; SLAW
polls the place mailbox at *lower* priority than deques `[P]`. Lens 1's empty cell (1) and lens 5's
open cell name this design.

**Documented rationale.** None published for this exact shape. The intent it preserves is the one
`thread_pool.rs:128` states: locality of inner-spawn work with its spawner, with siblings as a
fallback.

**Evidence.**
- FJP's justification for scanning both kinds in one array: "both kinds of queues should be sampled
  with approximately the same probability, which is simpler if they are all in the same array" `[S]`.
- crossbeam `Injector::steal_batch_and_pop` limit is `MAX_BATCH + 1 = 33`; a straddling grab can
  take 33 from a >63-slot wave `[L]` (`deque.rs:1760`, 0.8.7).
- An EMPTY `Injector` probe is two Acquire loads (head index, head block) + `atomic::fence(SeqCst)`
  (`mfence` on x86) + a Relaxed load of the tail — **no epoch pin** (`deque.rs:1795-1830`, 0.8.7
  `[L]`; the prior text's "each Injector probe pins the epoch" was wrong — U44 closed). Only
  `Stealer` probes pin (`deque.rs:650`). So the idle-path cost is W−1 extra full fences per round.
- A second way to close the same queue with **zero** extra probes is rayon's placeholder
  indirection (→P18), at the price of P1's TLS deque pointer.

**Failure modes.** The idle scan grows from W−1 to 2(W−1) shared probes per round, inside the spin
budget (→W11) — O(W²) coherence traffic at wave boundaries, the shape of Go #28808 (N5). The joiner
still drains its own injector into `scratch` first (`scope.rs:479`), so B is promoted to route (b)
exactly as under P1. `Injector` is MPMC: the owner's own drain now contends with thieves on the
injector's head, where under P1 the owner's pop is uncontended.

**Negative results.** N5 (fruitless scans dominate at high W).

**Cost.** SPAWN: unchanged from P0 (Injector push CAS + slot `fetch_or`, no pin) — **no improvement
on the spawn path**; the line stays single-writer only until a sibling's probe pulls it (axis 38).
STEAL: +1 `Injector` probe per sibling visited. IDLE: doubles the per-round probe count.

**Fit.** A: fixes reachability with the smallest diff and makes the false comment true. B: promoted to
route (b). M3: unchanged. Loc: keeps inner-spawn work near its spawner until a thief arrives.
Throughput prognosis: beats P0 everywhere; loses to P1 on the spawn path (two RMWs vs a plain
store) and on the idle path (double probes); the gap is largest at 1 µs bodies × 64W tasks.

---

### P3 — Every spawn to the global injector

**Axis cell.** 1: global injector · 2: receiver · 3: ≤33 per batch · 4: n/a (one source) · 7: none
· 11: closed trivially · 12: notify-one on push.

**Implemented by.** async-executor `lib.rs` (the schedule closure pushes to `state.queue`
unconditionally; the local-queue push is a TODO) — the substrate under **Bevy** `[S]`. rayon
`Registry::inject` for non-members `[S]`. oneTBB `task_arena::enqueue` → `my_fifo_task_stream`,
taken only "at the outermost dispatch level without isolation" `[S]`. Bitsquid task manager (one
priority heap under a critical section) `[B]`. Our Machinery fiber system (one global job queue)
`[B]`. boyko's own cross-pool / dispatcher arm (`worker.rs:373`) `[L]`.

**Documented rationale.** async-executor: none — the local path is an unimplemented TODO ("If
possible, push into the current local queue and notify the ticker") `[S]`. TBB: the enqueue stream
is "starvation-resistant" and therefore globally reachable, chosen when fairness beats locality `[S]`.
Bitsquid: "at our current level of task granularity… the global task queue should not be a
bottleneck" `[B]`.

**Evidence.**
- crossbeam `Injector::push`: Acquire loads of the tail index and block + SeqCst
  `compare_exchange_weak` on the shared tail index + `slot.state.fetch_or(WRITE, Release)`; 63 slots
  per block; no epoch pin `[L]` (0.8.7).
- Bevy's measured complaints with this substrate: nested spawn hits a shared MPMC queue + notify on
  every task; "wakes new threads slower as async executor limits to waking one thread at a time";
  missing work-first split (#10064) `[D]`; 10–15 % CPU on a near-idle app (#4718) `[D]`.
- Vyukov bounded MPMC: exactly 1 CAS per op on a shared counter; the only published number is 75
  cycles/op on a dual-core `[D]` (the scaling curve in circulation is unsourced — U).
- LCRQ/MS-queue: MS throughput reported to peak at two threads `[P*]` — unverified.

**Failure modes.** Every spawn is a contended CAS on one shared line; zero producer-consumer
affinity; the queue is the contention point at 16 workers with µs chunks.

**Negative results.** N1 (Go: centralised state inhibits scalability), N4 (Go pre-1.1 global
runqueue), N10 (Tokio: single MPMC rejected), N24 (Bitsquid kept it — the contrary datum, at coarse
granularity).

**Cost.** SPAWN: `Injector::push` CAS + slot `fetch_or` (no epoch pin; the same instructions as
P0's push, on a **multi-writer** line instead of P0's single-writer one — the class change axis 38
records, rev. 2) + the wake. STEAL: `steal_batch_and_pop` into the
thief's registered deque (≤33). IDLE: unchanged (the global injector is already stage 3).

**Fit.** A: fixed by one line (`worker.rs:369` → always global). B: promoted to route (b) via
`scope.rs:488`. M3: unchanged. Loc: none. **Role: the control** — the reachability floor against
which P1/P2's locality claim is measured on the grid. Throughput prognosis: wins over P0 at every body
size; loses to P1 at 1–10 µs bodies × 64W by the contended CAS; indistinguishable at 1 ms.

---

### P4 — Bounded local queue, overflow half to the global injector

**Axis cell.** 1: own bounded SPMC ring (256) + LIFO slot; overflow to global · 2: receiver, plus
an automatic sender-side spill · 3: half (steal) and half (spill, 128 of 256) · 11: closed by overflow
· 18: fixed 256, spill half.

**Implemented by.** Tokio `multi_thread/queue.rs` (`LOCAL_QUEUE_CAPACITY = 256`,
`push_back_or_overflow`, `steal_into2` with `n - n/2`) `[S]`; Go `proc.go` (`runq[256]`,
`runqputslow` moves half + the new g under `sched.lock`, `runqgrab` `n = n - n/2`) `[S]`
(go1.4/go1.5.4 read verbatim; master partially — U).

**Documented rationale.** Tokio 2019 blog `[B]`/`[D]`: crossbeam's growable deque "included expensive
memory reclamation overhead" (epoch RMW in the hot path); a fixed array with a packed head closes ABA
without reclamation. Which half is spilled: "when we take tasks out of the injection queue, we
always place them in the first half… if a task is in the second half… this task is not a task we
just got" `[S]`.

**Evidence.**
- Owner push: Acquire load of head + Release store of tail, no RMW; thief: one AcqRel CAS on the
  packed head `[S]`.
- Go's global-queue pop is batched too: `n = runqsize/gomaxprocs + 1`, capped at half the ring `[S]`.
- Tokio's rewrite: chained_spawn 2,019,796 → 168,854 ns (11.9×); hyper hello +34 % — **vendor-run**
  `[B]`.
- The spill costs one CAS + a bulk move of 128 tasks, amortised over 128 pushes at steady state `[S]`.

**Failure modes.** The spill is a latency spike on the producer at its busiest moment. Choosing the
wrong half makes tasks ping-pong. A bounded queue must define its overflow route; routing overflow
into a *private* queue would be P0 again.

**Negative results.** N8, N9, N11 (Tokio's three abandonments that led here).

**Cost.** SPAWN: plain stores in the common case; 1 CAS + memmove per 128. STEAL: one CAS per
half-batch. IDLE: unchanged.

**Fit.** A: closed. B: promoted to route (b). Loc: the LIFO slot keeps the just-spawned task hot
(→P10's caveat). Prognosis: the shape Tokio and Go converged on; a larger rewrite than P1 for the
same spawn-path cost; **fallback if P1's TLS deque pointer is judged unsound** under Tree Borrows.

---

### P5 — Affinity mailbox with a dual-residency proxy

**Axis cell.** 1: BOTH the spawner's deque AND a proxy in a named worker's inbox · 2: sender
publishes a hint, receiver claims, any thief may still take the original · 4: affinity id first, then
random · 7: this *is* the locality mechanism · 11: closed because the real task never leaves the
origin deque · 25: a proxy.

**Implemented by.** oneTBB `src/tbb/mailbox.h` — `task_proxy` with `task_and_tag` carrying
`pool_bit`/`mailbox_bit`; `mail_outbox::push` documented wait-free (one atomic exchange of the tail)
and "Padded to occupy a cache line"; `extract_task` CASes to claim, loser frees the proxy
(`cleaner_bit`) `[S]`. `task_dispatcher.h::receive_or_steal_task` probes the inbox first `[S]`.
Acar/Blelloch/Blumofe SPAA'00/TOCS'02 (mailbox → deque → steal; test-and-set mark) `[P]`. SLAW
place mailboxes (polled *after* deques) `[P]`. NUMA-WS single-entry remote mailboxes, pushed lazily
only on the span term `[P]`.

**Documented rationale.** ABB: a created thread is pushed "onto both the deque… and also onto the
tail of the mailbox of the process that the thread has affinity for"; double execution prevented by
an atomic mark `[P]`. TBB `note_affinity`: "Invoked by scheduler to notify task that it ran on
unexpected thread" — affinity is advisory `[D]`. TBB user guide: `affinity_partitioner` "should be
considered a tool, not a cure-all"; pays only when the loop does few ops per access, the data fits in
cache, and the same loop re-runs over the same data `[D]`.

**Evidence.**
- ABB measured up to 80 % over plain work stealing on heat/relax, "bad updates" ~80 % → ~10 %; the
  authors resorted to timestamping stale duplicates because direct synchronisation was too expensive
  `[P]`.
- ABB lower bound: uniprocessor locality does not transfer — a family with 3C misses on one processor
  and Θ(n) on two `[P]`; upper bound `M_P ≤ M_1 + O(⌈m/s⌉·C·P·T∞)` `[P]`.
- NUMA-WS: pushes only on sync completion, child return, successful steal — never on the work path
  `[P]`.
- The public `tbb::task` affinity API was removed in oneTBB 2021 as "complex and hence error-prone";
  the mailbox survives internally `[D]` (N20).

**Failure modes.** Two residencies → two frees and a CAS per extraction. A mail aimed at a busy
worker is dead weight until a thief takes the pool copy. Writing another core's line on the spawn
path is the most expensive thing a spawn path can do. **A naive imitation that moves the task into
the mailbox with no deque copy is P0.**

**Negative results.** N20; and the ABB caveat that initial placement "does not perform consistently
better under multiprogrammed work loads" `[P]`.

**Cost.** SPAWN: deque push + proxy allocation + a wait-free write to a *foreign* cache line + the
wake. STEAL: one CAS to claim the proxy. IDLE: one extra inbox probe.

**Fit.** A: closed (the deque copy). M3: this is the only *hybrid* that gets push-to-a-named-worker
without Go's/FJP's objection, because the task never leaves the stealable pool. Loc: the only
mechanism in the survey with a *measured* locality gain `[P]`, on a 14-processor 2002 machine.
Prognosis: not worth building on a single-socket target with no residency instrument (E6).

---

### P6 — Direct handoff / push-to-idle / work-dealing (the owner's mechanism 3 as placement)

**Axis cell.** 1: delivered to a chosen idle worker · 2: sender · 4: the idle registry chooses ·
6: registry consulted on every spawn · 7: destroyed by construction · 25: the task.

**Implemented by.** **No production runtime in the five lenses implements the NON-stealable form.**
The *stealable* form — sender-chosen placement into a queue thieves can still reach — is shipped
(FJP `externalPush`, HPX schedule hints, BEAM last-scheduler placement, Linux CFS
`select_idle_sibling`, PhysX first-accepting queue) and is catalogued as **P17**; the OS scheduler
under every surveyed engine does idle-keyed placement with a steal fallback `[R:S]`. The rejected
form: considered and rejected by Go
(`proc.go`, rejected approach #2) `[S]` and Java FJP ("work dealing") `[S]`. The academic
sender-initiated forms: Acar/Charguéraud/Rainey PPoPP'13 sender-initiated deals into
self-advertised idle cells, with Poisson-distributed deal attempts `[P]`; ADM (hardware messages;
slides only) `[B]`; XGOMP NA-RP remote push `[P]` (arXiv:2502.05293, HTML read); Charm++ seed
balancers `[D]`; id Tech 5's push-to-all-N is the degenerate form (→P12).

**Documented rationale.** Acar et al.: "Busy processors proactively deliver work to idle processors.
Idle processors mark themselves as available in shared cells." The Poisson delay exists to preserve
fairness `[P]`. Eager/Lazowska/Zahorjan 1986: sender-initiated better at light load, receiver at
heavy; "uniformly better" only when receiver-initiated transfers are much more expensive because a
*running* task must migrate `[P]` — a condition that does not hold in shared memory (PDF is a scan;
partially verified — U).

**Evidence.**
- **Go, verbatim:** "Direct goroutine handoff… would lead to thread state thrashing, as the thread
  that readied the goroutine can be out of work the very next moment, we will need to park it. Also,
  it would destroy locality of computation as we want to preserve dependent goroutines on the same
  thread; and introduce additional latency." `[S]`
- **FJP, verbatim:** "Work-stealing based on randomized scans generally leads to better throughput
  than 'work dealing' in which producers assign tasks to idle threads, in part because threads that
  have finished other tasks before the signalled thread wakes up can take the task instead." `[S]`
- **XGOMP NA-RP, measured:** pushing away "incurring a cost of more than 100 ns for each task that
  could otherwise be self-executed within nanoseconds" on Fib (10–80-cycle tasks); ~4× gain only for
  tasks > 10⁴ cycles; the mechanism is gated on granularity `[P]`.
- Acar et al.'s theorem gives sender-initiated a better constant than receiver-initiated (c = 1.0 vs
  ≈1.58) in the same bound — the strongest theoretical support for the proposal — but both private-
  deque variants land within a few percent of Cilk Plus and *lose* on matmul (−18 % vs concurrent
  deques) `[P]`.
- A-STEAL Theorem 12 is parametric: `W ≤ ((1+ρ−δ)/δ + (1+ρ)²/(δ(Lδ−1−ρ)))·T1`; "≈2·T1" is a
  Section-5 instantiation ("generally less than 2T1"), not the theorem (re-read via proxy; N58)
  `[P]`; "no adaptive scheduling algorithm can effectively utilize the available processors" when
  instantaneous parallelism is low `[P]`. BWS: CG at 32
  workers +144 % slower alongside MM; halving CG's workers improved both `[P]`.

**Failure modes.** Thread-state thrashing; locality destruction; latency; a commitment made on stale
information (the target may find other work first); a shared-line read on the SPAWN path
unconditionally — the acceptance criterion's worst case.

**Negative results.** N2, N6, N37, N47, N48.

**Cost.** SPAWN: registry read + claim RMW + remote enqueue (a foreign cache line) + unpark syscall,
**on every spawn**, whether or not anyone was idle. STEAL: none. IDLE: the target parks/unparks per
handoff.

**Fit.** M3: this is the proposal. A: it would close reachability by assignment rather than stealing.
Loc: the record says it destroys it. **Distinction that matters:** every runtime surveyed *has* an
idle registry and uses it to pick **which thread to wake** (→W6, present in boyko). What is rejected
is using it to decide **where the work goes** *such that only that worker can reach it*. The
buildable relative that keeps the destination stealable is **P17** (candidate M3-s). Prognosis for
this non-stealable form on our grid: a loss at 1–10 µs bodies (NA-RP's number), a wash above.
Recorded in E17; not shortlisted.

---

### P7 — Private deques + explicit steal requests over channels

**Axis cell.** 1: fully private per-worker deque · 2: receiver posts a request; victim answers at a
polling point (or forwards) · 3: one, or half, per request (adaptive) · 15: fully private · 10: zero
synchronisation on the owner's deque; CAS only on the request cells.

**Implemented by.** Acar/Charguéraud/Rainey PPoPP'13 receiver-initiated variant `[P]`. HPX
`local_workrequesting_scheduler.hpp` (`workrequesting_steal_request { thief, task channel, victim
mask, stealhalf_ }`; MPSC request channel + SPSC task channel per worker; unfulfilled requests
forwarded) `[S]`, shipped as `local-workrequesting-{fifo,lifo,mc}` `[D]`. Weave (Nim): "idle threads
send steal requests instead of actively stealing", `WV_StealAdaptativeInterval=25` `[D]`. Prell's
thesis (could not open — U).

**Documented rationale.** Acar et al.: concurrent deques "require expensive memory-fences, which can
degrade performance significantly" and are hard to extend for irregular computations `[P]`. HPX: "By
owning two channels, workers are able to receive steal requests and tasks independently of other
workers, which in turn enables efficient channel implementations based on single-consumer queues"
`[S]`. Weave: portability to non-cache-coherent targets `[D]`.

**Evidence.**
- Acar et al. bound with an explicit polling-delay parameter δ; measured vs concurrent deques on 30
  cores: matmul −18 %, cilksort −2 %, fib −2 %, matching +9 %, sample-sort −6 % `[P]`; coarse tasks
  need interrupt-based polling at ≈3.7 % `[P]`.
- Prell: "only slightly slower" than Chase-Lev `[P*]` — unverified.
- Weave: "3×–10× less overhead than TBB/OpenMP", tasks from ~2000 cycles — author's README `[B]`.

**Failure modes.** A victim that does not poll cannot be robbed — **structurally the same failure as
defect A, except designed in and analysed**. Transfer latency is a round trip. The victim pays a
shared-line load on its working path. matmul lost 18 %.

**Negative results.** N46 (private deques presented as competitive, not superior).

**Cost.** SPAWN: pure thread-local (no atomics). STEAL: request post (CAS) + victim's poll + reply.
IDLE: a thief with an outstanding request parks until served.

**Fit.** A: closed (the polling obligation is designed in). M3: the closest *principled* relative of
the owner's registry idea in the receiver direction. Prognosis: not for a frame-locked pool (E13).

---

### P8 — Split deque: private tail, public head, lazy release

**Axis cell.** 1: own deque, PRIVATE region by default · 2: hybrid — thief requests, owner exposes ·
3: whatever the owner releases (often half) · 15: split with a movable point · 10: owner push/pop
fence-free on the private region; one fence only when shrinking the shared region; thief one CAS.

**Implemented by.** Scioto `SpDeque` (Dinan et al. SC'09) `[P]`; Lace (van Dijk & van de Pol
Euro-Par'14) `[P]`; LCWS (Custódio, Paulino, Rito SPAA'23; arXiv:1810.10615) `[P]`.

**Documented rationale.** Dinan: the local portion is accessed without locking, the shared portion by
anyone `[P]`. LCWS: "Busy processors only expose work to be stolen after being targeted by one or
more steal attempts" — synchronisation proportional to SPAN, not WORK `[P]`.

**Evidence.**
- LCWS theory: expected sync cost `O((C_CAS + C_MFence)·P·T∞)` vs standard `O(W + S·P)`; running
  time still `O(W/P + S)` `[P]`.
- Lace vs Wool at 48 cores: fib 50 4.13 s (34.9×) vs 4.38 s; uts T2L 1.81 s (47.4×) vs 2.00 s; queens
  15 12.63 s vs 11.23 s (Wool wins) `[P]`.
- LCWS end-to-end: average +3.8 % / +1 % / +1.3 % on three machines; worst −0.8 % to **−102 %** `[P]`.
- Dinan at 8192 cores: UTS 99 %, BPC 97 % efficiency `[P]`.

**Failure modes.** The victim must reach a release point; a long sequential run starves thieves
(LCWS's criticism of Lace: "does not handle work exposure requests in constant time"). Worst-case
regressions to −102 %. A second index and protocol on an already subtle deque.

**Negative results.** N42 (the fence this exists to remove), N46 (LCWS's own mixed table).

**Cost.** SPAWN: private push, zero atomics. STEAL: request + release + one CAS. IDLE: thief spins on
the request.

**Fit.** A: closed by design. B: the release-half is the amortisation argument in its published form
— with the batch staying **stealable**. Prognosis: not worth building over crossbeam for µs chunks on
16 cores; the measured gains are single-digit percent at best.

---

### P9 — Lifelines: bounded random stealing, then a fixed low-degree graph with deferred, sender-fulfilled requests

**Axis cell.** 2: receiver for w attempts, then deferred sender · 4: random, then hypercube buddies
· 6: the failed thief goes **dormant** and is reactivated by a push · 13: the registry is
**sharded onto victims** · 20: distributed termination.

**Implemented by.** X10 GLB (Saraswat et al. PPoPP'11 — not opened; GLB library arXiv:1312.5691 —
read) `[P]`/`[P*]`.

**Documented rationale.** The hypercube satisfies three named requirements: fully connected (work
flows anywhere), low diameter (latency), low degree ("the number of buddies potentially sending work
to a dead vertex is low"). A lifeline victim with nothing "will still remember the request and try
to satisfy the request when it gets work from others" `[P]`.

**Evidence.** UTS linear speedup to 16,384 cores on Blue Gene/Q; efficiency 0.6 beyond 4,096 on the
K computer; BC std dev of work 4.027 → 1.141 `[P]`. 87 % efficiency on 2048 nodes `[P*]`.

**Failure modes.** Designed for distributed memory where a failed probe is a network round trip; on
16 shared-memory cores a failed steal is one cache miss. Latency O(diameter). Per-creditor state on
the working thread. `w` and `z` are tuning knobs with no closed form.

**Negative results.** none recorded by the authors; the design is a scale-out answer.

**Cost.** SPAWN: pays only when a lifeline is outstanding. STEAL: bounded (w). IDLE: **bounded** —
after w failures the thief stops burning bandwidth.

**Fit.** M3: the cleanest *published* "idle-worker registry a spawner pushes into" — and it is
distributed per victim, bounded by graph degree, and two-phase. The transferable idea for us is
**bounding the idle scan** (→W11), not the lifeline graph.

---

### P10 — Non-stealable single-slot LIFO cache ahead of the deque (P0 at capacity 1)

**Axis cell.** 1: a private one-task slot · 2: none for the slot · 7: maximal for producer-consumer
pairs · 10: spawn = one thread-local store, zero atomics for the slot.

**Implemented by.** Tokio `worker.rs::Core::next_task` / `schedule_local`, `lifo_slot`,
`MAX_LIFO_POLLS_PER_TICK = 3` `[S]`; Go `runqput(next=true)` → `p.runnext`, stolen only late with a
`usleep(3)`/`usleep(100)` delay (go1.5.4 verbatim; modern constant not read — U) `[S]`.

**Documented rationale.** Tokio: keeps the message hot between send and poll; "Running the LIFO slot
a handful of times seems sufficient to benefit from locality. More than 3 times probably is
over-weighting." `[S]` Go: "Sleep to ensure that _p_ isn't about to run the g we are about to
steal." `[S]`

**Evidence.**
- Tokio issue #4941 "make the LIFO slot… stealable"; PR #4936 `Builder::disable_lifo_slot` as "a
  stop-gap"; docs: "the LIFO slot cannot be stolen by other worker threads, which can result in
  lower total throughput when tasks tend to have longer poll times" `[D]`.
- Starvation in ping-pong workloads (#4323) — the reason for the 3-poll cap `[D]`.

**Failure modes.** Exactly P0's, bounded to one task. Tokio files it as a defect and ships an opt-out.

**Negative results.** N12.

**Cost.** SPAWN: one store. STEAL: impossible for the slot. IDLE: n/a.

**Fit.** A: the most-deployed Rust runtime has our defect at capacity 1 and calls it a defect. Loc:
the *only* reason anyone accepts it is message-passing locality, which a flat `par_iter` wave does
not have. Kotlin's time window (→P11) is the unbuilt answer to #4941.

---

### P11 — Time-window stealability

**Axis cell.** 1: own local FIFO with a timestamp per task · 2: receiver, gated · 7: the owner gets a
bounded head start · 10: one monotonic clock read per steal.

**Implemented by.** Kotlin `CoroutineScheduler.kt` — `WORK_STEALING_TIME_RESOLUTION_NS`: "a task from
FIFO buffer may be stolen only if it is stale enough" `[S]` (constant's value not read — U).

**Documented rationale.** A softer, time-bounded LIFO slot: freshly produced work is temporarily, not
permanently, private, capping the damage `[S]`.

**Evidence.** The window is explicit and finite where Tokio's is a permanent slot `[S]`.

**Failure modes.** Needs a cheap monotonic clock on the steal path; a long window reproduces the LIFO
starvation; for µs ECS chunks the window would be sub-µs.

**Cost.** SPAWN: one timestamp store. STEAL: one clock read + compare.

**Fit.** Loc: the only design that makes work *temporarily* private. Prognosis: not for µs chunks
(E12).

---

### P12 — Sender pushes to every worker's private list; per-list atomic index; no stealing

**Axis cell.** 1: every selected worker's inbox · 2: sender + explicit `SignalWork()` · 3: a whole
job LIST to N threads; one job per atomic increment within a list · 5: `Wait()` yield-spins · 13:
none.

**Implemented by.** Doom 3 BFG / id Tech 5 `neo/idlib/ParallelJobList.cpp::Submit` (`threads[i]
.AddJobList()` for i in 0..parallelism), `idJobThread::Run`, `::Wait` `[S]`.

**Documented rationale.** Per-job claim is `state.nextJobIndex = currentJob.Increment() - 1` — one
atomic per list, no queue; the sync-point skip loop is "an optimization to minimize the time spent
in the fetchLock section" `[S]`. Parallelism is a submit-time parameter (`MAX_JOB_THREADS = 32`)
`[S]`.

**Evidence.** No work stealing exists; `Wait()` is `while(signalJobCount > 0) Sys_Yield()` `[S]`.

**Failure modes.** Imbalance across lists is unfixable; the waiter yields, contributing nothing; every
submit costs N writes + N signals whether or not threads are idle; no nested spawn at all.

**Cost.** SPAWN: N inbox writes + N signals. STEAL: none. IDLE: worker blocks on its own signal.

**Fit.** M3: the degenerate "push to all" form of push-to-idle, shipped in a 2011 engine with a
statically built frame. Prognosis: not applicable to dynamic nested waves.

---

### P13 — Static partition, no transfer

**Axis cell.** 1: fixed partition at dispatch · 2: none · 5: idle until the barrier · 6: barrier ·
7: perfect and stable · 10: nothing shared but the barrier counter.

**Implemented by.** flecs `src/addons/pipeline/worker.c` (`flecs_run_pipeline_ops` with
`stage_index/stage_count`; `flecs_sync_worker`/`flecs_signal_workers` barrier) `[S]`, docs: "the
same entity is always processed by the same thread, until the next sync point" `[D]`. OpenMP
`schedule(static)` `[D]` (LLNL tutorial; spec page 403). HPX `static`/`static-priority` ("There is no
thread stealing in this policy") and pool isolation `[D]`. The baseline in every locality paper `[P]`.

**Documented rationale.** flecs: "guarantees ideal performance in most cases without requiring
synchronization between frames" `[D]`; systems are "too small & add too much scheduling overhead in
large applications" to schedule individually `[D]` (#1590).

**Evidence.**
- flecs docs contain no work stealing and no imbalance recovery `[D]`. Example 1000 entities / 4
  threads → four contiguous slices `[D]`.
- ABB: locality-guided WS "matches the performance of static-partitioning under traditional work
  loads but improves… up to 50 % over static partitioning under multiprogrammed work loads" `[P]`.
- Su et al. ASPLOS'24: "OpenMP static scheduling remains superior for non-irregular cases" `[P]`.

**Failure modes.** Makespan = slowest slice; equal entities ≠ equal work under fragmentation; no
recovery from a lane handed more systems by the first-level scheduler — the owner's observation is
this failure at the system level; no nested parallelism.

**Negative results.** N25 (flecs's contrary position on mechanism 1).

**Cost.** SPAWN: none. STEAL: none. IDLE: barrier wait.

**Fit.** M1/Loc: the strongest locality invariant in the field, and the only ECS that ships it;
`par_iter`'s equal-row chunks are a static partition with a steal fallback that today never fires.
Prognosis: a measured baseline for the locality instrument (E9), not a candidate.

---

### P14 — Single central queue (lock-based, and lock-free MPMC)

**Axis cell.** 1: central · 2: receiver · 4: n/a · 5: waiter helps from the same queue (Godot) or
blocks (Folly) · 11: trivially closed · 10: every op is an RMW on one shared line.

**Implemented by.** Godot 4 `worker_thread_pool.cpp` (`task_mutex`, `task_queue` +
`low_priority_task_queue`, `_wait_collaboratively`) `[S]`; Folly `CPUThreadPoolExecutor.cpp`
(`UnboundedBlockingQueue<CPUTask, LifoSem>`, no stealing code) `[S]`; Go pre-1.1 `[D]`; GNU libgomp
(one global task lock) `[P]` (XGOMP baseline); Bitsquid `[B]`; Vyukov bounded MPMC `[D]`;
Michael-Scott / LCRQ / SCQ `[P*]`.

**Documented rationale.** Total visibility, no imbalance by construction; Godot's waiter keeps draining
the same queue `[S]`; Folly's LIFO semaphore substitutes wake-order warmth for per-thread queues `[S]`.

**Evidence.**
- Godot: 1 mutex, 2 queues, 0 per-worker deques `[S]`.
- XGOMP beats GOMP's central queue by up to 96.5× (NQueens) on 192 cores `[P]`.
- Vyukov: 1 CAS/op; 75 cycles/op on a dual-core is the only number on the page `[D]`.
- Folly ships this at Meta scale with no stealing at all — the sanity floor: more machinery must earn
  its place `[S]`.

**Failure modes.** Cache-line ping-pong on head/tail/lock; throughput saturates then declines with
cores; zero locality; condvar notify per push.

**Negative results.** N1, N4, N24 (kept, at coarse granularity).

**Cost.** SPAWN: lock or CAS on one shared line + notify. STEAL: n/a. IDLE: condvar.

**Fit.** The **null hypothesis**: immune to A and B by construction, at a per-task cost the
criterion rules out for µs chunks at W=16. Not a candidate; the reference point for "how much does
our machinery earn".

---

### P15 — Nothing queued: lazy task creation, stack-as-deque, return barriers

**Axis cell.** 1: nothing placed; the continuation is made stealable in place · 2: receiver, reaching
into the victim's stack · 10: spawn path reduced to a store; all work-stealing cost on the steal path.

**Implemented by.** Mul-T lazy task creation (Mohr, Kranz, Halstead) `[P]`; X10WS OffStack /
Try-Catch (Kumar et al. OOPSLA'12) `[P]`; ReturnBarrierWS (VEE'14) `[P]`; Cilk-5 two-clone frames
`[P]`.

**Documented rationale.** "Each of these operations is only required for tasks that are actually
stolen"; steal ratios "range from 1 in a million to 1 in 10" `[P]`. Oldest-first because the oldest
continuation "represent[s] the largest available subtree" `[P]`.

**Evidence.** Mul-T fib-20 on 16 processors: eager 0.83× (slower than serial), load-based inlining
3.43×, lazy 6.25×; lazy created <1 % of possible tasks `[P]`. X10WS sequential overhead 4.1× →
15 %; heap-allocated frames were "just under half of the total overhead", deque ops ~30 % `[P]`.
Return barriers: −29 % to −60 % dynamic overhead; 30 % of steals "free" `[P]`.

**Failure modes.** Requires walking or rewriting another thread's stack — not available in stable
Rust. Steal latency rises.

**Negative results.** N38, N49 (heap frames on the spawn path).

**Cost.** SPAWN: one store. STEAL: expensive.

**Fit.** The cost-model endpoint of axis 1. Not buildable here (E14); the transferable datum is that
**a heap allocation per spawn was half of X10's overhead** — our `Box` at `scope.rs:339` (→W14).

---

### P16 — Arena / pool isolation: stealing forbidden across the boundary (the designed twin of P0)

**Axis cell.** 1: per-arena queues · 2: receiver within the arena only · 11: deliberately
partitioned reachability · 7: arena pinned to a NUMA node / core type.

**Implemented by.** HPX resource partitioner: "Task stealing never occurs between different thread
pools." `[D]`; oneTBB `task_arena::constraints`, `create_numa_task_arenas()` `[D]` (no statement about
cross-arena stealing in the docs).

**Documented rationale.** Isolate a latency-critical or NUMA-bound workload from, and protect it
from, other work; the price — unreachable work across the boundary — accepted knowingly.

**Evidence.** HPX states the rule as absolute; TBB leaves it implicit `[D]`.

**Failure modes.** This is P0 with the difference that **nothing is expected to cross**: HPX documents
it, so nobody spawns cross-pool work by accident. Our `injector_local` is partitioned but fed by the
ordinary spawn path. Static isolation cannot absorb uneven lane drain.

**Fit.** Marks the axis-11 value "broken by design"; ours is "broken by accident". Loc: the only
reason to partition reachability is a topology the single-socket target does not have (→L4; but
see L13 — the target class has sub-LLC clusters). The per-THREAD designed twin is thread-per-core
(→P19).
