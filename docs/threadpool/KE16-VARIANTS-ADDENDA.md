# KE16 — Variants added in refutation round 1: P17–P22, G18–G20, J12–J13, W15–W19, L11–L13

Part of the KE16 design space. Index, axes, legend and shortlist: `KE16-DESIGN-SPACE.md`.
Negative results `N…` and unverified claims `U…` are in `KE16-EVIDENCE.md`. Cost anchors as in
`KE16-VARIANTS-PLACEMENT.md`.

These are the designs two refuters found that the five lenses had not opened, plus the cells whose
"empty" verdict the refuters overturned. Each keeps its group letter so cross-references stay by
id; the group files carry a pointer here. Two provenance tags are specific to the refutation round
(defined in the index legend): `[R:S]`/`[R:D]`/`[R:P]`/`[R:B]` — read by a refuter at the stated
kind, **not re-opened by this synthesis**; a plain `[S]`/`[D]`/`[P]` in this file means this
synthesis re-read the source in this session.

---

## Group P additions — placement

### P17 — Sender-chosen placement into a REGISTERED per-worker queue (push with steal fallback): the owner's mechanism 3 in its buildable form

**Axis cell.** 1: another worker's queue that IS in the scan set · 2: sender chooses the destination
· 34 (placement key): idleness, hash, affinity id, data owner, or first-accepting · 11: closed ·
25: the task itself. Distinct from P6 (a *non-stealable* handoff — what Go and FJP reject), P5 (a
proxy with dual residency), P2 (own injector only) and P12 (every inbox).

**Implemented by.** Java FJP `externalPush`: an external submission lands in a submission queue at an
EVEN index of the same array workers scan, chosen by the submitter's `ThreadLocalRandom` probe
`[R:S]`. HPX `thread_schedule_hint_mode::thread` — "prefer scheduling a task on the local thread
number associated with this hint", the scheduler free to ignore it, queues stealable within the NUMA
domain `[R:D]`. BEAM: a readied process goes to the last scheduler it ran on unless its migration
path says otherwise; an empty scheduler steals `[R:D]`. Linux CFS `select_task_rq_fair` /
`select_idle_sibling`: a woken task is enqueued on an idle CPU in the waker's LLC domain, and an
idle CPU pulls (`sched_balance_newidle`; definition line not confirmed by the refuter's fetcher)
`[R:S]`. NVIDIA PhysX `PxDefaultCpuDispatcher::submitTask` — verbatim: `for(PxU32 i=0; i<nbThreads;
++i) { if(mWorkerThreads[i].tryAcceptJobToLocalQueue(task, currentThread)) { … return; } }
if(mHelper.tryAcceptJobToQueue(task)) …` — the sender pushes into the FIRST accepting bounded local
queue in index order, then a shared helper queue; workers take own-local then shared; no
worker-to-worker stealing, reachability closed by the shared fallback `[S]`.

**Documented rationale.** FJP: sampling submission and worker queues from one array "is simpler if
they are all in the same array"; wasted null probes "still cheaper than alternatives" `[S]` (lens 2).
PhysX exposes the idle policy as a per-dispatcher enum (`eWAIT_FOR_WORK` / `eYIELD_THREAD` /
`eYIELD_PROCESSOR` + `yieldProcessorCount`) `[S]` — no other surveyed pool does.

**Evidence.** The correct form of the P6 negative is narrower than the prior synthesis stated: no
runtime places work where ONLY the chosen target can reach it; several place it into the chosen
target's *stealable* queue (FJP, HPX hints, BEAM, CFS, PhysX) `[R:S]`/`[S]`. In boyko the cell is
**P2 plus one index**: `push_task` already loads the idle mask (`worker.rs:326` `[L]`) — after the
placement is committed. Reordered: pick `target` = a set bit of that mask (own injector when the mask
is zero), push to `injector_local[target]`, unpark `target`. The wake and the placement then agree
for the first time.

**Failure modes.** Go's rejected-approach-#2 objections apply *in part*: the target may be out of work
"the very next moment", but with a stealable destination the task is not stranded; the chunk leaves
the spawner's core by construction (for a flat `par_iter` wave whose rows are not yet touched this
is Unity's steal-half-for-locality case, not Go's dependent-goroutine case); a write to a FOREIGN
cache line on the spawn path (the P5 cost minus the proxy); a commitment made on stale information.

**Negative results.** N2, N6 (apply to the non-stealable form); N37 (NA-RP's >100 ns per pushed task
is the measured cost of exactly this write for fine tasks).

**Cost.** SPAWN: P0's `Injector::push` (SeqCst CAS + slot `fetch_or`) on a foreign line + the idle
load P0 already pays + one unpark aimed at the right worker. STEAL: P2's scan (siblings probe
injectors too). IDLE: P2's. COMPLETE: unchanged.

**Fit.** M3: **the only form of the owner's proposal the record does not reject** — candidate
**M3-s** in the index, to be measured on the grid against A1 and A3. Prognosis `[I]`: at 1 µs bodies
the foreign-line write plus a per-spawn unpark loses to A1 (NA-RP's number); at ≥100 µs bodies with
siblings parked at a wave boundary it may beat A1's "spawn locally and wait for a thief" by one
steal latency. That is the measurement, not a verdict.

---

### P18 — Per-worker Injector reached through a placeholder job on the REGISTERED deque (rayon `spawn_fifo`)

**Axis cell.** 1: a per-worker crossbeam `Injector` — boyko's `injector_local` shape · 11: closed
WITHOUT scan-set membership: the deque carries an indirection · 16: an extra `JobRef` per spawn.

**Implemented by.** rayon-core `job.rs::JobFifo { inner: Injector<JobRef> }`; `push`: "A little
indirection ensures that spawns are always prioritized in FIFO order… either way they will end up
popping from the front of this queue" — `self.inner.push(job_ref); JobRef::new(self)`; `execute`:
`loop { match this.inner.steal() { Success(job_ref) => break job_ref.execute(), Empty =>
panic!("FIFO is empty"), Retry => spin_loop() } }`; `registry.rs::WorkerThread::push_fifo` =
`self.push(self.fifo.push(job))` `[S]`. RFC 0001: "we actually push two items: First, we push the
task itself onto the FIFO… Second, we push an indirect task onto the worker's thread-local deque";
measured "performs equivalently to today's code" with a stated "indirection overhead" `[R:D]`.

**Evidence.** A reachable per-worker `Injector` of unbounded capacity exists in Rust on the same
crossbeam types boyko uses; thieves reach it through the scan they already run, at zero extra idle
probes `[S]`. This overturns E1's "nobody" in the reachability sense; the exact scan-set form (P2)
remains unbuilt.

**Failure modes.** The placeholder push needs the worker's own deque — **P1's TLS deque pointer is a
prerequisite**; an `Injector::steal` (fence + CAS) per executed job on top of the deque pop;
boyko's deques are already FIFO at both ends (L1), so the FIFO-for-spawns intent buys nothing here
unless the deques are switched to LIFO.

**Cost.** SPAWN: `Injector::push` (as P0) + a Chase-Lev push of the placeholder (no RMW). STEAL: the
existing deque scan + one `Injector::steal` on execution. IDLE: unchanged.

**Fit.** A: **A2′** — a second way to keep `injector_local`; dominates P2 on the idle path, loses
to P1 on the spawn path (one more Injector CAS). Worth a row on the grid only if the locality intent
of `injector_local` (spawns behind the deque's existing work) is shown to matter.

---

### P19 — Thread-per-core: a per-worker queue polled only by its owner, at unbounded capacity, by design

**Axis cell.** 1: per-worker queue, owner-only · 2: sender routes cross-core work by DATA OWNERSHIP
into a bounded SPSC ring per shard pair, drained in batches · 34: data owner · 12: no wake — a
polling reactor.

**Implemented by.** ScyllaDB Seastar (`smp_message_queue` on `boost::lockfree::spsc_queue`,
`queue_length = 128`, `batch_size = 16`; "each core works on data in its own part of memory, and
communication between cores happens via explicit message passing") `[R:S]`/`[R:D]`; glommio
(tasks are `!Send`; each `LocalExecutor` owns its queues; `shared_channel`) `[R:D]`.

**Evidence.** Corrects axis 1's prior note "no published precedent at unbounded capacity": the
precedent exists and is *designed* — with the discipline P0 lacks (sender-routed inboxes, batched
flush, nothing expected to cross). The accurate statement is: **no precedent where such a queue is
fed by a work-stealing pool's ordinary spawn path.**

**Fit.** The per-THREAD designed twin of P0 (P16 is the per-POOL twin). Not applicable to a
work-stealing pool; recorded to close the axis.

---

### P20 — Random shared priority heaps, two-choice take (Julia partr)

**Axis cell.** 1: any of `heap_c · W` shared heaps chosen by random `trylock` (no owner, no deque, no
centre, no partition) · 4: take = power-of-two-choices on minimum priority · 13: wake at most one.

**Implemented by.** Julia `base/partr.jl`: `heap_c = UInt32(2)`; insert `while
!trylock(tpheaps[rn].lock) rn = cong(heap_p) end`; take `rn1 = cong(heap_p); rn2 = cong(heap_p);
… if prio1 > prio2 { prio1 = prio2; rn1 = rn2 }` `[S]`; `src/scheduler.c` wakes "at most one
sleeping thread", replacing an O(`jl_n_threads`) broadcast `[R:S]`.

**Evidence.** A producer never touches a contended line deterministically (trylock retry to another
heap); a consumer samples two heaps — the only shipped occupant of the two-choice family the theory
lists (E24).

**Fit.** Not a candidate (every op takes a lock; ordering is priority, not LIFO/FIFO). Marks axis 1
and the wake-all negative N52.

---

### P21 — One shared ring, per-consumer cursors, slot claim by exchange (Jolt Physics)

**Axis cell.** 1: a single ring every consumer walks from its own cursor · 15: no deque — a
`nullptr` exchange claims a slot · 33: wake count = `min(jobs, threads)` · 19: a barrier wait runs
ONLY the barrier's own jobs.

**Implemented by.** Jolt `JobSystemThreadPool.cpp`: `mHeads = … Allocate(sizeof(atomic<uint>) *
inNumThreads)`; `QueueJobInternal` computes the minimum over all heads ("We calculated the head
outside of the loop, update head (and we also need to update tail to prevent it from passing
head)"); "Wake up threads" → `mSemaphore.Release(min(inNumJobs, (uint)mThreads.size()))`;
`ThreadMain`: "Exchange any job pointer we find with a nullptr" `[S]`. `JobSystemWithBarrier.cpp::
BarrierImpl::Wait` executes the first executable job of THIS barrier only `[R:S]`.

**Evidence.** No stealing exists because every consumer already sees every slot; the producer pays a
W-wide minimum over heads per submit instead. Shipped in a physics engine used in Horizon Forbidden
West `[R:D]` — the physics peer of the four boyko sites.

**Cost.** SPAWN: an O(W) min over head cursors + a semaphore release of `min(jobs, threads)`.
STEAL: none. IDLE: semaphore (`Acquire(max(1, GetValue()))` batches acquisitions `[R:S]`).

**Fit.** A G12 relative with per-consumer cursors; the O(W) scan on the spawn path rules it out
under the criterion for µs chunks; its own-barrier helping is a shipped J2 occupant in the domain
closest to ours.

---

### P22 — No queue at all: generation broadcast + per-worker claim cursors (ForkUnion)

**Axis cell.** 1: one descriptor, a `fork_generation` counter, no queue object · 3: static slices or
per-worker `fetch_add` cursors stolen when a worker's own slice is exhausted · 26: TPAUSE/WFET
parking · 9: nesting BANNED.

**Implemented by.** ForkUnion (Rust/C++, 2025) README: "spinning on old values of fork_generation
and stop"; "zero heap allocations, zero system calls, zero CAS operations"; stated limitations
`[R:B]`. Numbers in the README (e.g. 128× Xeon N-body dispatch 54/86 µs vs rayon 483/739 µs) are
author-run — recorded in §U, not relied on.

**Fit.** A G12/G18 hybrid; the only surveyed pool parking on hardware timed-wait (→W19); a second
shipped Rust instance of Unity's nested-spawn ban (S4, N56). Not a pool candidate for a nested-scope
API.

---

## Group G additions — granularity and promotion

### G18 — Static partition + steal from the victim's UNSTARTED remainder (libomp `static_steal`; affinity scheduling)

**Axis cell.** 1: fixed partition (as P13) · 2: receiver steals the unstarted remainder (unlike
P13's "none") · 3: a fraction of the remainder · 7: the stable owner→range map · 30: skewed by core
class.

**Implemented by.** LLVM libomp `kmp_dispatch.cpp` `kmp_sch_static_steal`: "steal 1/4 of remaining"
when `remaining > 7`, else "steal 1 chunk of 1..7 remaining"; 4-byte IVs use an 8-byte CAS on the
`(count, ub)` pair, 8-byte IVs "use lock for 8-byte induction variable"; on hybrid parts
"Iterations are divided in a 60/40 skewed distribution among CORE and ATOM processors" `[S]`.
Markatos & LeBlanc, IEEE TPDS 5(4) 1994 — "simultaneously balance the workload, minimize
synchronization, and co-locate loop iterations with the necessary data" `[P*]` (abstract only).

**Evidence.** This occupies E9 ("stable partition + steal fallback"), which the prior synthesis
marked unbuilt and blocked on the residency instrument. It is shipped at the LOOP level — the level
`par_iter` lives at. The verdict "blocked" was a choice, not an absence of precedent.

**Failure modes.** A per-loop CAS on the victim's `(count, ub)` word per steal; locks for wide
IVs; the partition is by iteration count, so skew across archetypes still lands on the barrier.

**Cost.** SPAWN: one descriptor. STEAL: one CAS on the victim's cursor per steal (a quarter of the
remainder). IDLE: unchanged.

**Fit.** A `par_iter` shape: each worker owns a contiguous row range (flecs's L8 invariant) with a
steal fallback; presupposes the range descriptor is reachable (P1–P4). Worth a design note for the
KE15 chunk-runner rework alongside G12; not a pool variant.

---

### G19 — Geometrically shrinking batch claim (guided / trapezoid / factoring)

**Axis cell.** 3 × 17: the batch SHRINKS as the shared index drains — few atomics at the head, a
small tail floor. Distinct from G12 (fixed batch, tail floor = one batch) and G11 (predicted cost).

**Implemented by.** libomp `kmp_sch_guided_iterative_chunked` — `limit = init + (UT)((double)
remaining * *(double *)&pr->u.p.parm3)`; a shrink-factor comment is NOT present in the source
`[S]`; `kmp_sch_guided_analytical_chunked` `[S]`; GSS/TSS/FAC survey arXiv:1809.03188 `[P*]`
(names only).

**Fit.** Refines axis 27's "tail-latency floor = one chunk", which the prior synthesis treated as
fixed. A `par_iter` knob over G12; not a pool variant.

---

### G20 — Batch spawn: one queue operation and one counter RMW per WAVE

**Axis cell.** 10 × 16 × 31: amortises the SPAWN-path atomics over the wave while keeping real task
objects (unlike G12) and independent of allocation (W14).

**Implemented by.** Tokio `scheduler/inject/rt_multi_thread.rs::push_batch` — links the tasks first,
then inserts the whole chain under one lock with one `len.store` `[R:S]`; Go `runqputbatch` →
`globrunqputbatch` moves a batch under one `sched.lock` `[R:D]` (golang/go #40457).

**Evidence.** boyko registers each task with `pending.fetch_add(1, AcqRel)` (`scope.rs:132` `[L]`)
and pushes each chunk separately; a `par_iter` wave is ≤W chunks and a physics wave 68–102 (index
A.4). `pending.fetch_add(N)` once per wave and one queue insertion per wave removes N−1 shared RMWs
on the spawn path **independent of which placement wins**. crossbeam has no batch push; the
Chase-Lev push is RMW-free per task, so under P1 the remaining saving is the `pending` RMW; under
P2/P3 a batch `TaskHandle` carrying N descriptors (the G12 shape) removes N−1 Injector CASes too.

**Cost.** SPAWN: one RMW per wave instead of per task. STEAL/IDLE: unchanged.

**Fit.** A/B: **candidate App-4** — a lever the prior shortlist could not see because axis 31 was
absent. Composable with every A-candidate.

---

## Group J additions — the blocked thread

### J12 — Scheduler-observed blocking → replacement worker (Linux cmwq; Windows IOCP)

**Axis cell.** 5 / 22 / 32: the block is detected in the scheduler with no API call; the replacement
is priced by an exact runnable count.

**Implemented by.** Linux Concurrency Managed Workqueue: "when the last running worker goes to
sleep, it immediately schedules a new worker so that the CPU doesn't sit idle while there are
pending work items" `[D]`. Windows I/O completion ports: the concurrency value "limits the number of
runnable threads associated with the completion port"; "Threads that block their execution on an
I/O completion port are released in last-in-first-out (LIFO) order… the system releases the last
(most recent) thread"; a waiter may run "if another running thread… enters a wait state for other
reasons"; "there may be a brief period when the number of active threads exceeds the concurrency
value"; "a good rule of thumb is to have a minimum of twice as many threads in the thread pool as
there are processors" `[D]`.

**Evidence.** J4 and J5 are CALLER-declared (`block_in_place`, `ManagedBlocker`, `ScopedBlockingCall`);
these two need no declaration and count runnable threads exactly.

**Fit.** Axis 32's cheapest value, unavailable to a user-space game pool without a kernel hook.
boyko's joiner parks inside a system body and the dispatcher parks for the frame (J11) with no
detection of either. Recorded; not a candidate.

---

### J13 — Time-gated compensation (Chromium `base::ThreadPool` MAY_BLOCK vs WILL_BLOCK)

**Implemented by.** `thread_group_impl.cc`: `kBackgroundMayBlockThreshold = Seconds(10)`,
`kBackgroundBlockedWorkersPoll = Seconds(12)` (foreground values come from
`kThreadPoolForegroundMayBlockThresholdParam.Get()` — not constants in the file); "execution
throughput should not be reduced forever if a task blocks forever"; "minimize impact on foreground
work, not maximize execution throughput" `[S]`; LIFO idle-worker set `[R:S]`. The MAY_BLOCK /
WILL_BLOCK max-tasks comment the refuter quoted is not present verbatim — the logic sits in
`BlockingStarted()` `[S]`.

**Fit.** The documented cure for FJP's N7 (transient blocks made worse by replacement): compensate
only after a threshold. Also the one pool on record whose stated criterion is the *opposite* of the
owner's. Not a candidate.

---

## Group W additions — wake protocol

### W15 — Wake-rate throttling on the WAKE path only (Folly `ThrottledLifoSem`) — occupies E3

**Axis cell.** 12: eager publication, but a sleeper is woken at most once per interval · 33: one
waiter in a "waking" state · 26: LIFO semaphore.

**Implemented by.** Folly `synchronization/ThrottledLifoSem.h`, verbatim: "ThrottledLifoSem is a
semaphore that can wait up to a configurable wakeUpInterval before waking up a sleeping waiter";
"This is realized by having at most one sleeping waiter being in a 'waking' state: when such waiter
is awoken, it immediately goes to sleep"; "since wakeUpInterval is relative to the last awake time,
in the regime where post()s are spaced at least wakeUpInterval apart the waiters are always awoken
immediately"; `Options { std::chrono::nanoseconds wakeUpInterval = {}; }` — default zero `[S]`.

**Evidence.** Exactly the E3 sub-variant the prior synthesis called "nobody" (W10: "publish eagerly,
unpark at most once per H µs") — shipped, and cited by the prior synthesis itself under W6 without
noticing. Distinct from W10 (gates *publication*) and W8 (one spinner relays).

**Failure modes.** A knob (the interval); one waiter is always in the waking state, so a burst of
posts is absorbed by a single thread until the interval elapses — a latency floor of one interval
per additional waiter.

**Cost.** SPAWN: a semaphore post (one RMW). WAKE: at most one syscall per interval. IDLE: LIFO
semaphore wait.

**Fit.** M3: bounds the syscall rate with eager publication; composable with W1/W2 (the trigger) and
W17 (the completion path). Candidate only if W-b leaves no-op wakes on the grid; it is now a
shipped design, not an empty cell.

---

### W16 — Wake fan-out COUNT per event (axis 33)

**Values, with occupants.** **one** (Go `wakep`, Tokio `worker_to_notify`, boyko `unpark_one_idle`,
Julia) `[S]` · **`min(new_jobs − idle, sleeping)`** when the queue was empty, else
`min(new_jobs, sleeping)` (rayon `new_jobs`) `[S]` · **`min(jobs, threads)`** (Jolt
`mSemaphore.Release(min(inNumJobs, mThreads.size()))`) `[S]` · **a tier up to a target** (Halide:
"Workers sleep on one of two condition variables, to make it easier to wake up the right number if
a small number of tasks are enqueued"; `target_a_team_size`; "If there's nested parallelism going
on, we just wake up everyone"; a separate channel for semaphore waiters "without a thundering herd
of genuinely-idle workers waking only to rescan and go back to sleep") `[S]` · **all** (flecs
barrier `[S]`; Unity job-completion wakes before 2022.3.62f1/6000.0.48f1 `[R:D]`).

**Evidence.** For a `par_iter` wave of W chunks with W−1 siblings parked, wake-one produces an O(W)
serial wake chain — Bevy #10064's "wakes new threads slower as async executor limits to waking one
thread at a time" is this value, not a trigger defect `[D]`; `min(jobs, sleepers)` pays W syscalls
on the spawner up front; FJP's rule (each activated worker activates another on an empty-queue push,
O(log W) to full activation) is the cascade in between `[S]`.

**Fit.** M3: the prior axis set had the trigger (12) and the order (13) but not the count. Candidate
**W-f**: measure wake count ∈ {1, cascade, `min(N, sleepers)`} on the grid. On a frame-locked load
(axis 27) the count is paid at every wave boundary.

---

### W17 — The completion-path signal: boyko's second and third wake triggers (axis 29)

**Axis cell.** 29: who is notified per task COMPLETION, gated or not, at what cost · 12: two
trigger values the prior list omitted — "every completion" and "at the join, before parking".

**Implemented by (boyko, `[L]` this checkout).** `scope.rs:157-160` `complete_task`:
`self.waker.unpark(); self.pending.fetch_sub(1, Ordering::AcqRel);` — the doc block: "The unpark is
UNCONDITIONAL (no `prev == 1` gate): learning we are last would require reading `pending` after the
sub — too late. An unconditional pre-decrement unpark is a cheap token store when the dispatcher
runs; a spurious wake when it is parked is harmless". `scope.rs:506-512`: before every
`park_timeout(50 µs)` the joiner calls `unpark_one_idle(inner)` — a wake issued on the PARK path.
Compare: rayon's latch set is a swap with a wake only when the owner is in its SLEEPING state
(`latch.rs`, not re-read this session `[R:S]`); Cilk-style continuation stealing has no per-task
completion signal at all `[P]`.

**Evidence.** Per completed task boyko pays an atomic swap on the joiner's parker line (a shared line
written by every completing worker — Rust `std` `unpark` always writes, "even NOTIFIED⇒NOTIFIED"
`[S]`) plus a `futex_wake`/`WakeByAddress` whenever the joiner is inside its `park_timeout`, then a
second shared RMW on `pending`. For a flat wave the completion path runs exactly as often as the
spawn path; the prior cost table had no column for it. Any W-b design that gates only the push-side
wake leaves two per-task wakes ungated on a frame-locked load.

**Gate design (for the architect, `[I]`).** The unpark-before-decrement order is a memory-safety
invariant (`scope.rs:139-151`) and must stay. A joiner-published `parked: AtomicBool` — stored
`true` before the final `is_drained` re-check and the park, cleared after — lets `complete_task`
replace the swap with an Acquire load in the common case (`if parked.load() { unpark }`). The race
(flag set after the completer's load; the joiner's re-check still sees `pending ≥ 1` because the
`fetch_sub` has not happened; the joiner parks; the decrement lands) loses one wake and is covered
by the existing 50 µs backstop — latency, not loss, and no new UAF window because the load precedes
the decrement exactly as the unpark did. Whether a load per completion beats a swap per completion
is a measurement on the grid.

**Cost.** COMPLETE (today): 2 shared RMWs + a conditional syscall per task. COMPLETE (gated): 1
shared RMW + 1 shared load per task, a syscall only when the joiner is really parked.

**Fit.** M3: **candidate W-d**; independent of A and of W-b. Also closes the axis-12 list.

---

### W18 — Topology-nearest / recently-spinning wake target (axis 13 wake ORDER, third value)

**Implemented by.** Linux `select_idle_sibling` — search idle cores / SMT siblings within the
waker's LLC domain `[R:S]`; `CONFIG_SCHED_CLUSTER` exists because single-socket parts have
"clusters of CPUs … sharing mid-level caches, last-level cache tags or internal busses" `[R:S]`;
google/marl `scheduler.cpp`: "Prioritize workers that have recently started spinning. If a spinning
worker couldn't be found, round-robin the workers" — the enqueuer targets a worker that is already
awake and scanning (the dual of Eigen's single spinner, W8); marl spins ~1 ms in 256-iteration
loops of 32 nops before parking `[S]`.

**Evidence.** The prior axis 13 listed LIFO (FJP/Kotlin/.NET/Folly), rotating (boyko) and OS-chosen
only. Composable with boyko's bitmap: `mask & neighbour_mask[wid]` first, then the rotor — one extra
AND on the wake path, no new RMW.

**Fit.** Loc/M3: empty cell **E21** in this tree; blocked on axis 30 (the target's topology is
unrecorded). marl's "recently spinning" preference needs W2's searching state and is then free.

---

### W19 — User-mode monitor/wait as the idle primitive (Intel WAITPKG `umonitor`/`umwait`/`tpause`)

**Implemented by.** Intel intrinsics guide `_umwait`/`_umonitor`/`_tpause` `[R:D]`; ForkUnion parks
on TPAUSE/WFET `[R:B]`. No surveyed pool uses it.

**Evidence.** A hardware wait on a cache line with no kernel object and a bounded deadline sits
between "spin" and "park" on axis 26: an idle worker sleeps on the idle bitmap's line and the
STORE that sets or clears a bit is the wake, so W-a/W-b's unpark can be a plain store.

**Failure modes.** Needs a `cpuid` gate and a fallback; power/latency characteristics are
part-specific; `umwait` deadlines are bounded by an OS-settable MSR.

**Fit.** M3: empty cell **E22**; recorded with an openable spec; not a candidate before the wake
protocol itself is settled.

---

## Group L additions — locality and topology

### L11 — Parallel depth-first scheduling / constructive sharing of ONE shared cache

**Axis cell.** 7: a global depth-first priority across all workers so concurrently running tasks
SHARE the last-level cache rather than split it — the locality theory for the single-socket
shared-LLC case. Distinct from L6 (anchor subcomputations to cache levels) and L4 (victim distance).

**Implemented by.** Liaskovitis, Chen, Gibbons, Ailamaki, Blelloch et al., SPAA'06 brief
announcement (read by the refuter via text proxy) `[R:P]`; full paper SPAA'07 `[P*]`; the bound
`M_1(C + P·D)` from Blelloch & Gibbons SPAA'04 `[P*]` (not opened).

**Evidence.** "bandwidth-limited irregular programs and parallel divide-and-conquer programs present
a relative speedup of 1.3–1.6X over WS, observing a 13–41% reduction in off-chip traffic" on a
simulated CMP `[R:P]`.

**Fit.** M1/Loc: the theory for mechanism 1's locality axis on the target class the prior synthesis
said the hierarchy "collapses" on. Not a candidate; it needs the L10 instrument to be applied.

---

### L12 — Heterogeneous-core awareness: skewed partition and faster-idle-takes-over

**Implemented by.** libomp `static_steal` initial partition "60/40 skewed distribution among CORE
and ATOM processors" `[S]`; Bender & Rabin, "high-utilization" scheduling for heterogeneous
processors, SPAA'00 / ToCS 35(3) 2002 `[P*]` (not opened).

**Evidence.** The prior synthesis mentioned P/E heterogeneity only as an L4 failure mode. Equal-row
`par_iter` chunks are equal work only on homogeneous cores; on a hybrid part the static grain is
wrong by the core-class ratio before any imbalance elsewhere.

**Fit.** Axis 30: record the target's core classes (`lscpu` / `cpuid`) before sizing G8's grain;
a knob on `par_iter`, not a pool variant.

---

### L13 — Sub-LLC affinity scopes with a non-strict escape, and a cache-hot migration threshold (Linux)

**Axis cell.** 7 × 30: preference for an L3/cluster-local worker with work-conservation fallback
— a measured cell between L4 (strict hierarchy) and P13 (static). Plus a numeric proxy for L10.

**Implemented by.** Linux unbound workqueues: affinity scopes `cpu`, `smt`, `cache`, `cache_shard`
(the default), `numa`, `system`; strict vs non-strict `[D]`. CFS pull balancing:
`__read_mostly unsigned int sysctl_sched_migration_cost = 500000UL;` — a task that ran within the
last 500 µs is treated as cache-hot by `task_hot` (body not read) `[R:S]`; hierarchical PULL by the
idle/periodic balancer toward itself (`sched-domains.html`) `[R:D]`.

**Evidence (the only official, harnessed single-socket number in the survey).** AMD Ryzen 9 3900x
(12 cores, 4 L3s), `docs.kernel.org` workqueue "Affinity Scopes and Performance" `[D]`, re-read
this session: saturated (24 issuers) `system` 1159.40 vs `cache` 1166.40 MiBps at ~99.3 % CPU;
8 issuers `system` 1155.40, `cache` 1154.40, `cache (strict)` 1112.00; under-saturated (4 issuers)
`system` 993.60, `cache` 973.40, `cache (strict)` **828.20** — the strict variant loses
work-conservation; the CPU-utilisation columns the refuter quoted (75.49 % / 66.84 %) were not
re-read here (§U). The kernel moved the default from `numa` to `cache_shard` `[D]` (N54).

**Failure modes.** Strict affinity under light load loses ~17 % of bandwidth on the target class
(N54); a 500 µs "hot" threshold is tuned for a general-purpose scheduler, not a 16 ms frame.

**Fit.** Loc/M1: corrects L4's "collapses to L3-shared on a single socket" — the target class has
sub-LLC clusters and the kernel measured the trade-off there. An L3-scope PREFERENCE with a
non-strict escape (`mask & cluster_mask[wid]` first, then any) is the shape to measure if axis 30 is
recorded; the 500 µs threshold is the only shipped numeric proxy for the L10 instrument.
