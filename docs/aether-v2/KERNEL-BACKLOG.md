# Kernel backlog — engine-side work, grouped by crate

Every engine addition the campaign needs, with the zero-cost-when-unused status and the oracle that
gates it. Aether-side work lives in [`CONSTRUCTS.md`](CONSTRUCTS.md) / [`MACHINES.md`](MACHINES.md).

**Id namespace (renamed 2026-08-29).** The bare `E#` / `M#` series collided with the `M#` machine
rulings in [`DECISIONS.md`](DECISIONS.md) and read as unqualified everywhere they were cited. This
file's ids are now `KE#` (`boyko_ecs`) and `KM#` (`boyko_macros`); the numbers are unchanged, so
`E11` → `KE11`. **The rename is COMPLETE across the corpus — and the corpus reaches OUTSIDE this
directory, which is what the previous version of this line got wrong.** The citing files,
**recomputed 2026-08-29** over `docs/` with `rg -n "\b(KE|KM)[0-9]+\b" docs/` (run as the PowerShell
`Select-String` equivalent on this checkout): in this directory, [`OPEN.md`](OPEN.md),
[`CAMPAIGN.md`](CAMPAIGN.md), [`DECISIONS.md`](DECISIONS.md), [`CONSTRUCTS.md`](CONSTRUCTS.md),
[`EVENTS.md`](EVENTS.md) and [`SPATIAL.md`](SPATIAL.md); outside it,
[`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) with its `docs/ru/` twin and
[`docs/gaia/DECISIONS.md`](../gaia/DECISIONS.md). **`SPATIAL.md` and `docs/gaia/DECISIONS.md` were
both absent from the previous enumeration**, and the second is why "across the directory" was the
wrong scope — `docs/gaia/` is a different corpus citing this backlog's ids.

**The list above is a dated snapshot, not the claim** — an enumeration of citers rots the moment a
sibling adds one, which is the defect it just carried. What is checkable without it, and what a
reader should run instead of trusting the list: every `\b(KE|KM)[0-9]+\b` hit over `docs/` resolves
to a row in this file, and the retired bare `E#`/`M#` form appears nowhere as a citation of it.
(Both halves are recipes to run, not results asserted here; the first was run to produce the list,
the second was not.) No waiver is carried for the retired bare form: a waiver that licenses known
rot is a measured failure class here (the anchor census abdicated 188 of 302 sites under one).
Note that a bare
`E1`/`E2`/`E3` inside [`DECISIONS.md`](DECISIONS.md) is that file's **own** event ruling and is not
a citation of this file.

## `boyko_ecs`

| # | Item | Why | Zero when unused | Oracle |
|---|---|---|---|---|
| KE1 | **Fix `Or` dense blindness** — `impl_or_filter_tuple` (iters/query/filter.rs) forwards none of the dense plumbing the AND tuple has; a dense arm inside `Or<...>` silently matches nothing. Blast radius **re-measured 2026-08-29: 112 textual `Or<(` matches**, not the "~117 production sites" this row carried — 4 production type positions (`boyko_ui` layout.rs:88/:118, text/measure.rs:61; `boyko_render` light_system.rs:989), 6 doc-comment mentions, 48 kernel-internal lines across four files (filter.rs 34, par_chunk.rs 7, filter_enable.rs 4, state.rs 3), 54 test/bench/fixture sites. Producing command and the ledger row: [`OPEN.md`](OPEN.md) | R0. The priority rests on the **silent-wrong-answer mechanism** — the query compiles, runs, and returns a plausible wrong set — **not** on blast radius, which the recount shrinks to 4 production positions. Precondition for any `or(...)` grammar | n/a (bug fix) | **red-first** test: `Or<(Changed<TableC>, Changed<DenseC>)>` vs a hand oracle |
| KE2 | **`Entities<'w>` read-only SystemParam** with `get(EntityId) -> Option<Entity>`; carrier pointer is `*const InlandStore`, NOT `*const EntityMaster` (type-enforced narrowness, the `EntityCounter` precedent) | machines need `me`; iterators yield `EntityId`, events carry `Entity` with generation; today the only resolver sits behind `&EcsMaster` (exclusive) | yes — pure type addition | unit tests + a debug assert mirroring the writer guard |
| KE3 | **`Query::{get, get_mut, single, single_mut, contains, first}`** — provenance is **SPLIT, not "port from `QueryView`, which has them all"**: `get`/`get_mut`/`single`/`single_mut` do port from `QueryView` (`crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs`:641/:767/:578/:601), but **`contains` and `first` do NOT exist on `QueryView` — they are NEW**. `get_mut` stamps the changed tick via the `SystemMeta` the `Query` param uniquely holds | "attack reads the TARGET's Health" today needs an exclusive system or an O(N) id scan. **First consumer: the R5 machine event router** | yes | **SPLIT oracle.** Ported half: port `QueryView`'s tests for the four ported methods. New half (no test to port): `contains` — including the alive-but-unmatched entity case; `first` — pinning a **STATED** order, which `QueryView` does not define, so the order is a decision this row makes rather than inherits |
| KE4 | **`Option<Res<R>>` / `Option<ResMut<R>>` params** — the null branch returns `None` instead of the `#[cold]` missing-resource panic; access declaration unchanged | unlocks the deferred `resource_exists` / `on_event` conditions; the one place the engine answers optionality with a panic | yes | unit test |
| KE5 | **Run-condition combinators** `CombinedSystem<A, B, OP>` + `Not` — **EAGER fold** (ruling D5: a skipped RHS freezes its change-tick window), access union, tick maintenance forwarded to BOTH children | `when (A or B)` / `unless C` | yes | the five D5 pin tests **cited by number**, per DECISIONS §D5: (1) both children evaluate every reached frame; (2) `run_once` on the RHS of an `or` whose LHS is true still consumes its run; (3) no bogus `Changed` burst from the would-be-skipped side on the following frame; (4) combined access = union; (5) tick maintenance forwarded to both |
| KE6 | **`ArchAdded` structural stamp** (ruling D2) — non-atomic, written only at cold sites already holding `&mut Archetype`; `Added<C>` consumers skip clean archetypes wholesale. The per-row value half is REJECTED; the amortized variant sits behind the specified bench | spawn-burst reactivity | yes (NCD const-fold) | stamp unit tests + the check-ticks clamp |
| KE7 | **`CommandQueue::{mark, rewind}`** — cursor save + drop-glue replay over the byte arena | all-or-nothing for the structural half; must DOCUMENT that `reserve_entity` ids are not reclaimed | yes | unit test incl. the reserved-id caveat |
| KE8 | **Event lanes**: `send`/`send_default` → `&self`; new `send_slice`; `MAX_EVENT_THREADS` → 65 + `MAX_WORKERS + 1 <= MAX_EVENT_THREADS` const-assert; debug-assert re-aim + TLS depth counter; `ordered` chunk-keyed lanes + boot-time sender-exclusivity refusal | [`EVENTS.md`](EVENTS.md) | yes (default path unchanged for serial users) | loom/stress on the lane path, **re-scoped**: the stress set must include a `WORKER_ID_UNATTACHED` thread sending concurrently with worker 0 — as scoped today the debug assert excludes that class *by construction*, so the run cannot fail on it. Determinism gate for `ordered`, **re-axed at [`CAMPAIGN.md`](CAMPAIGN.md) R4 the same way R6 was**: the property is *the parallel path emits the stream the serial path emits* — a fixed scenario's event stream from the parallel pass compared **against the serial-emission reference stream** for W ∈ {1, 2, N}. `build(1) == build(W)` is demoted to a **smoke** check (it compares two runs of the same code against each other and so cannot see a lane-assignment defect that is stable across runs — exactly the defect class `ordered` exists to exclude), and the run must **report per-worker send counts** (AIR-12's counts-not-exit-code rule) so a silently-serial run is red, not green. **Plus a two-emitter red fixture** for the sender-exclusivity refusal. What the contract itself says is on ballots **AB-3**/**AB-4** below; the fixtures are writable now |
| KE9 | **`par_for_each_chunk_entities`** — the parallel twin does not exist (the serial one does; `par_for_each_chunk` has no entity slice). Signature pinned: `par_for_each_chunk_entities<Func>(&mut self, f: Func, batching: BatchingStrategy) where Func: for<'c> Fn(&'c [EntityId], D::ChunkItem<'c>) + Send + Sync` (entity slice from :390; batching parameter and the bound from :652; `BatchingStrategy::default()` exists) | parallel machine passes; spatial Phase 3 | yes | new-unsafe review gate (entity-slice aliasing) + the serial-vs-parallel byte oracle its consumers use. Whether the batching value is author-visible (`parallel (batch = N)`) or always the default is ballot **AB-8** below — the signature above is pinned either way |
| KE10 | **`FLAGS_DIRECT`** — the enable-bit twin of `REQUIRES_DIRECT`: per-component initial flag states applied on attach (no transitive closure — a bit pulls no other bits; no ctor — a bit, not bytes) | the component `flags (…)` group | yes (`HAS_FLAGS` gate, same discipline as `HAS_REQUIRES`) | attach-path unit tests |
| KE11 | **Refuse (or filter) `#[require]` of an id whose storage kind owns no per-archetype pool — BOTH poolless kinds, Bitset AND Dense** (the row previously said "a bitset tag", which is half the class). Two sites reach an unfiltered pool lookup for such an id, and they are **two DIFFERENT calls** — (1) `BundleColumnCache::resolve_required_missing` (`crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs`) via `component_pools().pool_id_for(entry.component_id).expect("invariant: the archetype was expanded with every required id …")`, and (2) the `has_requires` required-ctor pass inside `migrate_entity_insert` (`crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs`) via `component_pools_mut().get_pool_mut(req_id).expect("invariant: target hosts every required id (expanded archetype)")`. Neither screens the storage kind first, unlike the sibling loop in `BundleColumnCache::resolve_and_cache`, which diverts a `StorageKind::Dense` id to `DENSE_POOL_SENTINEL` before resolving. Read as a guaranteed panic, **unconfirmed by a test** | `requires` of a `flag` — and, on the same mechanism, `requires` of a dense-storage component — is a boot-time crash carrying a misleading invariant message | n/a | write the red tests FIRST, **parameterised over both poolless kinds** — this is still a code-reading claim, and the tests demonstrate the panic regardless of which disposition wins. Disposition (refuse / filter / dense construct-and-commit route) is ballot **AB-6** below |
| KE12 | *(deferred, measure first)* `VmColumn` promotion to pub; the g6-style A/B of `iter` vs `for_each_chunk` on the machine-pass shape; the D2 value-half bench | — | — | criterion |

## `boyko_macros`

| # | Item | Why | Oracle |
|---|---|---|---|
| KM1 | **`state_chart!`** — the machine flattening moves here (leaf enum, LCA chains, innermost-wins, **per-leaf route merge**). The merge fixes the both-chains-run defect and lands first-declared-wins arbitration (owner: align) | Aether stops being a codegen authority; hand-written Rust gains hierarchical charts | red-first: two events, one leaf, one frame → exactly ONE exit/action/enter chain; reachability/dead-state analysis tests |
| KM2 | **Unlock `on_despawn`** in `#[derive(Component)]` — the derive still refuses the key ("deferred to Phase 14b") while the kernel hook field exists and fires | the one push-mandatory case (despawn has no pull carrier) is unreachable from the derive | derive test + hook-fire test |
| KM3 | `#[event]`: the `ordered` registration surface; the flat-constructor emission if it lands macro-side rather than Aether-side | [`EVENTS.md`](EVENTS.md), [`CONSTRUCTS.md`](CONSTRUCTS.md) | token pins. **No oracle here may be premised on "registration forgotten → silence"**: that class cannot be made red — both generated ends are a loud init-time panic (`event_not_preregistered_panic`; DECISIONS C3), so a test written against the silence would pass vacuously. What registers the `ordered` fact is ballot **AB-4** below |

## `aether_lang` / `aether`

The v2 construct rewrite (R3), the per-entity `machine` front-end (R5), `each`, `resource`, all
refusals, the domain-mismatch diagnostic, the cost-model note, and the two authored-scene emission
fixes (extras-bundle collapse; `spawn_batch` grouping) — specified in
[`CONSTRUCTS.md`](CONSTRUCTS.md) and [`MACHINES.md`](MACHINES.md). Gate discipline is v1's three
lanes: token pins in `aether_lang`, trybuild goldens, and the behaviour lane in `aether_tests`
against the real engine.

## New crate `boyko_spatial`

Phases 1–4 in [`SPATIAL.md`](SPATIAL.md). Phase 1 needs **zero kernel changes**; Phase 3 needs KE9.
The cell-hash + CSR + key-range-scatter internals are written as reusable building blocks (three
intended consumers — gameplay, physics-broadphase convergence, streaming cells).

## Substrate outside the workspace

- **`boyko_reflect`** — consumer: **AIR-06(b)** (the project-schema dump), rung R8. Status:
  **external — it is NOT a workspace member**; it lives on the unmerged branch `feat/reflection`
  (18 commits ahead). Recording this dependency is not gated on any ballot; the *sequencing*
  (R8 waits on the merge / AIR-06(b) descoped to its reflection-free halves / the merge pulled
  forward) is ballot **AB-9** below. Everywhere else the crate is cited — e.g. AI-ORIENTATION N1 —
  it is a **methodological precedent only**: the technique is portable, the dependency is not
  taken.

## Open ballots raised against this backlog

These are OPEN. Nothing below is decided here; each row states the question, the alternatives on
the table, and what it blocks. Ballot bodies live in [`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md).

| Ballot | Question | Alternatives | Blocks |
|---|---|---|---|
| **AB-3** | the unattached-thread lane contract behind KE8's `&self` send | (a) per-thread claimed host lanes (`MAX_EVENT_THREADS` is **64** in the tree; 65 is KE8's unlanded plan value, so while KE8 is unlanded this option raises `64 → 66+` — **two** lanes, one for KE8's own const-assert and one for the claimed host lane — with one-claimer enforcement); (b) `Err` on an unattached sender — breaks today's silent main-thread senders; (c) accepted hazard, documented as such | R4 (`&self` send). The re-scoped stress fixture in KE8 lands regardless |
| **AB-4** | what REGISTERS the `ordered` sender-exclusivity fact (KE8, KM3), plus the verbatim-escape disposition | registrant: a generated-path `register_ordered_emitter` / `SystemMeta` emit-access / the `#[event]` macro side. Verbatim escape: param-list refusal vs declared out-of-contract | R4 |
| **AB-6** | disposition of `#[require]` over a storage kind that owns no per-archetype pool (KE11) | (a) parse refusal — ⚠ this **narrows the ratified `storage = table \| dense` × `requires` surface**; (b) a dense required-ctor (construct-and-commit) route; (c) leave it KNOWN-OPEN and document the hook workaround | KE11's disposition and R3's wording. KE11's red tests land under all three |
| **AB-8** | the `each par` / machine-parallel surface behind KE9 | driver: `par_iter_mut` (per-row, ticks preserved — the standing recommendation, per C5's own tick rationale) vs `par_for_each_chunk` (chunked, tick-excluding); whether `soa par` exists at all; whether the batching key is author-visible (`parallel (batch = N)`) or always `BatchingStrategy::default()`; whether machines and `each` share one driver | R3 / R5 |
| **AB-9** | `boyko_reflect` sequencing for AIR-06(b) | R8 waits on the merge / AIR-06(b) descoped to the reflection-free halves (which still satisfy its oracle) / the merge pulled forward into the campaign | R8 |
