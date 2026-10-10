# KE17 — the apply-window barrier: what the number licenses

Companion to [KE17-APPLY-WINDOW-MEASUREMENT.md](KE17-APPLY-WINDOW-MEASUREMENT.md),
which carries the mechanism verification, the instrument and the numbers. This
file states what those numbers license and what stands in the way. It was
allowed to conclude "not worth building"; it does not, but the reason it does
not is narrower than the ticket assumed, and the blocker is bigger.

---

## 1. The number, in one line

The apply-window barrier costs **8.1 % of frame time on a shape built node-for-node
from the engine's own Main schedule**, 0 % on a serial chain (which is what the
physics Fixed schedule is), 0 % where there are no successors and no conflicts,
and **31-47 % on shapes that are realistic in kind but not currently present in
the engine**. Spread across five runs on a loaded machine: under one percentage
point on every row.

So: not a fraction of a percent, and the ticket does not close on the size. It
also is not the largest lever in the frame, and it is not shaped the way the
ticket's framing supposed.

## 2. Three things the measurement changed about the ticket

### 2.1 The dominant path is the CONFLICT check, not `pred_remaining`

The ticket described the barrier through successors: "the finished system's
`running` bit STAYS SET … `pred_remaining` of its successors stays above zero."
That path is real and it is what `s1` (47 %) and `s3` (36 %) are made of.

But on the engine-like shape the money is in the other check.
`try_dispatch_ready:1026` rejects a system whose `conflict_bits` intersect the
stale `running` set — and that catches systems **with no predecessors at all**,
which were ready from the first dispatch round.

Traced on `s6_engine_main_like`, which the simulation reproduces within 3.5 % of
the measured frame:

* Round 1 dispatches `{pack (600 µs), snap (30 µs), ssao_gate (1 µs)}`. The other
  two gates are rejected at `:1026` — they write the same `LightingConfig` as
  `ssao_gate`.
* `snap` finishes at 30 µs, `ssao_gate` at 1 µs. Thirteen lanes go idle.
* Nothing retires until `pack` finishes at 600 µs, because `pending` cannot
  reach `running` until then.
* `sv0_gate` — a 1 µs root system — therefore starts at **~600 µs**, held by a
  sibling gate's stale bit, which is stale because an unrelated 600 µs system
  shares the round.
* The remaining gates and `collect_lights` (80 µs) then string out one per
  round, ending at 1 197 µs instead of the 1 100 µs critical path. That tail is
  the 8.1 %.

The measured timeline agrees: `sys 3` at 655 µs, `sys 4` at 1 483 µs, on a shape
whose critical path is `pack → casters` and `pack → gather_mesh_draws`.

**Design consequence.** A fix that only releases `pred_remaining` early would
capture `s1`/`s3` and leave most of `s6` on the table. Clearing the `running`
bit is what buys both, and it is the same one-line site (`:745`) either way.

### 2.2 The shape that hurts most is not in the engine yet

`s1` (47 %) is "a wave of unequal producers each feeding one consumer" and `s3`
(36 %) is "a serial chain running beside one long system". Neither is registered
today: physics is a pure chain (no long sibling in the same schedule), and the
Main schedule's successors are few and short. The engine's exposure is the 8 %
shape.

That cuts both ways. The lever is worth 8 % now; it is worth 30-47 % the moment
someone registers a parallel chain beside a long gather — which is exactly what
"move physics into the Main schedule" or "run a second gather concurrently"
would produce. This is a lever whose value grows with the schedule, and the
schedule is growing.

### 2.3 A 16-worker pool never exceeded 3 concurrent systems on `s6`

`max_inflight = 3` in all five runs, at W=4 and W=16 alike. The ceiling is set by
the round structure plus the `LightingConfig` write class, not by the pool. Two
readings follow, and only the second is this ticket's:

* the five `sync_*_light_gate` systems serialise because they all take a write
  on the same resource. That is a **data-layout** question (do five O(1) bridges
  need to share one write class?), not a scheduler one, and it belongs to
  whoever owns `LightingConfig`.
* given that they serialise, the barrier turns each into a full dispatcher
  round-trip instead of a back-to-back hand-off. That is this ticket's.

## 3. SCH7 — the question that decides everything

The ticket named it and did not answer it: *a successor must not observe a
predecessor's deferred commands before the drain.* The measurement pass found
the answer is worse than "an invariant to preserve". **The barrier is currently
the sole mechanism that provides it, and the codebase says so in writing.**

`crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:833` opens a doc
block that declares the sync-point analyzer a deliberate no-op pass-through, and
gives this justification (`:859-873`, quoted):

> The apply window barrier (plan §2.2 SCH7 / §5.4.5.1) already serialises every
> system's `apply` against every concurrent worker. … Downstream systems run
> only after their predecessors' `apply` calls have returned (the executor sets
> `completed[i]` AFTER `apply`).
>
> Therefore: without explicit `ApplyDeferred` insertion, every
> `Commands`-enqueued mutation is visible to every downstream system — just at
> the cost of one extra dispatcher round per system that has deferred work.

The same doc lists the two prerequisites Phase 9.1 never landed:

> * `SystemMeta`/`SystemBox` does not yet expose a `has_deferred()` query — the
>   flag must thread through `SystemParam::init_access` into a new bit on
>   `Access` or a sibling cache.
> * `ApplyDeferred` is not yet a registered system type …

Confirmed against the source: `grep` for `has_deferred` across
`crates/boyko_ecs/src/ecs/core/{system,schedule}/` returns those three doc lines
and no code. `System::apply` (`system.rs:165`) defaults to a no-op, so the
information exists per-system at runtime but is not surfaced anywhere the
dispatcher can read cheaply.

**So the ordering inside `apply_window_drain` is load-bearing** — clear `running`
(`:745`) → `apply(world)` (`:764`) → `drain_deferred_hook_queue()` (`:771`) →
set `completed` (`:773`) → decrement successors (`:785`). Releasing a successor
before its
predecessor's `apply` has returned removes the guarantee that Phase 9.1 was
skipped on the strength of. It is not a theoretical hazard: it is a silently
wrong read (a successor querying an entity `Commands` has not spawned yet), in
the class this project has repeatedly found to be the expensive one.

### What a design must therefore supply

Any early-release design has to answer *before* touching the gate:

1. **Which systems may be released early.** The safe predicate is "the
   predecessor has no deferred payload". That is `has_deferred` — the exact
   prerequisite Phase 9.1 named as missing. Note that it is knowable both
   statically (does the system declare a `Commands`/`Deferred` param?) and
   dynamically (is the queue empty this frame?), and the dynamic form is
   strictly better here because most frames enqueue nothing.
2. **What happens to the rest.** Systems WITH a deferred payload still need the
   dispatcher's exclusive `&mut EcsMaster` to apply, so they still need
   quiescence — unless `ApplyDeferred` insertion lands, which is the other
   missing prerequisite.
3. **How `apply` ordering stays deterministic.** Today completions are applied
   in queue-pop order inside one window. Two windows means two orders; if any
   system's `apply` is order-sensitive against another's, that is a behaviour
   change, not an optimisation.

The pleasant part: the systems that pay the barrier hardest on the engine's own
schedule — the five O(1) `sync_*_light_gate` bridges — are precisely the ones
least likely to carry a deferred payload. The predicate that makes the fix sound
is the predicate that makes it profitable.

## 4. Verdict

**Do not close the ticket, and do not size the work as a gate change.**

* The lever is **8.1 % of frame time** on the engine's current Main shape, rising
  to 31-47 % on shapes the engine is one registration away from. That clears any
  reasonable bar for "worth building".
* It is **not** the biggest thing in a frame today, and a design that presents it
  as one will be wrong. On a loaded box the residual (dispatch round-trips,
  park/unpark, and whatever the concurrent KE16 pool work settles into) swamped
  it repeatedly.
* The work is gated on **Phase 9.1's two unlanded prerequisites**, not on the
  gate at `:623`. `has_deferred` on `SystemMeta` is the entry ticket. Estimating
  this as "change one condition" would be estimating the wrong thing.

Recommended next rung, in order:

1. Land `has_deferred` as a first-class `SystemParam` predicate (Phase 9.1's own
   first bullet). It is a prerequisite for the sync-point analyzer too, so it
   pays for itself twice.
2. Only then design the split window: retire a completed system whose
   `has_deferred` is false immediately (clear `running`, set `completed`,
   decrement successors) and keep the quiescence gate for the rest.
3. Re-run `ke17_apply_window` unchanged. `barrier/model` is structural and
   load-independent to under one point, so the before/after is readable even on
   a busy box — which is more than can be said for the criterion column.

## 5. Handed off, not claimed

Two observations this pass made and is NOT reporting as findings, because they
belong to work in flight and were measured against a moving target
(`crates/boyko_threadpool/src/*` has 1 800+ uncommitted lines from the
concurrent KE16 workflow):

* Under load, waves of independent systems were repeatedly observed executing
  **serially on a single worker id** while siblings idled — e.g. `s2` W=16, twelve
  independent root systems, all on lane 11, one after another, 15 lanes idle.
  The same shape fanned out to 12 lanes in the quiet run. This is NOT KE16
  defect A (no body here spawns anything; the dispatcher is not a worker), and
  it is not the barrier (the model prices those frames correctly only in the
  quiet run). It is a pool wake/fan-out question and it is KE16's.
* Multi-millisecond gaps between chain links appeared in the unguarded-timer
  smoke run (1.8 ms, 10.2 ms, 12.7 ms), consistent with the missed-wake path
  `PARK_TIMEOUT`'s own doc describes. They did not recur once the machine
  quieted, so no claim is made.

## 6. Status — rung 1 landed, rung 3 read (appended, does not amend §4)

§4's "recommended next rung, in order" has had its first and third items
executed. Recorded here rather than edited into §4 so the coordinates that
other files cite stay put.

**Rung 1 — `has_deferred` as a first-class `SystemParam` predicate: LANDED.**
`SystemParam::HAS_DEFERRED` is a REQUIRED associated const with no default
(`system_param.rs`), so an impl that stays silent is `E0046` rather than a
silent `false` — the one omission that would have made the whole design
unsound. `Commands` is the only `true` on the surface; the tuple impl ORs its
members; `()` is `false`. `FunctionSystem::has_deferred` forwards the const,
`System::has_deferred` defaults to **`true`** (the fail-safe side: a
hand-written system that says nothing keeps its barrier, so a forgotten
declaration costs performance and never soundness), and
`ScheduleBuilder::try_build` folds the answer into a `may_defer` `FixedBitSet`
beside `has_condition`, readable through `Schedule::may_defer`.

**§3's premise SURVIVED the check.** Across every `SystemParam` impl in the
tree — `Res`, `ResMut`, `Option<Res>`, `Option<ResMut>`, `NonSendRes`,
`NonSendResMut`, `Local`, `Entities`, `EventReader`, `EventWriter`, `Query`,
`()`, the twelve tuple arities, the twelve oversized stubs and two test stubs —
`Commands` is the ONLY one that overrides `fn apply` with a body. Everything
else inherits the trait's no-op. The tuple impl's override is a forwarder, not
a payload, and is exactly mirrored by its `HAS_DEFERRED` OR.

**Rung 2 — the split window itself: NOT built, deliberately.** The executor's
retire path is untouched: no change to the gate, the completion pop,
`apply_window_drain` or the `running` clear. It is held until KE16's pool
verdict is committed, because it edits the code path every `SCH7` safety
comment in `schedule.rs` is written about.

**Rung 3 — re-run: DONE, and it did not cancel the feature.** See
[KE17-APPLY-WINDOW-MEASUREMENT.md](KE17-APPLY-WINDOW-MEASUREMENT.md) §8. The
split recovers **8.1 % of `barrier_sim` on `s6`** — 100.0 % of what the barrier
costs there — because `s6`'s single deferring node (`snap_apply`) is short,
conflict-free and on nobody's critical path, while the five `sync_*_light_gate`
bridges that the 8.1 % is made of retire immediately. The decision rule's
close-with-a-number branch (split within 2 points of barrier) is not taken.

One thing §3's list of three questions should gain before rung 2 is designed,
because the measurement found it: **question 2 has a second half.** "What
happens to the rest" is not only "they still need quiescence" — under the split
they may reach quiescence LATER than they do today, because the early releases
keep the `running` set populated and the bar recedes. Measured at -2.5 % to
-12.5 % on one-hot masks over `s1` and `s5` (MEASUREMENT §8.5b). It is free on
the engine's shape today and it is the argument for `ApplyDeferred` insertion
being the other half of the fix rather than an optional follow-up.
