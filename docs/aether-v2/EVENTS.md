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

Rejected for the opt-in: the outbox pattern (full determinism, zero kernel changes — but one event
per entity per frame, a serial O(N) sweep costing 4–8× the pass itself, and a permanent widening of
the hot row).

## Safety obligations and gates

Lane exclusivity = the lane function is injective over {workers} ∪ {dispatcher} OS threads (TLS is
written in exactly two places; lanes = worker_count + 1; the const-assert closes the 65th-thread
hole; unattached threads are excluded by the re-aimed debug assert, and in release are documented to
route through the world-side send). No user code runs inside any `&self` batch write window. The
swap barrier and writer-handle uniqueness are untouched. The new `par_for_each_chunk_entities`
driver is genuinely new unsafe (the entity-slice aliasing contract) — it goes through the
code-reviewer gate, with a loom/stress story for the lane path, and the determinism gate for
`ordered` is a fixed-scenario replay comparing event streams across two runs at different worker
counts.
