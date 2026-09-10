# Parallel event emission and ordered events

The design that lifts R-PAR (a parallel machine pass could not emit events). Adjudicated from two
investigations plus a critic; rationale → [`DECISIONS.md`](DECISIONS.md) §E1–E3.

## The blocker, precisely

`par_for_each_chunk` requires `Func: Fn + Send + Sync` while `EventWriter::send` takes `&mut self`
— a `Fn` closure cannot call an `&mut self` method on a captured value, so a parallel pass
physically cannot send. Meanwhile the buffer underneath has been interior-mutable since Phase 6
(`send_one(&self)` over an `UnsafeCell` with an `unsafe impl Sync`), and lane selection is already
per-OS-thread (`current_worker_id_or_dispatcher_lane`), so the `&mut` on the public `send` enforces
an exclusivity the lane discipline already provides.

## The mechanism (three tiers)

1. **Kernel enabler**: `send` / `send_default` go `&self` — zero new unsafe. `send_many` STAYS
   `&mut self`: its user-supplied iterator is drained inside the len-read/publish window, so a
   foreign `next()` is a re-entrancy hole, closed structurally by the receiver. The
   send-outside-system `debug_assert` is re-aimed from `is_in_system_run()` (a proxy that a stolen
   task can satisfy by luck) to `current_worker_id() != WORKER_ID_UNATTACHED`; the
   `IN_SYSTEM_RUN` cell becomes a depth counter so inline execution and drop-steal do not trip the
   nesting assert.
2. **Batching**: a new `send_slice(&self, &[E]) where E: Copy` — the generated parallel pass
   accumulates firings in a stack `[E; 64]` and flushes per batch: one Release store and ONE
   `fetch_add(n)` on the shared frame counter. This removes the only non-scaling cost (a shared
   counter RMW per send — up to ~1 ms at 10k sends, which would eat the parallel win).
3. **Router contract**: same-frame duplicates per victim are folded with the per-event deposit
   policy (`inbox (StunHit = max)`); a commutative policy makes the fold order-independent, which
   is exactly where parallel emission would otherwise lose run-to-run stability of machine STATE.

Constants: `MAX_EVENT_THREADS` 64 → **65** so `MAX_WORKERS + 1 <= MAX_EVENT_THREADS` holds as a
compiled const-assert. ✅ **This half has LANDED** (`constants.rs:400` + the const-assert below it,
at `01a4436e`); the rest of KE8 — `&self` send, `send_slice`, the re-aimed debug assert — has not.
Ruling **E5** raises the constant once more, 65 → **66**, for the claimed host lane. Cost: one extra
128-byte lane pair + `capacity × size_of::<E>()` per preregistered type, setup only. `EventConfig`
pre-sizing documents the skew case: work stealing balances rows, not sends — capacity is judged
against the worst case of ONE worker.

`R-PAR` is lifted: `emit<E>` becomes legal in the tick/enter/exit/commit of a parallel pass
(lowered to the stack accumulator + `send_slice`). Still refused: `send_many` with user iterators
from pass bodies; emission from callbacks/`Drop`.

## Ordered events — opt-in [owner: build now, optional]

```
event StunHit { victim: entity(EnemyBrain), seconds: f32 } with { ordered }
```

Default events stay on fast TLS lanes with **no cross-run order guarantee** (nondeterminism enters
at exactly one seam: chunk→thread assignment via work stealing decides which lane an event lands
in, and cross-lane read order at swap follows lane index). `ordered` buys run-to-run byte-stable
order via **chunk-keyed lanes** plus a boot-time **sender-exclusivity refusal**: two systems
declaring parallel emission of one ordered type is a loud failure — which is precisely the
soundness hole that disqualified chunk-keying as the default (a concurrent TLS-keyed sender of the
same type from another system would write the same lane; the kernel cannot express cross-system
exclusivity implicitly, so the opt-in makes it an explicit registered contract). Past 64 chunks the
ordered path falls back to serial emission for that type.

**Sender exclusivity — the registered contract (ruling E6, ballot AB-4 closed 2026-08-30).** The
refusal's *timing* was already pinned at boot by three sites; its **predicate** and **registrant**
are now these.

- **Registrant — the generated path.** The generated plugin calls
  `register_ordered_emitter(event_id, system_id)` at plugin build, alongside the
  `preregister_event[_default]` it already emits. The symbol is new: it appears nowhere in
  `crates/` today.
- **Predicate.** At the end of plugin build, any event id declared `ordered` whose registered
  emitter count is **> 1** is a hard boot failure naming both systems.
- **Verbatim escape — refused at the param list.** A hand-written `EventWriter<E>` param for an
  `ordered` `E` is refused at `EventWriter::init_state`, the site that already fails loudly for an
  unregistered event (`event_not_preregistered_panic::<E>()`) and already has `E::event_id()` and
  the dispatcher in hand. A hand-written system therefore cannot emit an `ordered` event even as
  sole emitter; the escape is to declare the emitter in Aether.

Rejected, with prices, at [`DECISIONS.md`](DECISIONS.md) **E6**: `SystemMeta` emit-access (measured
— `EventWriter::init_access` is an empty body and events are deliberately outside the conflict
graph, so there is no event axis to read; adding one as a *write* would serialise every pair of
same-type emitters and destroy the parallel emission this document exists to enable), and the
`#[event]` macro side (structurally impossible — exclusivity is a property of the emitter set, and
the macro sees one type and zero systems; it does not even read its own attribute arguments today).

Companion red fixture (KERNEL-BACKLOG KE8): a **two-emitter** fixture for this refusal, plus
loom/stress re-scoped to include a `WORKER_ID_UNATTACHED` thread sending concurrently with worker 0
— as scoped today the debug assert excludes that class by construction, so the stress cannot reach
it.

Rejected for the opt-in: the outbox pattern (full determinism, zero kernel changes — but one event
per entity per frame, a serial O(N) sweep costing 4–8× the pass itself, and a permanent widening of
the hot row).

## Safety obligations and gates

Lane exclusivity = the lane function is injective over {workers} ∪ {dispatcher} OS threads (TLS is
written in exactly two places; lanes = worker_count + 1; the const-assert closes the 65th-thread
hole). Under ruling **E5** the domain widens to {workers} ∪ {dispatcher} ∪ {the one claimed host
thread}, at lanes = worker_count + 2; injectivity over the wider set is what one-claimer enforcement
buys, and it is the property the re-scoped stress fixture must actually test.

⚠ **The release-mode clause previously written here was false, and is replaced by the measured
behaviour.** Unattached threads are **not** routed anywhere else in release. The mapping lives in
`boyko_threadpool::current_worker_id_or_dispatcher_lane` (`crates/boyko_threadpool/src/tls.rs`) and
has **three** arms, not two: a worker returns **its own id**, the dispatcher returns
**`worker_count`** — its own reserved lane — and only `WORKER_ID_UNATTACHED` returns **`0`**, which
is worker 0's lane. ⚠ *An earlier revision of this paragraph said "every OS thread that is not a
pool worker maps to lane 0"; that over-stated the collision class, which is threads that never
entered an install scope.* `EventDispatcher::send_event`
(`crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs`) is the caller that turns that value
into the lane index. The unattached→lane-0 half is pinned today by
`event_send_from_unattached_thread_uses_lane_zero` (`crates/boyko_ecs/tests/event_send_from_worker.rs`);
the worker→own-id half is the same function's fall-through arm. So injectivity does
not hold over {workers} ∪ {dispatcher} ∪ {any other thread}: a host thread and worker 0 collide on
one lane. The re-aimed `debug_assert` guards the **param path only**; `EventDispatcher::send_event`
is guarded by **nothing**.

**What the collision costs, measured in release (2026-08-30).** An unattached thread and worker 0
each sending 4000 events into the shared lane, released together on a rendezvous barrier:
**6 of 6 runs lost events** — 1891, 2295, 2070, 183, 1473, 1940 out of 8000 (2.3 %–28.7 %) — and
**every send returned `Ok`**. The barrier is not incidental: without it the unattached thread
usually drains all its sends before the pool task is even enqueued, and 6/6 runs lose nothing. That
is how this hazard stays invisible to a stress test that does not force overlap. Underneath the lost
counts it is a data race on a `MaybeUninit<E>` slot behind an `UnsafeCell` — undefined behaviour,
not merely a lost update.

**Contract (ruling E5, ballot AB-3 closed 2026-08-30): the unattached sender gets a CLAIMED host
lane, one claimer enforced.** `MAX_EVENT_THREADS` 65 → **66**, the const-assert strengthening to
`MAX_WORKERS + 2 <= MAX_EVENT_THREADS`. **One lane, not two** — the ballot said two on the premise
that the 64→65 raise was unlanded; it landed at `01a4436e`, so only the host lane remains to buy.
The first unattached thread to send claims the host lane by CAS on an owner slot and keeps it; a
**second** distinct unattached sender is refused loudly (`Err`, plus a debug panic), because two
unattached senders sharing one lane reinstate exactly the race measured above.

⚠ **This ruling also owes an `unsafe` comment an edit, in whichever commit builds it.** `send_one`'s
`// SAFETY (U4 …)` clause 2 asserts *"only the worker pinned to `thread_index` accesses this
UnsafeCell"* (`event_buffer.rs`). The measurement above falsifies that sentence as the tree stands.
It is also why the "accepted hazard, documented" option was not available: choosing it would have
ratified a `// SAFETY:` comment whose stated invariant does not hold. Rejected alternatives and
their prices → [`DECISIONS.md`](DECISIONS.md) **E5**. Companion doc-rot fixes ride the same commit:
`EventWriter::send` doc and the `EventDispatcher` EVT1 paragraph.

**Lane-count obligation (ruling E4, ballot AB-2 closed 2026-08-30).** The obligation
`lanes = worker_count + 1` above is stated as a *safety* property, while `with { lanes N }`
(CONSTRUCTS §`event`) lets an author set the count. An author-settable `lanes N` must not be able to
violate a stated safety obligation while parsing green: the parse check enforces only the constant
**ceiling** (a real `Result` — `EventConfig::new` returns `Err(InvalidEventConfig)`), and the
binding constraint is the machine-dependent **floor** `lanes >= worker_count + 1`, which is not
representable at parse time.

**`lanes N` is therefore redefined from a count to a MINIMUM.** The effective lane count is
`max(N, worker_count + 1)`, resolved where the worker count is first known, and the raise is
**reported once at boot** rather than applied silently — at the worked shape the overshoot is about
1 MB per event type, which is worth a log line and not worth a refusal.

The floor is unchecked today on **every** path, and not for want of a denominator:
`current_worker_id_or_dispatcher_lane` returns a worker's own id and **ignores** its `worker_count`
argument, so handing it the right number clamps nothing. A violation is a panic in both profiles —
`thread_index 3 >= thread_count 1` in debug, `index out of bounds: the len is 1 but the index is 3`
in release — never a `Result`. ⚠ And the floor is already violated by the **default** path before
any author writes `lanes N`: `EcsMaster::new()` hard-wires `EventDispatcher::new(1)` with no setter,
so `preregister_event_default` allocates **one lane on every machine**. Measured in release: a
4-worker pool + default registration + 64 worker sends → **3 panicked**. Rejected alternatives
(boot refusal; dropping the knob, which is the cleaner language answer but a SCOPE call escalated to
the owner rather than taken) → [`DECISIONS.md`](DECISIONS.md) **E4**.

No user code runs inside any `&self` batch write window. The
swap barrier and writer-handle uniqueness are untouched. The new `par_for_each_chunk_entities`
driver is genuinely new unsafe (the entity-slice aliasing contract) — it goes through the
code-reviewer gate, with a loom/stress story for the lane path.

**The `ordered` determinism gate — re-axed at [`CAMPAIGN.md`](CAMPAIGN.md) R4, the same way R6 was.**
The property the gate must test is *the parallel path emits the stream the serial path emits* — a
fixed scenario's event stream from the parallel pass compared **against the serial-emission
reference stream** for W ∈ {1, 2, N}. `build(1) == build(W)` is demoted to a **smoke** check: it
compares two runs of the same code against each other and so cannot see a lane-assignment defect
that is stable across runs, which is exactly the defect class `ordered` exists to exclude. The run
must **report per-worker send counts** (AIR-12's counts-not-exit-code rule) so a silently-serial run
is red, not green.
