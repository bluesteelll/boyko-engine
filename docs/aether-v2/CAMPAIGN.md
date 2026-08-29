# Aether v2 — campaign plan

The redesign of the Aether language surface plus the engine work it stands on, as decided with the
owner across 2026-08-27..28. This directory is the **decision record and work plan**; the shipped v1
surface it revises is catalogued in [`../AETHER-V1-SURFACE-REVIEW.md`](../AETHER-V1-SURFACE-REVIEW.md).
Rationale for every call — what was chosen, what was rejected, and why — lives in
[`DECISIONS.md`](DECISIONS.md); a plan file states *what*, the decision log states *why*.

**Start here after 2026-08-29:**
[`../AETHER-GAIA-REVISION-2026-08-29.md`](../AETHER-GAIA-REVISION-2026-08-29.md) — the revision
record shared with the Gaia campaign: what the engine refuted, which gates were struck as
unfalsifiable, the per-file change summary, and the work order (what is buildable now, and which
ballot blocks each remaining rung).

**File map** (a plan is always split across files — the monolith failure class is measured):

| File | Holds |
|---|---|
| [`CAMPAIGN.md`](CAMPAIGN.md) | this file — scope, rung ladder, engine-layer split, gates |
| [`DECISIONS.md`](DECISIONS.md) | the full decision log with rejected alternatives |
| [`CONSTRUCTS.md`](CONSTRUCTS.md) | the v2 construct surface as a **delta** over v1 |
| [`MACHINES.md`](MACHINES.md) | per-entity state machines + timers (`machine … on entity`) |
| [`SPATIAL.md`](SPATIAL.md) | the `boyko_spatial` crate specification |
| [`EVENTS.md`](EVENTS.md) | parallel event emission + opt-in ordered events |
| [`KERNEL-BACKLOG.md`](KERNEL-BACKLOG.md) | engine-side work items grouped by crate |
| [`OPEN.md`](OPEN.md) | constructs and questions NOT yet ratified by the owner |

## Scope

**In:** the nine v1 constructs reshaped (groups in `with { }`, `tag`/`flag` split, two bundle forms,
event lane registration, system clause groups, the `each` construct, the `resource` construct);
per-entity state machines with compiled timers; the spatial index; parallel event emission; the
kernel enablers all of that needs.

**Out (separate campaigns, own decision records):** programmable/graph materials (`material` is
parked; the blocker is policy — is a material shader a source or an asset); **Gaia** — the data
language (owner, 2026-08-28: *Aether is for logic, Gaia is for data*) covering scenes, UI
documents and the DataAsset/DataTable analog, absorbing the scene-format campaign's pipeline
(own text → build-time bake with reflection → binary, zero runtime reflection). Physics and render
culling deliberately do **not** adopt the spatial index (the render cull is GPU-resident by
design; the physics broadphase has a different contract — see DECISIONS.md §Spatial).

## Rung ladder

Rungs are ordered by dependency, not preference. Each rung names its oracle — the thing that must
be red before the fix and green after, per the standing "a gate that cannot fail is not a gate"
lesson.

| Rung | What | Depends on | Oracle |
|---|---|---|---|
| **R0** | Kernel bug fixes: `Or` dense blindness (`impl_or_filter_tuple` declares no dense plumbing while the AND tuple does). R0's priority rests on the **silent-wrong-answer mechanism** — a dense arm inside `Or<...>` is never true and nothing reports it — **not** on blast radius: the measured census is **112 textual `Or<(` matches**, of which **4 are production type positions** (`boyko_ui` `layout.rs:88`/`:118`, `text/measure.rs:61`; `boyko_render` `light_system.rs:989`), 6 are doc-comment mentions, 48 are kernel-internal lines across four files (`filter.rs` 34, `par_chunk.rs` 7, `filter_enable.rs` 4, `state.rs` 3), and 54 are test/bench/fixture sites | — | a **red-first** test pinning `Or<(Changed<TableC>, Changed<DenseC>)>` against a hand oracle |
| **R1** | Kernel enablers, batch 1: `Entities` param; `Query::{get, get_mut, single, single_mut, contains, first}`; `Option<Res<R>>`/`Option<ResMut<R>>`; run-condition combinators (eager fold); `on_despawn` derive key; `CommandQueue::{mark, rewind}`; structural `ArchAdded` stamp; `MAX_EVENT_THREADS` → 65 + const-assert; **KE10** `FLAGS_DIRECT`; **KE11** the `#[require]` refusal for a poolless required id (**red test written FIRST**, parameterised over **both** poolless kinds — bitset AND dense; see KERNEL-BACKLOG KE11) | — | unit tests per item; the combinator fold semantics pinned by the five tests **D5(1)–D5(5)** enumerated in DECISIONS.md §D5 |
| **R2** | `state_chart!` moves the machine flattening into `boyko_macros`; per-leaf **route merge** (fixes the both-chains-run defect *and* aligns arbitration to first-declared-wins); reachability/dead-state analysis | — | a red-first test: two events on one leaf in one frame must run exactly ONE exit/action/enter chain |
| **R3** | Aether v2 construct rewrite (CONSTRUCTS.md) — component groups, `tag`/`flag`, bundle forms, event `with { lanes, capacity }` + auto-registration + flat ctor, system groups + `nonsend` + `chain`, `each`, `resource`, `plugin` `name()` removal. **Deliverable: commit probes C and E as trybuild fixtures** under `crates/aether_tests/tests/ui/` — probe C = a broken construct followed by `scen lab {}`; probe E = one block carrying two independent block-level defects; each pins error **COUNT == 2** | R1 (combinators, on_despawn, `FLAGS_DIRECT`), R2 (machine fronts `state_chart!`) | token pins + trybuild goldens, same three-lane gate discipline as v1; **+ AIR-10: a DECISIONS familiarity/false-friend line per risk-list keyword, present before the R3 surface hardens** |
| **R4** | Parallel event emission (EVENTS.md): `send(&self)`, `send_slice`, `par_for_each_chunk_entities`, router combine, `ordered` opt-in | R1 (65 lanes) | **re-axed the same way R6 was.** The property the `ordered` gate must test is *the parallel path emits the stream the serial path emits* — a fixed scenario's event stream from the parallel pass compared **against the serial-emission reference stream** for W ∈ {1, 2, N}. `build(1) == build(W)` is demoted to a **smoke** check: it compares two runs of the same code against each other and so cannot see a lane-assignment defect that is stable across runs, which is exactly the defect class `ordered` exists to exclude. The run must **report per-worker send counts** (AIR-12's counts-not-exit-code rule) so a silently-serial run is red, not green. Plus the loom/stress story for the lane path, re-scoped per KERNEL-BACKLOG KE8 to include a `WORKER_ID_UNATTACHED` sender concurrent with worker 0 |
| **R5** | Per-entity machines (MACHINES.md): `machine … on entity`, compiled timers, field elision | R1 (`Entities`, `Query::get`) — **⚠ blocked on ballot AB-5**, see §Open ballots on this ladder; R2 (chart core), R4 only for `parallel` | behaviour tests over both reference scenarios (enemy AI, ability) + size const-asserts + the D1 cost-model note |
| **R6** | `boyko_spatial` Phases 1–3 (SPATIAL.md); Aether `near` (Phase 4) | Phase 4 needs R5 | zero-alloc row; **`serial_build() == parallel_build(W)` byte-for-byte for W ∈ {1, 2, N}** against the Phase-1 serial path as the reference oracle (`build(1) == build(W)` is demoted to a **smoke** check — it compares two runs of the same code and cannot see a scatter defect). Fixture preconditions stated in the test: the largest matched archetype exceeds `MIN_ARCHETYPE_FOR_PARALLEL` (cited **by symbol**), W is pinned > 1 with an explicit pool installed, and a **per-worker touch counter is reported** (AIR-12's counts-not-exit-code rule) so a silently-serial run is red, not green. Own-cell double-visit pin test written **two-step** — the collision precondition asserted first, before the double-visit assertion — or with a structural collision (test hash / one-bucket table) |
| **R7** | ~~Ratification of the open constructs~~ **DONE 2026-08-28 — all eight adopted** (OPEN.md holds the rulings; specs in CONSTRUCTS.md). Implementation folds into R3: `set`, `exclusive`, `gpu`, `relation`, `resource`, `attributes`, payload binding, hierarchical `tags` | — | the same R3 gate lanes |
| **R8** | AI-orientation tooling ([`AI-ORIENTATION.md`](AI-ORIENTATION.md)): `check_block` + `aether check --format=json` (AIR-01), the `AE####` code registry + census (AIR-02/03), the generated grammar manifest + project schema dump (AIR-06 — see footnote †), `aetherfmt` + the **permutation-convergence gate** + comment survival (AIR-07/14), the compact generated surface file (AIR-11), recorded loop budgets (AIR-12). AIR-04/05/13 (accumulating block checks, resync fix, v1→v2 migration diagnostics) fold into **R3**; Gaia items bind through AIR-16/17 | R3 | every AIR oracle demonstrably red before its fix. **The probe blocks for `AD1` and `AD2` (blocks E and C — the defects renumbered from the old `G4`/`G5` on 2026-08-29) exist only in the 2026-08-28 session transcript and are NOT committed**, so no oracle may name them as artifacts: until the R3 fixtures land, every cell citing probe C or E must inline the block source or name the fixture path it will become |

† **AIR-06(b)** (project schema dump) depends on `boyko_reflect`, which is **not a workspace member**
and lives on the unmerged branch `feat/reflection` (18 commits ahead). Recording that dependency is
determinate and is **not** gated on ballot AB-9; only the sequencing (wait for the merge / descope
AIR-06(b) to the reflection-free halves / pull the merge forward) is on the ballot.

Rungs R0–R2 are pure engine work and can proceed in parallel worktrees (one worktree per system —
standing owner rule). R3 is the language pivot; everything after it layers on.

### Open ballots on this ladder

These are **not** decided here. Each is an owner ballot; the rung it blocks may not be declared done
while it stands open. This is the **complete** `AB-*` set — a partial list here is the same failure
as no list, because a rung whose row names no ballot reads as unblocked. Bodies live in
[`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29 (the consolidated ballot list); the Gaia
`F`/`GB` series lives in [`../gaia/CAMPAIGN.md`](../gaia/CAMPAIGN.md).

| Ballot | Question | Alternatives | Blocks |
|---|---|---|---|
| **AB-1** | Event auto-registration: ratify on ergonomics alone, or stage it. ⚠ **reopens the ratified C3 grant, licensed by measurement** — the grant's recorded ground was that the unregistered case "fails silently on both ends", and both generated ends are in fact a loud init-time panic | (a) ratify as specified; (b) STAGE under D4 until an in-tree consumer exists | **R3** (event construct) |
| **AB-2** | Where the `lanes N` FLOOR lives: parse can enforce only the constant ceiling, while the binding constraint `lanes >= worker_count + 1` is machine-dependent and unrepresentable at parse — and the path called "replaced" is a release-mode slice-index panic, not a `Result` | minimum silently raised at boot / boot refusal / drop the knob. The worked example changes under all three | **R3**, **R4** |
| **AB-3** | The unattached-thread lane contract. Measured: every OS thread that is not a pool worker maps to lane 0 — worker 0's lane — guarded by a `debug_assert` on the param path and by nothing on `send_event` | per-thread claimed host lanes (`MAX_EVENT_THREADS` is **64** today; 65 is KE8's unlanded plan value, so the raise is `64 → 66+` — two lanes while KE8 is unlanded — with one claimer enforced) / `Err` on unattached (breaks silent main-thread senders) / accepted hazard, documented | **R4** (`send(&self)`) |
| **AB-4** | `ordered` sender exclusivity: what REGISTERS the fact, plus the disposition of the verbatim escape | generated-path `register_ordered_emitter` / `SystemMeta` emit-access / the `#[event]` macro side; escape = param-list refusal vs declared out of contract | **R4** |
| **AB-5** | The machine event router's random-access mechanism, and the tick visibility that follows from it | (a) amend M4 from `get_component_mut` to `Query::get_mut` — then R5's Depends cell must cite `Query::get_mut` (BLOCKING for the router) instead of `Query::get`, a citation-precision fix since R1 already carries `get_mut`; (b) keep `get_component_mut`. Riding on the same answer: does `publish tracked` then see the deposit in the same frame, and is M7's tick bypass still needed — the two APIs stamp different ticks, and M7's remedy was derived from apply-window stamping | **R5** (and the M4 / M7 / D6 lines in DECISIONS.md, MACHINES.md §Event routing + cost model — they must move together or the record drifts against itself) |
| **AB-6** | `requires` of a dense-storage component. ⚠ the refusal option **narrows the ratified `storage = table\|dense` × `requires` surface** | parse refusal / a dense required-ctor route / known-open plus a hook workaround. KE11's red tests land now under any disposition — they demonstrate the panic either way | **R3**'s wording (and KE11's disposition, not KE11's tests) |
| **AB-7** | R-DENSE. ⚠ **re-grounds a ratified refusal** | unconditional, with a driver-independent ground that must be ESTABLISHED rather than asserted / lifted by `publish tracked` | **R5** |
| **AB-8** | The `each par` driver — two kernel drivers exist with opposite tick behaviour (`par_iter_mut`, per-row, ticks preserved; `par_for_each_chunk`, chunked, tick-excluding). On the same ballot: whether `soa par` exists at all, whether the batching key is author-visible (`parallel (batch = N)`), and whether machines and `each` share one driver | `par_iter_mut` (the recommendation, per C5's own rationale) / `par_for_each_chunk`; author-visible batch vs always-default | **R3**, **R5** |
| **AB-9** | `boyko_reflect` sequencing for AIR-06(b): not a workspace member, on the unmerged `feat/reflection` (18 commits ahead) — see the † footnote above | R8 waits on the merge / AIR-06(b) descoped to its reflection-free halves (which still satisfy its oracle) / the merge is pulled forward | **R8** |
| **AB-10** | AIR-10 residue | (a) is the measurement **script** required when the audit branch is taken, or does a DECISIONS audit line suffice; (b) is the joint Aether+Gaia vocabulary widening ratified — ⚠ (b) is a **scope change to a ratified requirement** | **R3**'s keyword surface only; the audit lines are written now, so R3 is not stalled on it |
| **AB-11** | `with`/`without` over a `flag` — both silent today (`with Flag` matches nothing, `without Flag` excludes nothing). ⚠ **adds a refusal where v1 documents non-refusal** | parse refusal with a did-you-mean at `enabled`/`disabled` (the recommendation — it dissolves the whole class) / forbidden-form-only, caught at doc generation | **R3**'s filter goldens |
| **AB-12** | *(a query, not a values call)* do the two dropped measured defects — the old `G1`/`G2` of the AI-orientation defect series — exist in the owner's session record? | (a) they exist → return as `AD5`/`AD6` with repros; (b) they never existed → the renumbered record stands | nothing (non-blocking) |
| **AB-13** | The flag-initial value vocabulary, and disambiguation across all three `on` positions (`machine … on entity`, `on E => T`, `flags (X = on)`). ⚠ the `flags → initial` rename **touches a ratified keyword** | `on\|off` reserved vs contextual; and: [`../gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) §Tier 3 **withdrew** the `flags → initial` rename (ground: `initial` is already the machine's initial-state keyword, so the rename recreates the defect it fixes) — does that withdrawal stand, or is a different rename wanted? | **R3** |

**Roll-up by rung** — the number a rung's own row does not show: **R0/R1/R2** — none, buildable now.
**R3** — AB-1, AB-2, AB-6, AB-8, AB-11, AB-13 (six; plus AB-10 on its keyword surface alone).
**R4** — AB-2, AB-3, AB-4. **R5** — AB-5, AB-7, AB-8. **R6** — none of its own; it depends on R5.
**R7** — ratification is done, but its implementation folds into R3, so it inherits R3's list.
**R8** — AB-9.

**Two blockers on this ladder are NOT ballots and must not be waited on as such:** K8 (a bundle
carrying a `link Entity` field has no honest spawn spelling) and K9 (the kernel `relates`/`related`
macro demands a private collection field plus `retain_empty`, inexpressible on a pub-field Aether
group) are architecture gaps routed to **R3's own design pass**, not to the owner — R3's `bundle`
and `relation` constructs may not be declared done while they stand. Both are recorded in
[`../gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) §Part E.

Two Gaia-series ballots also name rungs on **this** ladder. Both are **OPEN**; only their bodies are
housed on the Gaia side (see [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md)) — neither is decided:
**GB-9** must be decided *before* R0 lands (R0 is what makes the `Or`-over-dense emission ban's
original ground false), and **GB-8** names R8 as a candidate owner for the corpus-wide link/id
census.

## Engine-layer split

Where every addition lands. "Zero when unused" is the admission bar: a program not using the
feature pays nothing.

| Layer | Additions |
|---|---|
| **`boyko_ecs` (kernel)** | `Entities` param · `Query` random access + `first` · `Option<Res>` params · combinators · `ArchAdded` stamp · `CommandQueue::mark/rewind` · event `send(&self)` + `send_slice` + 65 lanes · `par_for_each_chunk_entities` · `FLAGS_DIRECT` · the `#[require]` refusal for a poolless required id (bitset **and** dense) · (deferred: `VmColumn` promotion) |
| **`boyko_macros`** | `state_chart!` (the machine codegen authority) · `on_despawn` key unlocked · (existing derives untouched otherwise) |
| **`aether_lang` / `aether`** | the v2 construct surface; per-entity `machine` front-end; `each`; `resource`; all refusals (R-Q, R-ELSE, R-ORD, R-HIST, R-CLOCK, R-ARITY, or-reserve, **R-DENSE**, **R-PAR** — which stands until R4 lifts it, and so needs its golden meanwhile); the domain-mismatch diagnostic; the cost-model note |
| **new crate `boyko_spatial`** | the hash grid, queries, census; Phase 1 needs **zero kernel changes** |
| **shared kernel building block (goal)** | cell-hash + CSR counting-sort + key-range scatter, designed for three consumers (gameplay now; physics broadphase convergence and coarse streaming cells later) so the structure is configured, not re-implemented |

## AI-oriented (owner directive, 2026-08-28)

Both **Aether and Gaia treat AI authorship as a first-class consumer**, alongside the human
programmer. What that means concretely is the subject of a commissioned research supplement, but
the axis is binding on every design decision from here on: a regular, low-ambiguity grammar an
agent can generate without trial-and-error; **canonical formatting** so machine edits produce
semantic diffs, not noise; **structured, machine-readable diagnostics** (the rustc
`--error-format=json` precedent) so an agent can self-correct from the compiler/baker output;
**stable edit-target identities** (a patch addresses a node by id, never by line position);
**schema introspection** (an agent can ask what components/fields/constructs exist instead of
guessing); and validation loops cheap enough to iterate against. Much of Aether's existing
discipline (spans on user tokens, did-you-mean, one-error-per-fault recovery, golden-pinned
messages) already serves this axis — the supplement's job is to inventory what is covered and
close what is not.

## Discipline

- Every rung commits atomically when its oracle is green; broken intermediate states are never
  committed (compiler + red-first tests are the oracle).
- All refusal messages land with trybuild goldens; every generated surface gets a token pin.
- Doc claims about engine behaviour cite the symbol, not just a line number — exact `:N` anchors
  rot (188 of 302 were waived in the last census) and are used only where freshly verified.
