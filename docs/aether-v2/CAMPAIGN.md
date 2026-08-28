# Aether v2 — campaign plan

The redesign of the Aether language surface plus the engine work it stands on, as decided with the
owner across 2026-08-27..28. This directory is the **decision record and work plan**; the shipped v1
surface it revises is catalogued in [`../AETHER-V1-SURFACE-REVIEW.md`](../AETHER-V1-SURFACE-REVIEW.md).
Rationale for every call — what was chosen, what was rejected, and why — lives in
[`DECISIONS.md`](DECISIONS.md); a plan file states *what*, the decision log states *why*.

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
parked; the blocker is policy — is a material shader a source or an asset); the scene asset format
and world streaming (`scene` narrows to authored scenes; the world lives in a baked binary format
with zero runtime reflection). Physics and render culling deliberately do **not** adopt the spatial
index (the render cull is GPU-resident by design; the physics broadphase has a different contract —
see DECISIONS.md §Spatial).

## Rung ladder

Rungs are ordered by dependency, not preference. Each rung names its oracle — the thing that must
be red before the fix and green after, per the standing "a gate that cannot fail is not a gate"
lesson.

| Rung | What | Depends on | Oracle |
|---|---|---|---|
| **R0** | Kernel bug fixes: `Or` dense blindness (`impl_or_filter_tuple` declares no dense plumbing while the AND tuple does; ~117 production `Or<(` sites sit above it) | — | a **red-first** test pinning `Or<(Changed<TableC>, Changed<DenseC>)>` against a hand oracle |
| **R1** | Kernel enablers, batch 1: `Entities` param; `Query::{get, get_mut, single, contains, first}`; `Option<Res<R>>`/`Option<ResMut<R>>`; run-condition combinators (eager fold); `on_despawn` derive key; `CommandQueue::{mark, rewind}`; structural `ArchAdded` stamp; `MAX_EVENT_THREADS` → 65 + const-assert | — | unit tests per item; the combinator fold semantics pinned by the 5 tests in DECISIONS.md §D5 |
| **R2** | `state_chart!` moves the machine flattening into `boyko_macros`; per-leaf **route merge** (fixes the both-chains-run defect *and* aligns arbitration to first-declared-wins); reachability/dead-state analysis | — | a red-first test: two events on one leaf in one frame must run exactly ONE exit/action/enter chain |
| **R3** | Aether v2 construct rewrite (CONSTRUCTS.md) — component groups, `tag`/`flag`, bundle forms, event `with { lanes, capacity }` + auto-registration + flat ctor, system groups + `nonsend` + `chain`, `each`, `resource`, `plugin` `name()` removal | R1 (combinators, on_despawn), R2 (machine fronts `state_chart!`) | token pins + trybuild goldens, same three-lane gate discipline as v1 |
| **R4** | Parallel event emission (EVENTS.md): `send(&self)`, `send_slice`, `par_for_each_chunk_entities`, router combine, `ordered` opt-in | R1 (65 lanes) | `build(1) == build(W)`-style determinism gates; loom/stress story for the lane path |
| **R5** | Per-entity machines (MACHINES.md): `machine … on entity`, compiled timers, field elision | R1 (`Entities`, `Query::get`), R2 (chart core), R4 only for `parallel` | behaviour tests over both reference scenarios (enemy AI, ability) + size const-asserts + the D1 cost-model note |
| **R6** | `boyko_spatial` Phases 1–3 (SPATIAL.md); Aether `near` (Phase 4) | Phase 4 needs R5 | zero-alloc row, `build(1) == build(W)`, the own-cell double-visit pin test |
| **R7** | Ratification of the open constructs (OPEN.md): `set`, `exclusive`, `gpu`, `relation`, hierarchical tags, `attributes`, event payload binding | owner | — |

Rungs R0–R2 are pure engine work and can proceed in parallel worktrees (one worktree per system —
standing owner rule). R3 is the language pivot; everything after it layers on.

## Engine-layer split

Where every addition lands. "Zero when unused" is the admission bar: a program not using the
feature pays nothing.

| Layer | Additions |
|---|---|
| **`boyko_ecs` (kernel)** | `Entities` param · `Query` random access + `first` · `Option<Res>` params · combinators · `ArchAdded` stamp · `CommandQueue::mark/rewind` · event `send(&self)` + `send_slice` + 65 lanes · `par_for_each_chunk_entities` · (deferred: `VmColumn` promotion) |
| **`boyko_macros`** | `state_chart!` (the machine codegen authority) · `on_despawn` key unlocked · (existing derives untouched otherwise) |
| **`aether_lang` / `aether`** | the v2 construct surface; per-entity `machine` front-end; `each`; `resource`; all refusals (R-Q, R-ELSE, R-ORD, R-HIST, R-CLOCK, R-ARITY, or-reserve); the domain-mismatch diagnostic; the cost-model note |
| **new crate `boyko_spatial`** | the hash grid, queries, census; Phase 1 needs **zero kernel changes** |
| **shared kernel building block (goal)** | cell-hash + CSR counting-sort + key-range scatter, designed for three consumers (gameplay now; physics broadphase convergence and coarse streaming cells later) so the structure is configured, not re-implemented |

## Discipline

- Every rung commits atomically when its oracle is green; broken intermediate states are never
  committed (compiler + red-first tests are the oracle).
- All refusal messages land with trybuild goldens; every generated surface gets a token pin.
- Doc claims about engine behaviour cite the symbol, not just a line number — exact `:N` anchors
  rot (188 of 302 were waived in the last census) and are used only where freshly verified.
