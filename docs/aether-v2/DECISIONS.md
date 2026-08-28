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

**C2. `bundle` gets two forms — positional (primary) and named.** The derive already accepts tuple
structs; v1 was stricter than the derive for no reason, and the field name in a named bundle is a
pure repeat of the type. Named stays for `..base` struct-update and named missing-field errors.
Rejected: order-free positional construction via generic slot traits — it converts a missing or
duplicated slot from a compile error into a runtime panic (compile-but-lie, the recorded lesson).

**C3. `event` gains `with { lanes N capacity N }` + plugin auto-registration + a generated flat
constructor `E::new(...)` in source field order.** Reasons: forgetting registration today fails
*silently on both ends* (reads return an empty slice; the canonical `let _ = send(...)` swallows
`EventNotRegistered`); the two-lane rewrite forces authors to name generated types they never wrote.
Bounds (1..=64, 1..=16384) are checked at parse on the author's token instead of a runtime Result.
Rejected (deferred): flat read accessors — N methods per event to save one word per access, while
the constructor removes *invented names*.

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
word).

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
gate. **[delegated F6]** The participant context (`entity(EnemyBrain)`) becomes a **debug_assert in
the router** — the recorded dead datum gets its first reader; release cost zero.

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
do not) is forced to all-or-nothing via tick bypass.

**M8. Owner options (per-machine / per-event, not global policy):** **[owner]**
history is opt-in (`with { history }` +1 byte shallow; `history deep` +8 bytes lifts R-HIST — R-HIST
stands for shallow because restoring a clock-arming leaf without its clock re-fires its enter
effect); the re-deposit policy is declared per inbox event (`inbox (StunHit = max)` with
`replace | max | sum | ignore`, default `replace`) — a commutative choice also makes same-frame
duplicates order-independent, which is exactly where parallel emission loses determinism.

**M9. Default schedule for `on entity` machines is `fixed`.** **[owner]** Physics and the
simulation systems live in Fixed; replay determinism is a stated goal; a variable-dt Main machine
diverges across runs by f32 accumulation. `schedule update` is written explicitly for visual/UI
machines. F5 dissolves into F4: a machine's domain vs the app's `EventUpdatePolicy` mismatch
(fixed machine under `EveryFrame`, or update machine under `WaitForFixed`) becomes a build-time
diagnostic instead of a policy question.

## Rulings that closed judge disagreements

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
runs both children every reached frame, unions access, forwards tick maintenance to both. Pinned by
tests before any sugar lands.

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
each worker owns a bucket range, zero atomics, byte-identical to the serial build by construction,
gated by `build(1) == build(W)`. **P6. Own-cell acceptance is MANDATORY in Phase 1**: two walked
cells can hash to one bucket, so a member passing the distance test could be yielded twice (double
damage, nondeterministically rare — the worst fingerprint class on record); accept a member only
when iterating its own cell, plus a pin test with a forced collision. **P7. Consumption from a
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

## Doc repairs bundled with the campaign

- The `g6` bench header claimed "boyko ~5%/11% slower" and cited a nonexistent path; the measured
  tables say parity with the sign unstable across runs (one run 8% faster at default flags). Fixed.
- The dead-datum OPEN-QUESTIONS entry is resolved (F6: router debug_assert).
- A new OPEN-QUESTIONS entry records the both-chains-run machine defect and its scheduled fix (R2).
