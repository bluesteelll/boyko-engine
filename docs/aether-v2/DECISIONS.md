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
  empty slice for an unregistered type. No generated system uses it.

The `with { lanes, capacity }` group and the generated flat constructor are unaffected — their own
reasons (the two-lane rewrite forces authors to name generated types they never wrote; the
constructor removes *invented names*) stand. **The auto-registration grant now stands on no recorded
ground → ballot AB-1** (below). Rejected (deferred): flat read accessors — N methods per event to
save one word per access.

**Bounds are stated symbolically — `1..=MAX_EVENT_THREADS` and `1..=MAX_EVENT_CAPACITY`, never
numerically.** A numeric literal in this line dates the document to one build of the constant. **In
the tree today `MAX_EVENT_THREADS` is 64** (`crates/boyko_ecs/src/ecs/constants.rs`, alongside
`MAX_EVENT_CAPACITY = 16384`); ruling **E3** below raises it to 65, and that raise has **not
landed**: it is carried as a KERNEL-BACKLOG **KE8** work item, so 65 is a plan value and 64 is the
engine's. The symbolic form is correct under both. What the parse check enforces is only the constant **ceiling**. The
binding constraint is the machine-dependent **floor** `lanes >= worker_count + 1`
(`event_dispatcher.rs:258-261`), which is *unrepresentable at parse* — the worker count is not known
until boot. And the runtime path this check was said to "replace" has no `Result` to replace: a lane
index past the end is a release-mode **slice-index panic** (`event_buffer.rs:243`/`:301`/`:341-346`),
with the lane computed from the dispatcher-wide count (`event_dispatcher.rs:279-282`). Where the
floor lives is **ballot AB-2**. Companion pin, to land in `crates/aether_tests`: the parser's
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
victim row (`get_component_mut`), not via an `EventReader` in the pass.** Keeps the pass one linear
walk; a dead/stale victim is a silent `None` (safe); the deposit's generation check is the liveness
gate. **[delegated]** (the dead-datum entry this closes is the 2026-08-27 entry in
[`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md), **reading 1**). The participant context
(`entity(EnemyBrain)`) becomes a **debug_assert in the router** — the recorded dead datum gets its
first reader; release cost zero. Scope, stated because the entry's own wording is wider than the
fix: this resolves the datum **for machine `inbox` events only**; for every other event the datum
stays unread, and reading 2 is unfunded.

> **Ballot AB-5 (open — do not settle by edit).** The router's random-access mechanism, and the
> tick visibility that follows from it. Alternatives: **(a)** amend M4 to `Query::get_mut`;
> **(b)** keep `get_component_mut`. The two APIs **stamp different ticks**, so the answer decides
> two dependent lines that must move in the same commit: does `publish tracked` (M7 / D6) then see
> the router's deposit **in the same frame**, and is M7's tick bypass still needed at all — M7's
> remedy was derived from apply-window stamping, which is the behaviour of the API being replaced.
> Blocks: **R5**, and the matching cells in [`MACHINES.md`](MACHINES.md) §Event routing + cost
> model and CAMPAIGN R5's Depends. Editing MACHINES.md alone would create an EN-internal drift
> pair against this ruling.

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
the current half-alive untracked state (the router's `get_component_mut` bumps the tick, timer rows
do not) is forced to all-or-nothing via tick bypass. ⚠ **Both the `get_component_mut` citation and
the tick-bypass remedy are on ballot AB-5** (see M4): the remedy was derived from the apply-window
stamping of the API named here, so if AB-5 selects `Query::get_mut` this correction has to be
re-derived, not merely re-cited. D6 rides the same answer.

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

## Doc repairs bundled with the campaign

- The `g6` bench header claimed "boyko ~5%/11% slower" and cited a nonexistent path; the measured
  tables say parity with the sign unstable across runs (one run 8% faster at default flags). Fixed.
- The dead-datum OPEN-QUESTIONS entry is resolved (M4: router debug_assert) — for machine `inbox`
  events only; see M4 for the scope qualifier.
- A new OPEN-QUESTIONS entry records the both-chains-run machine defect and its scheduled fix (R2).
