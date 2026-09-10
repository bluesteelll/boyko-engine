# Aether v2 — decision log

Every call made in the 2026-08-27..28 design sessions: what was chosen, what was rejected, and why.
Owner-made calls are marked **[owner]**; delegated calls decided under the standing "perf and
architecture forks are decided without asking" rule are marked **[delegated]**.

The recurring test behind most of these: *a check that cannot fail is not a check; a syntax in which
the mistake cannot be written beats a diagnostic that catches it.*

---

## Language shape

**S1. Metadata moves into `with { }` groups; every line is `keyword payload`; every list is
parenthesised.** Rejected: the v1 flat item list (a reader must classify each line by memorised
vocabulary — the owner's original complaint: "каша, не ясно что тег что компонент что хук");
Python-style header inheritance `component X: A + B [tags] <hooks>` (imports the wrong semantics —
`require` is not inheritance; `<>` collides with generics so a generics mistake would produce a
hooks diagnostic; `+`-separated groups do not scale to name-value keys).

**S2. No optional-bracket forms, ever.** One spelling per construct. Rejected: short forms without
parentheses for single-item lists. Reason: the measured `at` lesson — the eager no-paren parse
swallows a node body, the hint needed a heuristic, and `sdf` was left with no diagnostic at all,
structurally. Two grammars for one construct is the class, not the instance.

**S3. Negative flags become values: `bundle = off`, `clone = off`, `serialize = off`.** Rejected: a
`tags (no_bundle, no_clone, …)` group. Reason: only `no_bundle` is a pure flag; `no_clone` and
`no_serialize` have value-bearing siblings (`clone = fn`, `stable_name`), so a tags group splits one
concept across two groups — and `clone = off` vs `clone = fn` makes the derive's mutual-exclusion
error *unwritable* instead of diagnosable.

> **Ballot AB-13 (open — do not settle by edit).** S3 fixed the *negative-flag* vocabulary
> (`… = off`). It did **not** fix the **initial-value vocabulary of the `flag` construct**
> (`flags (Visible = on, Stunned = off)`), and nothing in the record does.
> The ballot must answer all four parts together, because they interact:
> **(1)** the value vocabulary itself — `on | off` vs alternatives (`true | false`,
> `set | clear`, `enabled | disabled`);
> **(2)** whether the chosen words are **reserved keywords or contextual** (contextual keeps them
> usable as identifiers, at the price of a grammar that reads differently in two places);
> **(3)** disambiguation across **all three** `on` positions already in the surface —
> `machine … on entity`, `on E => T` (a transition head), and `flags (X = on)` (a value) — a
> reader and a generator must be able to tell them apart without lookahead;
> **(4)** the group's NAME. PENDING's Tier 3 does not propose `flags → initial` — it **withdraws**
> that rename, on the ground that `initial` is already the machine's initial-state keyword, so the
> rename recreates the collision it was meant to fix
> ([`../gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) Tier 3). Part (4) is
> therefore: does that withdrawal stand, or is a DIFFERENT rename wanted?
> ⚠ Any rename here **touches a ratified keyword**; under AIR-10 a ratified word is not relitigated
> without measurement, and the offered measurement is the collision audit over the three `on`
> positions. Blocks: **R3** (the `flag` construct surface and its filter goldens).

**S4. The kernel-facing group is named `kernel`.** Rejected: `meta` (the whole `with` block is
metadata — the name describes the container, not the group), `policy` (second choice, kept as
fallback).

**S5. Field-head keywords `link` (entity remap opt-in) and `key` (relationship FK marker).**
Rejected: `remap` (names the mechanism, not the author's claim — the claim is "the referenced
entity is saved together with me"), `ref` (a real Rust keyword), `persist`/`saved` (read as
per-field serialization, which does not exist).

## Constructs

**C1. `tag` and `flag` are two constructs.** Rejected: v1's `tag X(bitset)`. Reason: the two have
different metadata surfaces (a bitset tag can take no hooks — no pool, they can never fire; no
requires — no insert path; bundle suppressed), so a separate keyword makes the illegal combinations
*unwritable*. Rejected: `tag`+`marker` naming (synonyms encode nothing; `flag` says "toggled").
**AIR-10 familiarity axis (audit, no measurement):** `tag` is a *true friend* — "tag" already means
a zero-sized marker component in Unity, Bevy and flecs, which is exactly what Aether's `tag` is.
`flag` is a *near friend*: in general programming a flag is a boolean that is set and cleared, and
Aether's `flag` is precisely a toggleable bit. The residual risk is the pair, not either word — a
generator that wants a *toggleable* marker may reach for `tag` because that is the ECS word it
knows, and the refusal it then meets must name `flag` in its did-you-mean. Both spellings are kept;
a rename would need the AIR-10 measurement, which has not been run.

**C2. `bundle` gets two forms — positional (primary) and named.** The derive already accepts tuple
structs; v1 was stricter than the derive for no reason, and the field name in a named bundle is a
pure repeat of the type. Named stays for `..base` struct-update and named missing-field errors.
Rejected: order-free positional construction via generic slot traits — it converts a missing or
duplicated slot from a compile error into a runtime panic (compile-but-lie, the recorded lesson).

**C3. `event` gains `with { lanes N capacity N }` + plugin auto-registration + a generated flat
constructor `E::new(...)` in source field order.**

*The original reason for the auto-registration half was refuted by measurement.* It read
"forgetting registration today fails *silently on both ends* (reads return an empty slice; the
canonical `let _ = send(...)` swallows `EventNotRegistered`)". The engine says otherwise:

- **Both generated ends panic loudly at init.** `event_not_preregistered_panic` is raised from the
  reader and the writer alike (`event_reader.rs:317-320`, `event_writer.rs:194-197`, message in
  `diagnostics.rs:79-87`), and Aether lowers its event params to exactly these two — `expand.rs:329-330`.
  A forgotten registration is a loud boot failure in every generated program, not a silent read.
- **The `let _ = send(...)` clause is deleted, not narrowed:** `EventWriter::send` cannot return
  `EventNotRegistered`, so there was never a `Result` to swallow.
- **The one surviving silent case is the direct API path**, not the generated one:
  `EcsMaster::events_of` → `EventDispatcher::events` (`event_dispatcher.rs:351-356`) returns an
  empty slice for an unregistered type. No generated system uses it. **That last clause stopped
  being an observation and became a guarantee on 2026-08-30**: ballot AB-5 (ruling **M4a**) turned
  precisely on it. The rejected option would have made the machine event router an *exclusive*
  system, which cannot take `EventReader<E>` and so would have had to read through `events_of` —
  putting a **generated** system on this path for the first time. Choosing `Query::get_mut` is what
  keeps this sentence true.

The `with { lanes, capacity }` group and the generated flat constructor are unaffected — their own
reasons (the two-lane rewrite forces authors to name generated types they never wrote; the
constructor removes *invented names*) stand. **The auto-registration grant now stands on no recorded
ground → ballot AB-1** (below). Rejected (deferred): flat read accessors — N methods per event to
save one word per access.

**Bounds are stated symbolically — `1..=MAX_EVENT_THREADS` and `1..=MAX_EVENT_CAPACITY`, never
numerically.** A numeric literal in this line dates the document to one build of the constant — as
this paragraph itself demonstrated: it read "in the tree today `MAX_EVENT_THREADS` is 64 … that
raise has not landed" until 2026-08-30, when the constant was checked and found to be **65**
(`crates/boyko_ecs/src/ecs/constants.rs:400`, alongside `MAX_EVENT_CAPACITY = 16384`). Ruling **E3**
landed at `01a4436e`; see the measurement note under E3. The symbolic form was correct across the
change, which is the whole point of it. What the parse check enforces is only the constant
**ceiling** — and that half genuinely is a `Result`: `EventConfig::new` returns
`Err(InvalidEventConfig)` for `thread_count == 0 || > MAX_EVENT_THREADS`
(`event_config.rs:41-52`). The binding constraint is the machine-dependent **floor**
`lanes >= worker_count + 1` (`event_dispatcher.rs:258-261`), which is *unrepresentable at parse* —
the worker count is not known until boot. And the runtime path this check was said to "replace" has
no `Result` to replace: a lane index past the end is a release-mode **slice-index panic**
(`event_buffer.rs:243`/`:301`/`:341-346`) — measured in both profiles, see **E4** — with the lane
computed from the dispatcher-wide count (`event_dispatcher.rs:279-282`), a count that is pinned at
`1` in every tree-constructed world. Where the floor lives was **ballot AB-2**, now ruled at **E4**.
Companion pin, to land in `crates/aether_tests`: the parser's
accepted bounds are asserted equal to `boyko_ecs::ecs::constants::{MAX_EVENT_THREADS,
MAX_EVENT_CAPACITY}` — the module is `ecs::constants`, not a crate-root `constants`
(`crates/boyko_ecs/src/ecs/constants.rs`; `boyko_ecs`'s root exports only `pub mod ecs`),
so a constant change cannot silently diverge from the grammar.

> **Ballot AB-1 (open — do not settle by edit).** Auto-registration of Aether events in the
> generated plugin: on what ground?
> Alternatives: **(a)** ratify on ergonomics alone (it removes a boot-time panic the author can
> only fix by writing a line the language already knows how to write); **(b)** **stage** it under
> D4's rule — build the form only when a real in-tree consumer exists — since the silent-failure
> premise that justified it is refuted.
> Blocks: **R3**'s event construct. Reopening C3 is licensed: the premise was refuted by
> measurement, not by preference.

**C4. `system` clauses become four groups (`schedule`/`sets`/`order`/`when`) with NO `with`
wrapper.** The body brace terminates the group list; a wrapper is needed only where groups follow
the body (component, event). Additions: `chain` (exists in `SystemConfig`, has **no `_set` variant**,
so it accepts only a sibling system name — refusal otherwise); `nonsend<T>` / `mut nonsend<T>`
(today the most common render param goes through the verbatim escape, which is never mut-inferred —
the one place a forgotten `mut` is a compile error instead of working code); `or(...)` filter —
**staged**, see D4.

**C5. `each` is a separate construct, not a mode of `system`, and lowers to `iter_mut` by
default.** Separate because only ~2 of 6 real systems are a single loop — implicit iteration cannot
be the default of `system`. `iter_mut` default is a **reversal of the first spec** ("always
`for_each_chunk`"), reversed on evidence: ~37 of 74 real `Query<...>` declarations carry a term the
chunked driver refuses (`Enabled`/`Disabled`, `Changed`/`Added`, `Mut<T>`), and the chunked driver's
`&mut [T]` does not bump change ticks — the most inviting spelling in the language would have been a
tick-blind write silently killing every downstream `Changed<>`. `each soa` / `each par` opt in
explicitly. Binding names derive from the type (`mut Transform` → `transform`): type→name is safe
and collision-checkable; name→type is not (paths and generics cannot be recovered from a lowercase
word). **AIR-10 familiarity axis (audit, no measurement):** `each` is a *false friend in position*.
The word is maximally familiar — `for each` — but in every language that carries it, `each` is a
**statement inside** a routine; in Aether it is a **construct that declares a whole scheduled
system**. A generator that has seen `each` elsewhere may emit it in body position, where it is a
parse error, and may expect a surrounding `system` to hold it. Mitigation is structural rather than
lexical: `each` is legal only at construct position, so the mistake is a refusal, not working code.
Spelling kept.

**C5a. Ballot AB-8 RESOLVED 2026-08-30 — `each par` lowers to `par_iter_mut`; `par_for_each_chunk`
is reachable only through `soa`.** A performance fork, so it is decided here with numbers rather
than sent to the owner.

*Tick behaviour, measured rather than taken from the labels* (both legs run over 2048 rows, well
clear of the 1024-row inline floor, with the writer keyed `.before` the reader):

| driver | rows a `Changed<>` reader sees, of 2048 |
|---|---|
| `par_iter_mut` over `Mut<T>` | **2048** |
| `par_for_each_chunk` over `&mut [T]` | **0** |

*Cost, measured* — 200 000 rows × 200 frames, 8 workers, release, five samples:
`par_iter_mut` is **1.17–1.47×** `par_for_each_chunk` (median ≈1.19), a delta of **0.03–0.07
ns/row**. Absolute ns/row is **not** stable run to run (0.13→0.34 across samples, machine noise);
the *ratio* is, which is why the ratio is what is recorded. Scaled to the machine cost model's own
10 000 rows that delta is **0.3–0.7 µs/frame**, against the **34–40 µs** that ruling D1 measured for
the 5-arm jump table at the same row count — so tick preservation costs **≈1–2 %** of the pass and
is invisible beside the cost D1 already accepted. The measurement carried a falsification guard
(row count and per-row increment count asserted after timing) so a no-op could not have produced it.

*The ruling.* `each par` → `par_iter_mut`. Buying back `Mut<T>`, `Changed`/`Added`, and
`Enabled`/`Disabled` for ~1.5 % is the same trade C5 already made when it reversed the first spec:
~37 of 74 real `Query<…>` declarations carry a term the chunked driver refuses.

*Rejected: `par_for_each_chunk` as the `par` driver.* Price — the most inviting parallel spelling in
the language would be structurally tick-blind, silently killing every downstream `Changed<>`. That
is C5's recorded defect verbatim, and the 0-of-2048 row above is it reproduced.

The three questions that rode on this ballot, answered with it:

1. **Does `soa par` exist? YES — and it is the ONLY route to `par_for_each_chunk`.** `soa` already
   means "chunked, tick-blind, and the diagnostic says so"; letting `par` *alone* select the chunked
   driver would reinstate tick-blindness under a word that does not declare it. Tick-blindness stays
   behind the word that announces it: `each par` → `par_iter_mut`, `each soa par` →
   `par_for_each_chunk`.
2. **Is the batching key author-visible? NO in v1** — refused with a `did-you-mean` at the verbatim
   escape. The ground is measured, not taste: `BatchingStrategy` has **three** fields
   (`batches_per_thread`, `min_batch_size`, `max_batch_size`), so the proposed one-scalar
   `parallel (batch = N)` cannot name the knob it appears to name. *Rejected alternative:* map `N`
   to `batches_per_thread`; its price is that an author writing `batch = 64` and expecting 64 rows
   per chunk gets 64 chunks **per thread** — a silent misreading of their own tuning, and
   `min_batch_size` defaults to `MIN_ARCHETYPE_FOR_PARALLEL` = 1024, which interacts with the inline
   floor in a way one scalar cannot express. Both parallel drivers expose
   `batching_strategy(BatchingStrategy)` as a builder, so the knob stays reachable from the verbatim
   escape and the form is built when a measured in-tree consumer appears — D4's rule, applied.
3. **Do machines and `each` share one driver? They share the LADDER, not the default.** Both select
   tracking by *term* (`Mut<T>` bumps, `&mut T` does not) and both climb the same three rungs —
   `iter_mut` → `soa`/chunked → the `par` variants above. Their **defaults** differ, and each
   default was measured separately: `each` defaults to `iter_mut` (C5), the machine pass to the
   chunked driver (M7/D6). Unifying the defaults would reopen M7/D6's ratified choice, which AB-5
   licensed only for the `get_component_mut` citation and the withdrawn bypass — **not** for the
   driver. It is left standing deliberately, not by oversight.

**C6. `plugin` unchanged, except the emitted `name()` override is dropped.** The trait default
returns the fully-qualified type name, which is strictly more informative in duplicate-plugin
diagnostics; the override was a small regression shipped without a reason.

**C7. `machine` codegen moves to `boyko_macros` as `state_chart!` FIRST; Aether fronts it.**
**[owner]** Reason: `machine` was the ONE construct where Aether itself was the codegen authority,
violating the language's own stated rule ("boyko_macros stays the single codegen authority").
Rejected: a runtime hierarchy (tree walk + a parallel data structure, for a feature whose whole
value is costing nothing at runtime).

**C8. `resource` construct (`resource N { … } with { init | value E }`).** Needed by the machine
design (the focus-broadcast pattern); v1 has no way to declare + insert a resource. `option<entity>`
sugar exists because `Entity` does not derive `Default`, so `with { init }` on a bare entity field
cannot compile — the refusal says so. *Status: used by the accepted machine design; formal
ratification tracked in OPEN.md.*

## Per-entity machines (MACHINES.md)

**M1. State is a byte in a table component — not an archetype, not an enable-bit, not a component
per state.** Numbers: state-as-component = 5 archetypes and a full-row memcpy per transition (~590ns
vs ~90ns replace-in-place) plus a structural-generation bump; state-as-EnableTag migrates nothing
but `Enabled<T>` is not archetypal, which un-compiles the chunked driver. A byte transition is three
in-place stores in the same row visit.

**M2. Timers are a countdown `f32` with `INFINITY` as "no timer", armed by the transition that
enters the leaf; slot count = graph coloring over leaves (both reference charts need ONE slot).**
Rejected: a kernel `Timer` component + tick system (a second data home for the same fact, plus a
`Mut`-ticked timer poisons `Changed<T>` for every consumer every frame); count-up timers (need a
stored threshold too). `INFINITY` makes the per-row tick branchless (`clock -= dt` uses the identity
element) — same trick as the focus broadcast returning `+INFINITY` for "no player".

**M3. Field elision: no synthesized field is emitted unless the chart uses it.** A pure
event-switched machine compiles to `struct M(u8)`. This answers the owner's objection ("автомат без
времени — лишняя информация?") and mirrors the `HAS_HOOKS`/`HAS_REQUIRES` const-gating discipline.

**M4. Events reach machines via an O(events) router system depositing a bit + payload into the
victim row (`Query::get_mut`), not via an `EventReader` in the pass.** Keeps the pass one linear
walk; a dead/stale victim is a silent `None` (safe); the deposit's generation check is the liveness
gate. **[delegated]** (the dead-datum entry this closes is the 2026-08-27 entry in
[`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md), **reading 1**). The participant context
(`entity(EnemyBrain)`) becomes a **debug_assert in the router** — the recorded dead datum gets its
first reader; release cost zero. Scope, stated because the entry's own wording is wider than the
fix: this resolves the datum **for machine `inbox` events only**; for every other event the datum
stays unread, and reading 2 is unfunded.

**M4a. Ballot AB-5 RESOLVED 2026-08-30 — the router uses `Query::get_mut` (option (a)).**
Decided by the orchestrator under the standing perf/architecture rule; every ground below was
measured in this tree, not read off a plan.

*The tick half, measured under an explicit ordering edge* (writer keyed `.before` the reader, so the
test distinguishes "same frame" from "one frame later" — which the pre-existing tests do not):
`Query::get_mut` yielding `Mut<T>` is observed by a `Changed<T>` reader **in the same frame**;
`EcsMaster::get_component_mut` is **not**, even with the reader ordered after it. Both were run as a
temporary probe against `boyko-ecs` and agree with the two APIs' own doc comments
(`iters/query/query.rs` `get_mut` — "the `meta` threaded through the `set_table_mut` calls is what
closes that hole"; `ecs_master/component_api.rs` `get_component_mut` — "§ Inside a `Schedule` frame
(Bug #56 interaction) … observed … on the **following** frame"). The standing in-tree pins are
`tests/ke3_query_random_access.rs::get_mut_stamps_the_changed_tick_from_the_system_meta` and
`tests/phase14b_get_component_mut.rs::changed_query_observes_get_component_mut_write_on_the_following_frame`;
both are green at `01a4436e`.

*The decisive ground is not the tick — it is that option (b) cannot read its own events.*
`get_component_mut` takes `&mut self`, so the only schedulable shape carrying it is
`ExclusiveFunctionSystem`, whose body is `FnMut(&mut EcsMaster)` with — in the module's own words —
"no param tuple, no per-param state" (`system/exclusive_function_system.rs` §Exclusive vs
SystemParam-based systems). A (b) router therefore **cannot take `EventReader<E>`** and must read
through `EcsMaster::events_of`, whose contract is "Returns an **empty slice** if `E` was not
registered or if no events were sent last frame" (`ecs_master/event_api.rs`). That is the one
surviving silent path C3 already identified, and it collapses *unregistered* and *nothing happened*
into the same value — the recorded "NULL read AS AN ANSWER" class. Option (a)'s router is an
ordinary system, takes `EventReader<E>`, and a forgotten registration is a **loud boot panic**
(`system/params/event_reader.rs:320` → `diagnostics.rs:81`). Measured both ways in the probe.
`events_of` also returns the **previous** frame's events, so (b)'s end-to-end latency is **two**
frames — contradicting [`MACHINES.md`](MACHINES.md) §Event routing's own "one frame under
`EveryFrame`".

*Rejected alternative — (b) `get_component_mut`. Its price:* a machine whose `inbox` event is
un-preregistered routes **nothing, silently, forever**; two-frame latency against a documented one;
and the M7 bypass mechanism below, which exists only to repair (b).

*A ground I expected and DID NOT get, recorded so it is not re-argued.* Exclusive systems declare
`Access::universal()` and are single-per-round by construction (`schedule/schedule.rs` — "An
exclusive system, once accepted, blocks everything else in this round … `break`"), so (b) looked
like a per-event-type scheduler barrier. **Measured: it is not.** Against four concurrently
dispatchable worker systems over 20 000 rows — with a control proving the schedule really had
concurrency to lose (1 thread 235–250 µs vs 8 threads 79–133 µs, **2.7–3.2× speedup**) — an
exclusive router cost **0.78–1.00×** of a `Query` router at both 1 and 8 routers, i.e. below the
±20 µs run-to-run noise, and the sign flipped between runs. **Scheduler cost is not a ground for
either option**; the barrier is one extra dispatcher round, not a serialization of the workers.

*The two riding questions, answered with this ruling:*
1. **Does `publish tracked` see the router's deposit in the same frame? YES**, under (a) — measured
   above. Under (b) it did not, which is what made the question live.
2. **Is M7's tick bypass still needed? NO** — it is *replaced by a term choice*, not re-derived.
   Measured: a plain `&mut T` obtained through `Query::get_mut` writes the column and bumps **no**
   tick, while `Mut<T>` through the same call bumps at `this_run`. So the router emits `Mut<M>` when
   `publish tracked` is on and `&mut M` when it is off, and the all-or-nothing M7 demanded falls out
   of the generated term with no bypass mechanism at all. (b) had no such lever:
   `get_component_mut` returns `Mut<T>` unconditionally — the typed path has no untracked variant —
   which is precisely why a bypass had to be invented for it.

Riding lines moved in this same edit: M7 and D6 below, [`MACHINES.md`](MACHINES.md) §Event routing,
and CAMPAIGN R5's Depends cell.

**M5. R-Q: `query<...>` inside guards/enter/exit/tick/commit is a COMPILE ERROR.** Cross-entity
reads go through `res<>` broadcast (1→N), events (N→M, next frame), or `near` (spatial, once it
lands). The naive per-entity design — 10 000 rows each doing a random lookup — does not become slow;
it becomes unwritable.

**M6. Arbitration: FIRST declared wins, and the global machine is ALIGNED to it.** **[owner: align]**
The global machine's last-write-wins was an artifact (one system per route; registration order);
the fix is the R2 route merge, which the both-chains-run defect requires anyway — one piece of work
closes the defect and the divergence.

**M7. Writes in the pass are plain `&mut`; `Changed<Leaf>` is dead by type (the chunked driver
excludes `Mut`/`Ref`) and this is chosen.** Replacement: `entered()` (`prev != leaf`, same frame,
free) enforced by R-ORD (a consumer naming `.entered()`/`.fail()` without `order (after M)` is a
compile error, not a convention). Escape: `with { publish tracked }` switches the pass to
`iter_mut` + `Mut`, paying a per-row fetch; **default off** (ruling D6). Corrections folded in from
the D6 adjudication: tracked bumps ONLY on a leaf-transition commit (never on `clock -= dt`), and
the half-alive untracked state (the router bumped the tick, timer rows did not) is forced to
all-or-nothing.

**The remedy for that last correction is NOT a tick bypass — it is the router's TERM, and this is
the AB-5 amendment** (M4a, resolved 2026-08-30). Measured in this tree: through `Query::get_mut` a
plain `&mut M` writes the column and bumps **no** tick, while `Mut<M>` through the same call bumps
at the system's `this_run`. So the generated router emits `&mut M` when `publish tracked` is off and
`Mut<M>` when it is on, and "all-or-nothing" is a property of the emitted term rather than a
mechanism anyone has to build, test or remember. The bypass was an artifact of the API M4 used to
name: `EcsMaster::get_component_mut` returns `Mut<T>` unconditionally — its typed path has **no**
untracked variant — so under that API a bump could only be undone, never declined. **The bypass is
withdrawn, not re-derived.** D6 rode the same answer and is amended below.

**M8. Owner options (per-machine / per-event, not global policy):** **[owner]**
history is opt-in (`with { history }` +1 byte shallow; `history deep` +8 bytes lifts R-HIST — R-HIST
stands for shallow because restoring a clock-arming leaf without its clock re-fires its enter
effect); the re-deposit policy is declared per inbox event (`inbox (StunHit = max)` with
`replace | max | sum | ignore`, default `replace`) — a commutative choice also makes same-frame
duplicates order-independent, which is exactly where parallel emission loses determinism.

**M9. Default schedule for `on entity` machines is `fixed`.** **[owner]** Physics and the
simulation systems live in Fixed; replay determinism is a stated goal; a variable-dt Main machine
diverges across runs by f32 accumulation. `schedule update` is written explicitly for visual/UI
machines. This ruling also *removes* a question rather than answering it: a machine's domain vs the
app's `EventUpdatePolicy` mismatch (a fixed machine under `EveryFrame`, or an update machine under
`WaitForFixed`) becomes a **build-time diagnostic** instead of a policy the owner has to choose.

## Rulings that closed judge disagreements

*Numbering note: the series runs D1, D2, D4, D5, D6; no D3 appears in the transcribed record and
nothing cites one. The gap is recorded rather than silently carried.*

**D1. Jump-table cost — MEASURED, the "it drowns in memory" position lost.** Shuffled leaves cost
3.4–4.0 ns/row (35–40 µs at 10k rows), invariant across a 200× working-set range (L2 → DRAM) while
the memory baseline grew 4.6× — the cost is additive, the pass is not bandwidth-bound, mechanism
confirmed in the emitted jump table. No mitigation: per-run dispatch is refuted by the probe's own
data (mean run length 1.24 at five states), table sorting is rejected on cost (row_ptr churn), a
side row-order index is banned by Principle 0. Action: the cost-model note states leaf-coherence
sensitivity verbatim; no arm-count warning (coherence, not arm count, sets the price).

**D2. Coarse dirty summaries — structural half only.** A non-atomic `ArchAdded` stamp written only
at cold sites already holding `&mut Archetype` is race-free and lets `Added<C>` consumers skip
clean archetypes wholesale. The per-row value half is REJECTED (an atomic on a line hammered by
every worker — `par_iter` splits one archetype's rows across workers; also breaks the plain-store
tick disjointness invariant); the amortized variant is deferred behind a specified benchmark,
default no-build.

**D4. `or(...)` in the filter grammar — reserved, not built.** v1 refuses `or`/`|` at filter
position with a span diagnostic pointing at the verbatim escape; the form is built only when a real
in-tree consumer exists AND the kernel `Or`-dense fix is green. The kernel fix itself is immediate
and independent (R0). Reasoning: every grammar form is permanent maintenance; the worst outcome on
record is a filter that silently matches nothing.

> ⚠ **R0 removed one of D4's two conjuncts, and this line must not be read as unaffected.
> R0 LANDED 2026-08-29** ([`CAMPAIGN.md`](CAMPAIGN.md) R0; [`KERNEL-BACKLOG.md`](KERNEL-BACKLOG.md)
> KE1). The ground here is a **conjunction** — a real consumer *and* a green kernel fix — so that
> landing retired the second half, and D4's reserve now stands on the consumer clause **alone**.
> That is a
> narrower ground than the one recorded above, and it is not the same argument. Say which clause is
> load-bearing when the reserve is next cited.
>
> This is the **Aether-side** twin of a Gaia ruling whose ground R0 removed *entirely*: the
> generated-code ban on `Or<(Changed<A>, Changed<B>)>` over dense
> ([`../gaia/DECISIONS.md`](../gaia/DECISIONS.md) §UI bindings, item 8). That was ballot **GB-9**,
> ✅ **RULED 2026-08-30: option (b) — the ban is DELETED with a record.**
>
> ⚠ **The ruling turned on THIS line, and the answer went against it.** D4's coupling was the named
> candidate ground for keeping the ban, and it was measured and rejected: **D4 reserves the Aether
> *surface* `or(...)`, while that ban governed *generated code*, and Gaia's own ratified GN2 says
> the baker emits no Rust.** A ban on a shape the generator cannot emit, grounded in a reserve on a
> surface the generator does not write, is a rule with no subject. **D4 itself is untouched** — its
> reserve stands, on the consumer clause alone, exactly as the paragraph above records. What is
> settled is that D4 cannot be lent out as a second document's ground.

**D5. Run-condition combinators fold EAGERLY — no short-circuit.** Not (only) because `run_once`
mutates on evaluation: a condition's change-tick window advances only when it actually runs, so a
short-circuited RHS freezes its `Changed` window and observes a bogus burst later. `CombinedSystem`
runs both children every reached frame, unions access, forwards tick maintenance to both.

Pinned by **five** tests, written before any sugar lands. They are cited by number from CAMPAIGN R1
and from KERNEL-BACKLOG KE5, so the numbering is load-bearing:

1. **Both children evaluate every reached frame** — instrument each side with a call counter and
   assert both advance on a frame where the LHS alone would have decided the result.
2. **`run_once` on the RHS of an `or` whose LHS is true still consumes its run** — the RHS's
   one-shot latch flips even though the combined result never needed it.
3. **No bogus `Changed` burst from the would-be-skipped side on the following frame** — the
   negative half of test 1: with eager folding, the next frame sees no accumulated window.
4. **Combined access is the union of the children's access** — assert on the composed
   `SystemMeta`/access set, not on observed behaviour, so a scheduler that happens to serialize
   cannot hide a missing entry.
5. **Tick maintenance is forwarded to both children** — each child's last-run tick advances on
   every reached frame, which is the mechanism tests 2 and 3 depend on.

**D6. `publish tracked` — the synthesizer's refusal of the Mut-graft stands.** `Ref`/`Mut` are
deliberate non-members of the chunked driver's data trait; one gets either `Changed` or the chunk
driver, never both. See M7 for the three corrections folded in.

> **D6a — amended 2026-08-30 by the AB-5 ruling (M4a).** The refusal itself is *confirmed at
> source*, and more strongly than D6 stated: `ChunkedQueryData`'s own doc names `Ref<'_, T>` /
> `Mut<'_, T>` as "NON-members (deliberate)", and `par_for_each_chunk`'s `// SAFETY:` block states
> that this makes `NEEDS_CHANGE_DETECTION` const-fold to `false`. Measured end to end: a
> `par_for_each_chunk` write over 2048 rows is seen by a `Changed<>` reader on **0** of them, while
> the same write through `par_iter_mut` + `Mut<T>` is seen on **2048**. What changes is only the
> third M7 correction: **the "tick bypass" is withdrawn**, because with the router on
> `Query::get_mut` the tracked/untracked split is carried by the emitted term (`Mut<M>` vs
> `&mut M`) and needs no mechanism. `publish tracked` **does** now see the router's deposit in the
> same frame — that was measured, and it was the open half of AB-5.

## Events (EVENTS.md)

**E1. Parallel emission = relaxed TLS lanes + `send_slice` batching + router combine. Rejected:
outbox as the default** (turns the parallel pass into a serial sweep over the same bytes and
permanently widens the hot row), **chunk-keyed lanes as the default** (a concurrent TLS-keyed
sender of the same event type from another system writes the same lane — sound only under a
cross-system exclusivity invariant the kernel cannot express by default). `send` goes `&self` with
zero new unsafe (the buffer has been interior-mutable since Phase 6); the per-batch single
`fetch_add(n)` removes the one non-scaling cost (a shared counter RMW per send). R-PAR is lifted.

**E2. `ordered` events are OPT-IN (`event E with { ordered }`), built now.** **[owner: build now,
optional]** Implementation: chunk-keyed lanes + a boot-time sender-exclusivity refusal (two parallel
emitters of one ordered type = loud failure, which closes the E1 soundness hole *for the opted-in
types*), serial fallback past 64 chunks. Default events stay on fast TLS lanes with no order
guarantee. Rejected for the opt-in: outbox (full determinism but 1 event/entity/frame and a serial
O(N) sweep costing 4–8× the pass itself).

**E3. `MAX_EVENT_THREADS` 64 → 65.** **[delegated]** Cost is one extra 128-byte lane pair plus
`capacity × size_of::<E>()` per preregistered type, setup only; the alternative (cap workers at 63)
taxes wide machines forever. The `MAX_WORKERS + 1 <= MAX_EVENT_THREADS` const-assert becomes true
and compiled-in.

**✅ E3 HAS LANDED (measured 2026-08-30, at HEAD `01a4436e` — the commit this corpus was written
alongside).** `crates/boyko_ecs/src/ecs/constants.rs:400` reads `pub const MAX_EVENT_THREADS: u32 =
65;` and the `boyko_threadpool::MAX_WORKERS < MAX_EVENT_THREADS as usize` const-assert is compiled
in immediately below it (`MAX_WORKERS = 64`, `crates/boyko_threadpool/src/thread_pool.rs:49`).
`git log -L400,400:crates/boyko_ecs/src/ecs/constants.rs` dates the raise to `01a4436e` itself.
Every corpus sentence saying "65 is a plan value and 64 is the engine's" was true when written and
is false now; the surviving instances are corrected in the same commit as this ruling. **The rest of
KE8 is still unlanded** and the distinction is load-bearing for E5: `EventWriter::send` is still
`&mut self` (`event_writer.rs:110`), `send_slice` does not exist anywhere in `crates/`, and the
send-outside-system `debug_assert` still tests `is_in_system_run()` (`event_writer.rs:112`), not the
re-aimed `current_worker_id() != WORKER_ID_UNATTACHED`.

**E4. The `lanes N` floor is enforced at boot by RAISING, and `lanes N` is redefined from a count to
a MINIMUM.** **[delegated — ballot AB-2]** The effective lane count is
`max(N, worker_count + 1)`, resolved where the worker count is first known; the raise is **reported
once at boot**, not silent.

*Measured ground (2026-08-30, throwaway probe, run and deleted).* The floor has no check anywhere,
and the reason is structural: `current_worker_id_or_dispatcher_lane`
(`crates/boyko_threadpool/src/tls.rs:69-77`) has **three** arms, and on the arm that matters — a
pool worker — it returns that worker's **own id** and never consults `worker_count`, so passing a
correct denominator does not clamp anything. ⚠ *This sentence read "ignores the `worker_count`
argument **entirely**" until 2026-08-30; that is false of the function as a whole — the dispatcher
arm returns `worker_count` itself. The operative claim (no clamp on the worker arm, hence an
out-of-range index) is unaffected, and E5 six pages later states all three arms correctly.*
A violation is
therefore not a `Result` on any path — measured in both profiles on a one-lane buffer:

| profile | `send::<E>(3, …)` on a 1-lane buffer |
|---|---|
| debug | panic — `thread_index 3 >= thread_count 1` (the `debug_assert`) |
| release | panic — `index out of bounds: the len is 1 but the index is 3` |

So the ballot's characterisation is **confirmed**: a release-mode slice-index panic, not a `Result`.
It is a *safe* bounds panic, not UB — which is why raising, not refusing, is affordable.

*What the probe additionally found, and what actually forces this ruling.* The floor is already
violated by the **default** path, before any author writes `lanes N`. `EcsMaster::new()` and
`EcsMaster::with_capacity()` both hard-wire `EventDispatcher::new(1)`
(`ecs_master.rs:429` / `:476`), `default_thread_count` is assigned at exactly that one site and has
no setter, and `preregister_event_default::<E>()` derives its config from it
(`event_api.rs:39`) — so the ergonomic registration allocates **one lane on every machine**.
Measured end-to-end in release: a 4-worker pool + `preregister_event_default` + 64 worker sends →
**3 panicked**, 61 succeeded. The same field is the denominator `send_event` passes
(`event_dispatcher.rs:279-282`), so it is also why `send_event` never reaches the reserved
dispatcher lane in any tree-constructed world: the probe read `default_thread_count == 1` after
`EcsMaster::new()` **and still 1 after `preregister_event::<E>(EventConfig::default_for(5))`**, and
the dispatcher-thread lane index computed from it is `0`. The `send_event` doc's own obligation —
"Phase 9 callers must preregister event types with `EventConfig::default_for(worker_count + 1)`"
(`event_dispatcher.rs:258-261`) — is met by **no caller in the tree**.

*Feasibility.* `App::new()` builds the pool in the constructor (`app.rs:201-202`, `with_pool`) and
`add_plugin` runs `plugin.build(self)` afterwards (`app.rs:550-556`), so the worker count is known
before any plugin preregisters an event. The blocking site is only `EcsMaster::new()`'s hard-wired
`1`; the fix is a dispatcher lane-count setter applied once the pool is known.

*Rejected.* **Boot refusal** — price: one source that is correct on a 4-core laptop becomes
unbootable on a 64-core server, machine-dependently, for a number the author cannot know at
authoring time. That converts a memory overshoot into a deployment failure and is strictly worse.
**Dropping the knob** — technically the cleanest answer, because `lanes` is not an author-meaningful
quantity (its only correct value is machine-derived, so the author has no information the engine
lacks). It is NOT taken here: it deletes a ratified surface from the language, which is a SCOPE call
and the owner's. It is escalated as a recommendation, not exercised.

*Why the raise is reported rather than silent.* The overshoot is real and worth a line of log: at
the worked shape (`capacity 4096`, an 8-byte event) `lanes 32` raised to 65 on a wide machine
allocates ~33 extra lanes × 4096 × 8 B ≈ **1 MB per event type** the author did not ask for.
Silence would hide that; refusal would over-punish it.

**E5. The unattached sender gets a CLAIMED host lane, with one claimer enforced.** **[delegated —
ballot AB-3]** `MAX_EVENT_THREADS` 65 → **66**, the const-assert strengthening to
`MAX_WORKERS + 2 <= MAX_EVENT_THREADS`. **One lane, not two** — the ballot said two on the premise
that E3 was unlanded; E3 has landed (above), so its own const-assert obligation is already paid and
only the claimed host lane remains to buy.

*Measured ground.* First, a correction the ballot needs: it is **not** true that "every OS thread
that is not a pool worker maps to lane 0". `current_worker_id_or_dispatcher_lane` has three arms —
a worker returns its own id, the **dispatcher returns `worker_count`, its own reserved lane**, and
only `WORKER_ID_UNATTACHED` returns `0`. The collision class is threads that never entered an
install scope. (In the tree the dispatcher *also* lands on lane 0, but for the different E4 reason:
the denominator it is handed is `default_thread_count - 1 == 0`.)

Second, the hazard, measured in **release** with an unattached thread and worker 0 sending 4000
events each into one lane. A rendezvous barrier is required to make it reproducible — without one
the unattached thread usually drains all its sends before the pool task is enqueued and 6/6 runs
lose nothing, which is exactly how this hazard stays invisible. With the barrier, **6 of 6 release
runs lost events**: 1891, 2295, 2070, 183, 1473, 1940 out of 8000 (2.3 %–28.7 %). **Every single
send returned `Ok`.** Silent loss, never an error.

*Why "accepted hazard, documented" (option c) is unavailable.* Documentation is an acceptable
disposition for a hazard that is loud or bounded. This one is neither: it is silent data loss **and**
undefined behaviour — two threads writing the same `MaybeUninit<E>` slot through an `UnsafeCell`
with no synchronisation. Decisively, the tree already contains the documentation option's output,
and it is **false**: `send_one`'s `// SAFETY (U4 …)` clause 2 reads *"only the worker pinned to
`thread_index` accesses this UnsafeCell"* (`event_buffer.rs`), which the measurement above falsifies
directly. Choosing (c) would mean ratifying a `// SAFETY:` comment whose stated invariant does not
hold — the precise defect class this campaign exists to remove, and a violation of principle 8.
That comment owes an edit whichever way the rung is built.

*Rejected.* **`Err` on unattached (option b)** — price: it breaks the escape hatch the engine
itself documents. `EventWriter::send`'s doc directs main-thread and FFI callers to
`EcsMaster::events().send_event::<E>(...)` and says "the unattached-thread fallback there routes
safely" (`event_writer.rs:100-103`). Option (b) makes the documented remedy start returning `Err`,
and it buys nothing (a) does not — the host thread has a legitimate need to send.

*The contract that lands with this ruling.* One host lane exists. The first unattached thread to
send claims it by CAS on an owner slot and keeps it. A **second** distinct unattached sender is
refused loudly (`Err`, plus a debug panic), because two unattached senders sharing one lane
reinstate the measured race exactly. This concedes option (b)'s breakage for the second claimant
only — the genuinely unsound case — while leaving the single-main-thread case, which is every
sender in the tree today, working unchanged.

**E6. The `ordered` sender-exclusivity fact is registered by the GENERATED PATH, and the verbatim
escape is REFUSED at the param list.** **[delegated — ballot AB-4]**

*Registrant: a generated-path `register_ordered_emitter` at plugin build.* `register_ordered_emitter`
appears nowhere in `crates/` today (docs-only), so it is new under every alternative — the choice is
not between an existing mechanism and a new one.

*Predicate (the ballot's own complaint was that the refusal named its timing but not its predicate).*
`register_ordered_emitter(event_id, system_id)` appends at plugin build; at build end, any event id
carrying `ordered` whose registered emitter count is **> 1** is a hard boot failure naming both
systems. Boot-time timing was already pinned by three sites; this supplies the missing predicate and
registrant.

*Rejected: `SystemMeta` emit-access.* This is not a cheaper variant of the same thing — it is a
reversal of a ratified decision plus a self-defeating one. Measured: `EventWriter`'s `init_access`
is an **empty body**, carrying the comment *"Phase 12 EW5 / Q2 Option A: events stay OUTSIDE the
conflict graph"* (`event_writer.rs:211-221`). `SystemMeta.access` therefore holds **zero** event
information; there is no event axis to read. Adding one has a price beyond the reversal: `Access` is
a *conflict* structure, so the moment event writes enter it as writes, the scheduler serialises any
two systems emitting the same event type — destroying exactly the parallel emission E1 exists to
enable. The alternative, a deliberately non-conflicting access axis, is a new registry wearing
`SystemMeta`'s name, i.e. option (a) with worse placement.

*Rejected: the `#[event]` macro side.* Structurally impossible, not merely awkward. Exclusivity is a
property of the **emitter set** — which systems send `E` — and the macro sees one type declaration
and zero systems, so it cannot count emitters. (It cannot even read the word today:
`boyko_macros/src/lib.rs:261` is `pub fn event(_args: TokenStream, input: TokenStream)`, which
**forwards** them as `event::expand(_args, input)`; the discard happens one hop later in
`boyko_macros/src/event.rs:9`, whose body never reads `_args`. ⚠ *This citation named only
`lib.rs:261` until 2026-08-30, where a reader checking it sees the arguments being passed along —
the opposite of the claim. The claim itself holds; the site was one file short.*)

*Verbatim escape: param-list refusal.* A hand-written `EventWriter<E>` param for an `ordered` `E` is
refused at system-param init. Implementable at the existing site: `EventWriter::init_state` already
runs per system and already fails loudly for an unregistered event via
`event_not_preregistered_panic::<E>()` (`event_writer.rs:192-197`), and `E::event_id()` plus the
dispatcher are both in hand there. *Rejected: "declared out of contract"* — price: a
documentation-only guard over a soundness property, which is the disposition this very cluster just
measured the worth of (the false `send_one` SAFETY clause under E5; a `debug_assert` that is
debug-only on the param path and absent on `send_event`). Price of the refusal, stated honestly: a
hand-written system cannot emit an `ordered` event even when it is the only emitter; the escape is
to declare the emitter in Aether.

## Spatial (SPATIAL.md)

**P1. Unbounded spatial HASH, not a bounded grid.** Memory O(N) not O(world volume); a teleport
costs one hash; no AABB maintenance pass; the cell-size cross-platform `cbrt` caveat disappears.
**P2. Storage on `ScratchColumn` — zero kernel changes in Phase 1.** **P3. CSR counting-sort with
payload permutation** (permute the 16-byte items, not indices — the query walk then reads
contiguous memory). **P4. Payload id is `u32 EntityId`** (the 24-bit packing idea died on facts:
`POOL_MAX_ROWS` caps pool rows, not ids; generations are unreachable from the chunked driver
anyway — liveness is the deposit-time check, M4). **P5. Parallel build via key-range ownership** —
each worker owns a bucket range, zero atomics, byte-identical to the serial build by construction.
The gate is **`serial_build() == parallel_build(W)` byte-for-byte**, with the **Phase-1 serial path
named as the reference oracle**: the claim is *identical to serial*, so serial has to be one side of
the comparison. `build(1) == build(W)` is kept only as a **smoke** check — it compares two runs of
the same parallel code and is blind to any defect the scatter shares across worker counts. (The
serial path is the oracle; nothing retires it.)

**P6. Own-cell acceptance is MANDATORY in Phase 1**: two walked
cells can hash to one bucket, so a member passing the distance test could be yielded twice (double
damage, nondeterministically rare — the worst fingerprint class on record); accept a member only
when iterating its own cell.

The pin test is written in **two steps**, because a collision fixture that has silently stopped
colliding is a green test measuring nothing:

1. **Assert the collision precondition first** — keyed on the two cells' **hash and the bucket
   count**, never on a census cell size. The table is unbounded, so a later edit to the member set
   can change the bucket count and quietly *un-collide* a hand-picked pair; step 1 turns that into
   a red test instead of a vacuous pass.
2. Only then assert the member is yielded exactly once.

The **structural-collision** form is equally blessed and preferred where available: a test-only hash
that maps the two cells to one bucket by construction, or a one-bucket table, so the precondition
holds by definition rather than by arithmetic luck. **P7. Consumption from a
machine guard is a plain `res<SpatialGrid>` read — no scheduler changes** (the pattern is shipped
twice already: physics reads its grid resource from systems; the boids demo states "no core change
needed"). **P8. The cell-hash + CSR + scatter code is designed as a shared kernel building block**
for three consumers — gameplay now; the physics broadphase converging later (same substrate,
different instance: AABBs, substep cadence, pair output); coarse world-streaming cells later.
Render culling is explicitly excluded: this engine's cull is GPU-resident (host computes frustum
planes and pushes them to the cull shader; HZB is GPU) — a CPU hash accelerates nothing there.

## Ratifications 2026-08-28(2) — the former open constructs

All eight adopted by the owner in one pass; rulings in [`OPEN.md`](OPEN.md), specs in
[`CONSTRUCTS.md`](CONSTRUCTS.md). The notable reasoning:

- **O1/O2/O3 (`set`, `exclusive`, `gpu`)** — pure language surface over existing kernel features;
  the owner's framing is correct that no engine trade-off existed, they had simply never come up.
- **O4 `relation` as a synthesizing construct** over a visibility exception — same principle as
  `tag`/`flag`: the invalid (a hand-written reverse index; desynced cross-references) becomes
  unwritable rather than diagnosable.
- **O6 hierarchical tags** — adopted AFTER the honest accounting: zero runtime difference vs
  manual tags (same archetype, same bits); the value is the ancestor-implication invariant killing
  the silent partial-attach class. Gated: expander-computed archetype ceiling; `#[require]` only
  for `sticky` vocabularies (requires never un-attach — a removable taxonomy strands ancestors).
- **O7 `attributes`** — adopted as a foundation despite the missing `effect` layer, because the
  recompute-from-base pattern is the part hand-rolled buff code reliably gets wrong; `effect`
  stacks on later without rework.
- **O5 `resource`** — ratified; the owner floated a broader "resources" campaign whose scope is to
  be clarified when it opens — the construct does not wait for it.

**Gaia** (owner, same session): the data language — scenes, UI documents, the DataAsset/DataTable
analog — is its own campaign. *Aether is for logic, Gaia is for data.* It inherits the
scene-format pipeline decisions (text → bake → binary, reflection only at build time). Research
commissioned; Gaia gets its own plan directory after the research lands.

## AIR-10 — familiarity / false-friend audit

AIR-10 requires **a DECISIONS line per new keyword** before the R3 surface hardens: a familiar word
carrying unfamiliar semantics is presumed worse than a novel word *until measured*. These lines
**record the audit; they rename nothing.** Ratified spellings stand — AIR-10's own rule is that a
ratified word is not relitigated without the measurement, and the measurement (N generations per
candidate against the trybuild corpus) has **not** been run for any word below. The risk list
AIR-10 names is five words; the other two are audited at their own rulings — `flag` vs `tag` under
**C1**, `each` under **C5**.

- **`set`** — the **strongest false friend in the surface**. Everywhere else `set` is either a
  mutation verb (a setter, `set x = 1`) or a container type (`HashSet`, Python's `set`); in Aether
  it names a **scheduler `SystemSet`** — an ordering bucket, not an assignment and not a collection.
  The predicted generator failure is assignment-shaped (`set foo = …` at construct position) and it
  is *writable-looking*, which is the class AIR-10 exists for. Kept, because it is the kernel's own
  word for the thing (`SystemSet`) and inventing a synonym would split the vocabulary between the
  language and the engine it lowers to. Consequence recorded instead: the construct-position
  refusal for a `set` line that looks like an assignment must be one of R3's seeded goldens.
- **`attributes`** — a **direct false friend**. In Rust, C# and HTML, an "attribute" is *metadata on
  a declaration* (`#[derive(...)]`); Aether's `attributes` is a **gameplay-stat construct** —
  base values plus modifiers with recompute-from-base. Aether even *has* a metadata surface (the
  `with { }` groups), so the wrong reading has somewhere plausible to land. Kept per the O7
  ratification; the mitigation is that the word appears only as a construct head, never inside a
  `with { }` group, so the two never occupy the same position.
- **`relation`** — **friend or false friend depending on the reader's prior**. For a reader with
  flecs exposure it is a true friend (flecs *relationships* are the same idea, and O4 adopts the
  same synthesizing stance). For a reader with a database prior it reads as a relational **table**,
  and for a reader with neither it reads as a plain noun with no operational content. No spelling
  tested better on inspection; kept as the ecosystem-nearest word, with the note that its
  diagnostics should say "reverse index" rather than lean on the noun.

## Sequencing rulings — ballots that order the ladder rather than shape the language

These carry the **ballot's own id** as the entry id, because there is one name for one thing and the
ladder, `AI-ORIENTATION.md` and [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) already cite them that
way. They shape no construct, so they belong to no construct series.

**AB-9. `boyko_reflect` is MERGED FORWARD as its own rung before R8; AIR-06(b) is NOT descoped; and
engine-crate reflection opt-in is a FOURTH precondition that no option on the ballot named.**
**[delegated — ballot AB-9, ruled 2026-08-30]** The question was sequencing: does R8 wait on the
`feat/reflection` merge, is AIR-06(b) (the project-schema dump) descoped to its reflection-free
halves, or is the merge pulled forward.

*Measured, and the ballot's own figures were stale in both directions.* Merge-base
`5ec1699f`; `git rev-list --left-right --count feat/aether-v2...feat/reflection` → **15 ahead, 20
BEHIND**. "18 commits ahead" was taken against a different base, and the *behind* count — the one
the ballot never carried — is the one that grows. `git merge-tree --write-tree feat/aether-v2
feat/reflection` (**no merge performed**) → **7 conflicted paths**: 4 docs (`FEATURE_MAP.md`,
`SYSTEMS.md`, `OPEN-QUESTIONS.md`, `ru/OPEN-QUESTIONS.md`), 2 trybuild `.stderr`
(`unknown_key_rejected.stderr` a content conflict; `on_despawn_rejected.stderr` a **modify/delete** —
stages 1 and 3 only, because `01a4436e` deleted it here while reflection modified it, so it needs a
decision rather than a re-bless), and **one source file**, `crates/boyko_macros/src/component.rs`, at
**2 hunks / 23 lines**. Payload: **124 files, +44 956/−214, 3 new workspace members**
(`boyko_reflect`, `reflect_fixture`, `reflect_dogfood`).

*The ballot's load-bearing claim is false as it would be used.* AIR-06's oracle clause 2 demands that
a component appear **in the dump**, and "the dump" is (b)'s *project* schema; a *grammar* manifest
generated from parser dispatch tables cannot contain a workspace component. Descoping (b) does not
leave the clause satisfied — it **removes its subject**. Reflection-free, the clause is satisfiable
only by declaring the probe in an `aether!` block, which greens the gate over a dump covering **0 of
the 138 `#[derive(…Component…)]` sites in the engine crates** — measured over `crates/*/src` with a
multi-line-aware scan: `boyko_ecs` 43, `boyko_ui` 36, `boyko_render` 21, `boyko_physics` 15,
`boyko_scene` 14, `boyko_demo` 8, `boyko_input` 1 = **138**; **185** across all of `crates/*/src`
(the balance is `boyko_macros` 29, `aether_lang` 16, `aether` 2 — tooling, not engine). Measured
alongside it: **zero production `aether!` declaration blocks exist**.

*And the merge alone does not fix it, which is the fourth item.* `boyko_reflect::registry::type_info_of`
returns `None` for a component without `#[component(reflect)]` — reflection is **opt-in per
component**. On `feat/reflection`, `#[component(reflect` appears at **50 attribute sites, and not
one is in an engine crate** — `reflect_fixture` 47 and `reflect_dogfood` 3, both test/dogfood
crates. Re-measured 2026-08-30 in `D:/wt/reflect` with
`grep -rnE '^\s*#\[component\(reflect' --include=*.rs crates/`, which requires the attribute to
open a code line.

⚠ **This row read *107 attribute sites across 34 `.rs` files* until 2026-08-30, and that number was
the TEXTUAL grep**: `grep -rn '#\[component(reflect' --include=*.rs crates/` returns **106**, of
which **47 are prose** — `//`, `///` and `//!` lines *describing* the attribute. That is where the
old breakdown's `boyko_reflect` 4 and `boyko_macros` 3 came from: those crates talk about the
attribute, they do not carry it. **The conclusion is unchanged and in fact strengthened** — the
engine-crate count was zero on the inflated figure and is still zero on the strict one.

⚠⚠ **Note where the old figure sat.** The very next sentence warns that *a looser grep contradicts
this and is wrong* — and correctly excludes two `.toml` hits. Its own number was that same looser
grep, one hop further in, **inside `.rs`**. A caveat about over-counting, written beside an
over-count, is the shape to watch for: the warning made the sentence read as if it had already been
audited. The `.toml` half of the warning stands and is kept below.

`git grep component(reflect)` over `crates/*` also
returns `boyko_render/Cargo.toml:43` and `boyko_scene/Cargo.toml:51` — both **comments describing the
opt-ins that have not arrived**, not opt-in sites. Anyone re-checking this claim must scan for the
attribute in `.rs`, not for the string.

*Rejected, with the price.* **R8 waits on the merge** — the gate goes green over a dump covering 0 of
138 engine components while the branch keeps diverging, and the divergence concentrates in
`crates/boyko_macros/src/component.rs`, the one file both campaigns edit. **Descope AIR-06(b)** — the
same zero coverage **plus** the loss of the oracle's only workspace-facing clause: a gate that cannot
fail.

*Not decided here, on purpose.* **Which** route the engine-crate opt-in takes — a sweep marking
components `#[component(reflect)]`, or a derive that opts in by default — is **R8's own design pass**:
it carries a per-component static plus `OnceLock` cost that has to be measured, not argued.

*Riding line moved with this ruling, and flagged because it tightens an oracle:* AIR-06's red-first
clause in [`AI-ORIENTATION.md`](AI-ORIENTATION.md) reads "a hand-written `#[derive(Component)]`
component **from an engine crate**" as of 2026-08-30; it read "a probe-crate component" before, and
that form is satisfiable by a route measuring nothing. *What it unblocks:* **R8** carries no open
ballot of its own any more. That is **not** buildable — the ruling *adds* two pieces of work (the
merge rung, and the opt-in route), and R8's `Depends` cell still names **R3**, which carries four
open owner ballots (AB-1, AB-6, AB-11, AB-13).

## Doc repairs bundled with the campaign

- The `g6` bench header claimed "boyko ~5%/11% slower" and cited a nonexistent path; the measured
  tables say parity with the sign unstable across runs (one run 8% faster at default flags). Fixed.
- The dead-datum OPEN-QUESTIONS entry is resolved (M4: router debug_assert) — for machine `inbox`
  events only; see M4 for the scope qualifier.
- A new OPEN-QUESTIONS entry records the both-chains-run machine defect and its scheduled fix (R2).
