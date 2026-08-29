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
compiled const-assert (today 64 + 1 > 64 — the a1 harness hang class). Cost: one extra 128-byte
lane pair + `capacity × size_of::<E>()` per preregistered type, setup only. `EventConfig`
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

> **OPEN BALLOT AB-4 — what registers the sender-exclusivity fact.** The refusal above names its
> *timing* (boot-time — already pinned by three sites) but not its **predicate** or its
> **registrant**, and a refusal that names neither cannot be implemented or made red. Alternatives
> for the registrant: (a) the **generated path** calls a `register_ordered_emitter` at plugin
> build; (b) **`SystemMeta` emit-access** — the fact is derived from declared access, not
> registered; (c) the **`#[event]` macro side** owns it. Second, coupled question: the
> **verbatim-escape disposition** — a hand-written `EventWriter<E>` param bypasses every generated
> registration, so either the param list refuses it for an `ordered` type, or such senders are
> declared out of contract. Blocks **R4**. Companion red fixture (KERNEL-BACKLOG KE8): a **two-
> emitter** fixture for this refusal, plus loom/stress re-scoped to include a
> `WORKER_ID_UNATTACHED` thread sending concurrently with worker 0 — as scoped today the debug
> assert excludes that class by construction, so the stress cannot reach it.

Rejected for the opt-in: the outbox pattern (full determinism, zero kernel changes — but one event
per entity per frame, a serial O(N) sweep costing 4–8× the pass itself, and a permanent widening of
the hot row).

## Safety obligations and gates

Lane exclusivity = the lane function is injective over {workers} ∪ {dispatcher} OS threads (TLS is
written in exactly two places; lanes = worker_count + 1; the const-assert closes the 65th-thread
hole).

⚠ **The release-mode clause previously written here was false, and is replaced by the measured
behaviour.** Unattached threads are **not** routed anywhere else in release: every unattached OS
thread maps to **lane 0 — worker 0's own lane**. The mapping lives in
`boyko_threadpool::current_worker_id_or_dispatcher_lane` (`crates/boyko_threadpool/src/tls.rs`),
whose `WORKER_ID_UNATTACHED` arm returns `0`, while a worker returns its own id — so worker 0 also
returns `0`. `EventDispatcher::send_event`
(`crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs`) is the caller that turns that value
into the lane index. The unattached→lane-0 half is pinned today by
`event_send_from_unattached_thread_uses_lane_zero` (`crates/boyko_ecs/tests/event_send_from_worker.rs`);
the worker→own-id half is the same function's fall-through arm. So injectivity does
not hold over {workers} ∪ {dispatcher} ∪ {any other thread}: a host thread and worker 0 collide on
one lane. The re-aimed `debug_assert` guards the **param path only**; `EventDispatcher::send_event`
is guarded by **nothing** — and its own doc comment asserts a distinct id per thread, on the same
page as the EVT1 claim it contradicts (`event_dispatcher.rs`, `send_event` doc / EVT1 paragraph).
This paragraph records the measurement; it is **not** a contract.

> **OPEN BALLOT AB-3 — unattached-thread lane contract.** The truthful CONTRACT text lands with
> this ruling, not before. Alternatives: (a) **per-thread claimed host lanes** — raise the const
> which is `MAX_EVENT_THREADS = 64` in the tree (65 is KE8's unlanded plan value), so the raise is
> `64 → 66+`, two lanes while KE8 is unlanded — one for its own const-assert, one for the claimed
> host lane — with one-claimer enforcement; (b) **`Err` on unattached** — breaks silent
> main-thread senders that work today; (c) **accepted hazard**, documented as such and left
> unguarded. Blocks **R4** (the `&self` send). Companion doc-rot fixes ride the same commit:
> `EventWriter::send` doc and the `EventDispatcher` EVT1 paragraph.

**Lane-count obligation.** The obligation `lanes = worker_count + 1` above is stated as a *safety*
property, while `with { lanes N }` (CONSTRUCTS §`event`) lets an author set the count. An
author-settable `lanes N` must not be able to violate a stated safety obligation while parsing
green: the parse check enforces only the constant **ceiling**, and the binding constraint is the
machine-dependent **floor** `lanes >= worker_count + 1`, which is not representable at parse time.
Reconcile with whatever ballot **AB-2** selects (minimum-raised-at-boot / boot refusal / drop the
knob) — until then this is a known hole, not a guarantee.

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
