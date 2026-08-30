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

## Relation to Gaia — separate scopes, ONE body of work

"Out of scope" above means *this ladder does not build it*; it does **not** mean a separate project.
*Aether is the language for **logic**, Gaia the language for **data*** — the split is by subject
matter, and the two share a kernel, one AI-orientation requirement set, one diagnostic envelope and
one id namespace. Concretely, and each checkable today: [`gaia/DECISIONS.md`](../gaia/DECISIONS.md)
cites **this campaign's `KE#` ids** directly (and `KERNEL-BACKLOG.md`'s header records that the
`E#` → `KE#` rename was first scoped wrongly *because* it treated `docs/gaia/` as someone else's
corpus); [`AI-ORIENTATION.md`](AI-ORIENTATION.md) — a file in **this** directory — carries
**AIR-08** and **AIR-16** with `Gaia spec` in their own Rung column, **AIR-17** with `Gaia G0`, and
**AIR-09** with `R8 / Gaia tooling`; the `AE####` / `GA####` code registries are one discipline
minted by AIR-02; and two ballots of the **Gaia** `GB` series name rungs on **this** ladder
(**GB-9** → R0, **GB-8** → R8, both listed below). ✅ **Both RULED 2026-08-30** — and **GB-8's
placement did not survive its own ruling**: the id census widens corpus-wide at **Gaia G0**, and
only the *link* census lands on this ladder at R8. GB-9's "→ R0" was a decision deadline, never a
build dependency, and it is now closed.

**And the campaigns find each other's defects.** Three defects in *shipped* code — the `Or` dense
arm (**KE1**, rung R0), `any_changed_since` over a dense/bitset bind source (Gaia **GK-2**), and the
load path's dropped `requires` closure (Gaia **F4(ii)**) — are **one mechanism**: code that resolves
a per-archetype pool without screening the storage kind, and so is blind to the kinds that own no
pool. Two of the three were raised by the *data* campaign, in kernel and UI code *this* campaign
owns. ⚠ **`with`/`without` over a `flag` (AB-11) was listed here as the fourth member; R0's census
measured it and it is NOT** — same *class* (a presence test resolved through a mechanism the storage
kind does not participate in) but a **different mechanism**: the failing code is
`With::matches_component_set`'s fall-through to `mask.contains(state.id)` in the **leaves**, not a
`get_pool` resolve, and it needs no `Or` at all — a bare `Query<&P, With<Flag>>` is already wrong.
R0's fix demonstrably does not reach it (`crates/boyko_ecs/tests/ab11_flag_filter_polarity.rs`, run
with the fix in the tree). The count of the pool-resolution class is **four blind sites**, and AB-11
is not among them — see [`KERNEL-BACKLOG.md`](KERNEL-BACKLOG.md) §Census. The joint account, the
couplings table, and the owed enumeration of the class live in
[`../AETHER-GAIA-REVISION-2026-08-29.md`](../AETHER-GAIA-REVISION-2026-08-29.md) §One mechanism,
three shipped defects and §Aether and Gaia are one body of work.

## Rung ladder

Rungs are ordered by dependency, not preference. Each rung names its oracle — the thing that must
be red before the fix and green after, per the standing "a gate that cannot fail is not a gate"
lesson.

| Rung | What | Depends on | Oracle |
|---|---|---|---|
| **R0** | ✅ **LANDED 2026-08-29.** `Or` now forwards `HAS_DENSE` (OR-fold) and `resolve_dense` (per-arm) and deliberately does NOT forward `HAS_DENSE_INCLUDE` / `dense_include_candidates` — a disjunct bounds no candidate set, so forwarding those two would have traded the fix for a new silent-wrong-answer. Oracle: `crates/boyko_ecs/tests/ke1_or_dense_blindness.rs`, 8 hand-oracle tests over 8 arm shapes; **7 observed red before the fix, all 8 green after**. The defect had **two** polarities, not the one this row assumed: a dense `Without` arm answers from the same NULL store and **excludes nothing**. Also landed, as R0's census half: the class table in [`KERNEL-BACKLOG.md`](KERNEL-BACKLOG.md) §Census (42 production pool-resolution sites classified; **4 blind**, 2 of them owned by KE11/AB-6 and 2 by Gaia GK-2 — the second GK-2 site, `get_component_changed_tick`, is **not named in GK-2's own row**), plus `#[ignore]`d red tests for the GK-2 pair and for ballot **AB-11**'s flag-filter polarity. The census also raised **one NEW unowned kernel bug, KE13** — `QueryView::get`/`get_mut` never call `F::filter_fetch` and resolve `resolve_dense` for `D` only, so a **dense** `With`/`Without` filter is applied by `iter()` and silently ignored by `get()`; `get`'s own doc comment asserts "there is no silent-ignore path". Measured red, needs a rung. Kernel bug fixes: `Or` dense blindness (`impl_or_filter_tuple` declares no dense plumbing while the AND tuple does). R0's priority rests on the **silent-wrong-answer mechanism** — a dense arm inside `Or<...>` is never true and nothing reports it — **not** on blast radius: the measured census is **112 textual `Or<(` matches**, of which **4 are production type positions** (`boyko_ui` `layout.rs:88`/`:118`, `text/measure.rs:61`; `boyko_render` `light_system.rs:989`), 6 are doc-comment mentions, 48 are kernel-internal lines across four files (`filter.rs` 34, `par_chunk.rs` 7, `filter_enable.rs` 4, `state.rs` 3), and 54 are test/bench/fixture sites. ⚠ **Landing R0 falsified the recorded ground of a Gaia ruling** — the generated-code ban on `Or<(Changed<A>, Changed<B>)>` over dense, whose disposition was ballot **GB-9**, ✅ **RULED 2026-08-30: the ban is DELETED with a record** ([`../gaia/DECISIONS.md`](../gaia/DECISIONS.md) §UI bindings item 8). The reading of "decide before R0 lands" that this section argued for — a staleness deadline on the DECISION, not a build blocker on the RUNG — was the correct one, and R0 was rightly not stalled | — | a **red-first** test pinning `Or<(Changed<TableC>, Changed<DenseC>)>` against a hand oracle |
| **R1** ✅ **LANDED 2026-08-30 @`01a4436e`** — every item below is in the tree, each with its own oracle file (`ke2_entities_param.rs`, `ke3_query_random_access.rs`, `ke4_optional_resource_params.rs`, `ke5_condition_combinators.rs`, `ke6_arch_added_stamp.rs`, `ke10_flags_direct_on_attach.rs`, `km2_on_despawn_derive.rs`), and **`MAX_EVENT_THREADS` is now 65 in `constants.rs:400`** — ballot **AB-3**'s ruling raises it again to 66. **KE11 landed as MEASUREMENT only**, per ballot AB-6: its four `should_panic` tests confirm the defect at both sites for both poolless kinds and fix nothing. ⚠ *This row read as work-to-do until 2026-08-30 — an implementer reading it would have redone landed work, and the lane ruling's claim that "the surviving instances are corrected in the same commit" did not reach it.* | Kernel enablers, batch 1: `Entities` param; `Query::{get, get_mut, single, single_mut, contains, first}`; `Option<Res<R>>`/`Option<ResMut<R>>`; run-condition combinators (eager fold); `on_despawn` derive key; `CommandQueue::{mark, rewind}`; structural `ArchAdded` stamp; `MAX_EVENT_THREADS` → 65 + const-assert; **KE10** `FLAGS_DIRECT`; **KE11** the `#[require]` refusal for a poolless required id (**red test written FIRST**, parameterised over **both** poolless kinds — bitset AND dense; see KERNEL-BACKLOG KE11) | — | unit tests per item; the combinator fold semantics pinned by the five tests **D5(1)–D5(5)** enumerated in DECISIONS.md §D5 |
| **R2** ✅ **DONE 2026-08-30** | `state_chart!` moved the machine flattening into `boyko_macros`; per-leaf **route merge** (fixed the both-chains-run defect *and* aligned arbitration to first-declared-wins); reachability/dead-state analysis as a hard error | — | red-first `aether_tests/tests/r2_chart_arbitration.rs` — MEASURED failing at two exits / two actions / two enters and settling on the LAST-declared route, green afterwards at exactly one chain on the FIRST. One flattening implementation, not two: Aether's `machine` lowers to a `state_chart!` invocation. 8 of the 9 `machine_*` trybuild goldens passed the move byte-identical, which is the evidence that the extra macro layer moves no caret |
| **R3** | Aether v2 construct rewrite (CONSTRUCTS.md) — component groups, `tag`/`flag`, bundle forms, event `with { lanes, capacity }` + auto-registration + flat ctor, system groups + `nonsend` + `chain`, `each`, `resource`, `plugin` `name()` removal. **Deliverable: commit probes C and E as trybuild fixtures** under `crates/aether_tests/tests/ui/` — probe C = a broken construct followed by `scen lab {}`; probe E = one block carrying two independent block-level defects; each pins error **COUNT == 2** | R1 (combinators, on_despawn, `FLAGS_DIRECT`), R2 (machine fronts `state_chart!`) | token pins + trybuild goldens, same three-lane gate discipline as v1; **+ AIR-10: a DECISIONS familiarity/false-friend line per risk-list keyword, present before the R3 surface hardens** |
| **R4** | Parallel event emission (EVENTS.md): `send(&self)`, `send_slice`, `par_for_each_chunk_entities`, router combine, `ordered` opt-in | R1 (65 lanes) | **re-axed the same way R6 was.** The property the `ordered` gate must test is *the parallel path emits the stream the serial path emits* — a fixed scenario's event stream from the parallel pass compared **against the serial-emission reference stream** for W ∈ {1, 2, N}. `build(1) == build(W)` is demoted to a **smoke** check: it compares two runs of the same code against each other and so cannot see a lane-assignment defect that is stable across runs, which is exactly the defect class `ordered` exists to exclude. The run must **report per-worker send counts** (AIR-12's counts-not-exit-code rule) so a silently-serial run is red, not green. Plus the loom/stress story for the lane path, re-scoped per KERNEL-BACKLOG KE8 to include a `WORKER_ID_UNATTACHED` sender concurrent with worker 0 |
| **R5** | Per-entity machines (MACHINES.md): `machine … on entity`, compiled timers, field elision | R1 (`Entities`, **`Query::get_mut`** — the router's mechanism, AB-5 **RESOLVED** 2026-08-30 to option (a), DECISIONS **M4a**; `get_mut` already landed at `01a4436e`, so this is a citation, not a new dependency); R2 (chart core), R4 only for `parallel`. **Still blocked on ballot AB-7** (R-DENSE's ground — owner's; its candidate premise was measured and refuted, see MACHINES.md §Refusals) | behaviour tests over both reference scenarios (enemy AI, ability) + size const-asserts + the D1 cost-model note |
| **R6** | `boyko_spatial` Phases 1–3 (SPATIAL.md); Aether `near` (Phase 4) | Phase 4 needs R5 | zero-alloc row; **`serial_build() == parallel_build(W)` byte-for-byte for W ∈ {1, 2, N}** against the Phase-1 serial path as the reference oracle (`build(1) == build(W)` is demoted to a **smoke** check — it compares two runs of the same code and cannot see a scatter defect). Fixture preconditions stated in the test: the largest matched archetype exceeds `MIN_ARCHETYPE_FOR_PARALLEL` (cited **by symbol**), W is pinned > 1 with an explicit pool installed, and a **per-worker touch counter is reported** (AIR-12's counts-not-exit-code rule) so a silently-serial run is red, not green. Own-cell double-visit pin test written **two-step** — the collision precondition asserted first, before the double-visit assertion — or with a structural collision (test hash / one-bucket table) |
| **R7** | ~~Ratification of the open constructs~~ **DONE 2026-08-28 — all eight adopted** (OPEN.md holds the rulings; specs in CONSTRUCTS.md). Implementation folds into R3: `set`, `exclusive`, `gpu`, `relation`, `resource`, `attributes`, payload binding, hierarchical `tags` | — | the same R3 gate lanes |
| **R8** | AI-orientation tooling ([`AI-ORIENTATION.md`](AI-ORIENTATION.md)): `check_block` + `aether check --format=json` (AIR-01), the `AE####` code registry + census (AIR-02/03), the generated grammar manifest + project schema dump (AIR-06 — see footnote †), `aetherfmt` + the **permutation-convergence gate** + comment survival (AIR-07/14), the compact generated surface file (AIR-11), recorded loop budgets (AIR-12). AIR-04/05/13 (accumulating block checks, resync fix, v1→v2 migration diagnostics) fold into **R3**; Gaia items bind through AIR-16/17 | R3 | every AIR oracle demonstrably red before its fix. **The probe blocks for `AD1` and `AD2` (blocks E and C — the defects renumbered from the old `G4`/`G5` on 2026-08-29) exist only in the 2026-08-28 session transcript and are NOT committed**, so no oracle may name them as artifacts: until the R3 fixtures land, every cell citing probe C or E must inline the block source or name the fixture path it will become |

† **AIR-06(b)** (project schema dump) depends on `boyko_reflect`, which is **not a workspace member**
and lives on the unmerged branch `feat/reflection`. Recording that dependency is determinate and was
never gated on ballot AB-9; the sequencing was, and **AB-9 is ✅ RULED 2026-08-30 — the merge is
pulled forward as its own rung before R8; AIR-06(b) is NOT descoped.**

Measured for that ruling, replacing the "18 commits ahead" this footnote used to carry: the branch is
**15 ahead of and 20 BEHIND** `feat/aether-v2` (merge-base `5ec1699f`), and the behind-count is the
one that grows. `git merge-tree --write-tree` (no merge performed) reports **7 conflicted paths** —
4 docs, 2 trybuild `.stderr` (one a **modify/delete**: `01a4436e` deleted `on_despawn_rejected.stderr`
here while reflection modified it), and **one source file**, `crates/boyko_macros/src/component.rs`,
at **2 hunks / 23 lines**. The merge brings 15 commits, 124 files, +44 956/−214, and **3 new
workspace members** (`boyko_reflect`, `reflect_fixture`, `reflect_dogfood`).

⚠ **And the merge is necessary, not sufficient.** Reflection is opt-in per component
(`type_info_of` → `None` without `#[component(reflect)]`), and **not one of the 50 opt-in sites on
`feat/reflection` is in an engine crate**. AB-9 therefore attaches a fourth item to R8's Lands that
no option on the ballot named: engine-crate opt-in, by a sweep or by a derive default — **which of
the two is R8's own design pass and is NOT decided by AB-9**, because it carries a per-component
static + `OnceLock` cost that has to be measured. Until one exists, the dump covers the empty set
under all three of the ballot's original options. Full body:
[`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29.

Rungs R0–R2 are pure engine work and can proceed in parallel worktrees (one worktree per system —
standing owner rule). R3 is the language pivot; everything after it layers on.

### Open ballots on this ladder

These are **not** decided here. **Six of the thirteen are no longer open** — AB-2, AB-3, AB-4, AB-5,
AB-8 and AB-9 were ruled 2026-08-30 under the owner's delegation of the perf/architecture forks
(`CLAUDE.md`); their rows are struck and point at the ruling that replaced them, and the rows are
**kept, not deleted**, because the record of what was open is what makes the ruling auditable. The
rest are owner ballots, and the rung one blocks may not be declared done while it stands open. This
is the **complete** `AB-*` set — a partial list here is the same failure as no list, because a rung
whose row names no ballot reads as unblocked. Bodies live in
[`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29 (the consolidated ballot list); the Gaia
`F`/`GB` series lives in [`../gaia/CAMPAIGN.md`](../gaia/CAMPAIGN.md).

| Ballot | Question | Alternatives | Blocks |
|---|---|---|---|
| **AB-1** | Event auto-registration: ratify on ergonomics alone, or stage it. ⚠ **reopens the ratified C3 grant, licensed by measurement** — the grant's recorded ground was that the unregistered case "fails silently on both ends", and both generated ends are in fact a loud init-time panic | (a) ratify as specified; (b) STAGE under D4 until an in-tree consumer exists | **R3** (event construct) |
| ~~**AB-2**~~ | ✅ **RULED 2026-08-30 [delegated] → [`DECISIONS.md`](DECISIONS.md) E4.** `lanes N` becomes a **minimum**, not a count: effective `max(N, worker_count + 1)`, resolved where the worker count is first known, raise **reported** at boot | *Rejected:* boot refusal (machine-dependent unbootability); dropping the knob — the cleaner language answer but a SCOPE call, **escalated to the owner, not taken**. Measured: the floor is unchecked on every path (`current_worker_id_or_dispatcher_lane` ignores its `worker_count` arg) and violation panics in **both** profiles, never `Result`; the **default** path already violates it — 3 of 64 worker sends panicked in release | ~~R3, R4~~ **unblocked** |
| ~~**AB-3**~~ | ✅ **RULED 2026-08-30 [delegated] → E5.** Claimed host lane, **one claimer enforced**; `MAX_EVENT_THREADS` 65 → **66**, const-assert to `MAX_WORKERS + 2 <= MAX_EVENT_THREADS` | ⚠ *Two ballot premises were false:* the const in the tree is **65, not 64** (KE8's lane half landed at `01a4436e`), so this buys **one** lane not two; and the dispatcher has **its own** reserved lane — only `WORKER_ID_UNATTACHED` maps to 0. *Rejected:* `Err` on unattached (breaks the engine's own documented main-thread route); **accepted hazard** — unavailable, the loss is silent (6/6 release runs lost 183–2295 of 8000, every send `Ok`) **and** UB, and the tree already holds that option's output as a **false** `// SAFETY:` clause | ~~R4~~ **unblocked** |
| ~~**AB-4**~~ | ✅ **RULED 2026-08-30 [delegated] → E6.** Registrant = the **generated path** (`register_ordered_emitter` at plugin build); predicate = an `ordered` event id with emitter count **> 1** fails boot naming both systems; verbatim escape **refused** at `EventWriter::init_state` | *Rejected:* `SystemMeta` emit-access — measured, `init_access` is an **empty body** (events are outside the conflict graph), and adding a write axis would serialise every same-type emitter pair, surrendering E1's parallel emission; the `#[event]` macro side — structurally impossible (exclusivity is a property of the emitter **set**; the macro sees one type, zero systems, and discards its own `_args`) | ~~R4~~ **unblocked** |
| ~~**AB-5**~~ | ✅ **RULED 2026-08-30 → [`DECISIONS.md`](DECISIONS.md) M4a** (+ M7, D6a): the router uses **`Query::get_mut`**. Decisive ground is not the tick but the event read — `get_component_mut` forces an **exclusive** system, whose body takes **no param tuple**, so it cannot hold `EventReader<E>` and must use `events_of`, which returns an **empty slice for an unregistered type** (silent) and reads the **previous** frame (2-frame latency). Riding answers: `publish tracked` **does** see the deposit same-frame (measured); M7's tick bypass is **withdrawn**, replaced by the emitted term (`Mut<M>` tracked / `&mut M` not) | *Rejected:* (b) `get_component_mut` — price: silent total event loss on a forgotten registration, doubled latency, and a bypass mechanism that exists only to repair it. *Hypothesis refuted and recorded:* the expected per-event **scheduler barrier does not exist** — 0.78–1.00×, below noise, against a control proving 2.7–3.2× real concurrency | ~~R5~~ — R5 still waits on **AB-7** |
| **AB-6** | `requires` of a dense-storage component. ⚠ the refusal option **narrows the ratified `storage = table\|dense` × `requires` surface** | parse refusal / a dense required-ctor route / known-open plus a hook workaround. KE11's red tests land now under any disposition — they demonstrate the panic either way | **R3**'s wording (and KE11's disposition, not KE11's tests) |
| **AB-7** | R-DENSE. ⚠ **re-grounds a ratified refusal — STILL THE OWNER'S.** But its premise is no longer open: the candidate driver-independent ground was **measured 2026-08-30 and REFUTED on both conjuncts** — the router's deposit path does **not** assume a table row (both `get_component_mut` and `Query::get_mut` carry working dense arms, pinned green in `ke3_query_random_access.rs`), and there **is no** layout const-assert (every dense `const assert` in the kernel belongs to a *driver*). Dense is fully iterable **and** tracked under `iter_mut` (64/64 rows). Detail: [`MACHINES.md`](MACHINES.md) §Refusals | unconditional with a driver-independent ground — **that ground does not exist as stated** / lifted by `publish tracked`, which is what the engine supports. ⚠ Note for whoever takes it: lifting does **not** make `parallel` machines work over dense — **both** parallel drivers compile-reject it (`E0080`), because *"the chunk runner has no world cell"*, not because of chunking. A genuinely driver-independent ground may arrive from **AB-6**/KE11 (`requires` over dense panics) — also the owner's, and **not** assumed here | **R5** |
| ~~**AB-8**~~ | ✅ **RULED 2026-08-30 → [`DECISIONS.md`](DECISIONS.md) C5a**: `each par` → **`par_iter_mut`**. Measured, not asserted: 2048/2048 rows reach a `Changed<>` reader through `par_iter_mut`, **0**/2048 through `par_for_each_chunk`; the tracked driver costs **1.17–1.47×** (0.03–0.07 ns/row) = **0.3–0.7 µs** at the cost model's 10 000 rows against the **34–40 µs** D1 measured for the jump table alone — **~1–2 % of the pass**. `soa par` **exists** and is the only route to the chunked driver. Batching key **not** author-visible in v1. Machines and `each` share the **ladder**, not the default | *Rejected:* `par_for_each_chunk` as the `par` driver — price: the most inviting parallel spelling would be structurally tick-blind (C5's own recorded defect, reproduced as the 0/2048 above). *Rejected:* `parallel (batch = N)` — `BatchingStrategy` has **three** fields, so one scalar cannot name what it appears to name; an author writing `batch = 64` would get 64 chunks **per thread**, not 64 rows per chunk | ~~R3, R5~~ |
| ~~**AB-9**~~ | ✅ **RULED 2026-08-30 [delegated] → [`DECISIONS.md`](DECISIONS.md) §Sequencing rulings, entry AB-9.** `boyko_reflect` sequencing for AIR-06(b): not a workspace member, on the unmerged `feat/reflection` — see the † footnote above for the measured merge cost. **The merge is pulled forward** as its own rung before R8; **AIR-06(b) is NOT descoped** (the claim that the reflection-free halves "still satisfy its oracle" was measured false — descoping removes clause 2's subject); and a **fourth item no option named** — engine-crate reflection opt-in — is attached to R8's Lands as a precondition | *Rejected:* R8 waits on the merge — price: the gate greens over a dump covering **0 of 138** engine components while the branch diverges in `crates/boyko_macros/src/component.rs`, the one file both campaigns edit. *Rejected:* descope (b) — same zero coverage **plus** the loss of the oracle's only workspace-facing clause: a gate that cannot fail. Full body: [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29 | ~~R8~~ **unblocked** (the merge rung and the opt-in decision are *work*, not ballots) |
| **AB-10** | AIR-10 residue | (a) is the measurement **script** required when the audit branch is taken, or does a DECISIONS audit line suffice; (b) is the joint Aether+Gaia vocabulary widening ratified — ⚠ (b) is a **scope change to a ratified requirement** | **R3**'s keyword surface only; the audit lines are written now, so R3 is not stalled on it |
| **AB-11** | `with`/`without` over a `flag` — both silent today (`with Flag` matches nothing, `without Flag` excludes nothing). ⚠ **adds a refusal where v1 documents non-refusal** | parse refusal with a did-you-mean at `enabled`/`disabled` (the recommendation — it dissolves the whole class) / forbidden-form-only, caught at doc generation | **R3**'s filter goldens |
| **AB-12** | *(a query, not a values call)* do the two dropped measured defects — the old `G1`/`G2` of the AI-orientation defect series — exist in the owner's session record? | (a) they exist → return as `AD5`/`AD6` with repros; (b) they never existed → the renumbered record stands | nothing (non-blocking) |
| **AB-13** | The flag-initial value vocabulary, and disambiguation across all three `on` positions (`machine … on entity`, `on E => T`, `flags (X = on)`). ⚠ the `flags → initial` rename **touches a ratified keyword** | `on\|off` reserved vs contextual; and: [`../gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) §Tier 3 **withdrew** the `flags → initial` rename (ground: `initial` is already the machine's initial-state keyword, so the rename recreates the defect it fixes) — does that withdrawal stand, or is a different rename wanted? | **R3** |

**Roll-up by rung** — the number a rung's own row does not show: **R0/R1/R2** — none, buildable now.
**R3** — AB-1, ~~AB-2~~, AB-6, ~~AB-8~~, AB-11, AB-13 (**four open**; AB-2 ruled 2026-08-30 at
**E4** and AB-8 the same day at **C5a**, plus AB-10 on its keyword surface alone).
**R4** — ~~AB-2, AB-3, AB-4~~ — **all three ruled 2026-08-30** (**E4**, **E5**, **E6**); R4 carries
**no open ballot**. It is not thereby buildable: E5 raises `MAX_EVENT_THREADS` to 66 and E4 needs a
dispatcher lane-count setter, and the rest of KE8 (`&self` send, `send_slice`, the re-aimed debug
assert) is still unlanded — those are work, not ballots.
**R5** — **AB-7 only**; AB-5 and AB-8 were RULED 2026-08-30 (DECISIONS **M4a** / **C5a**).
**R6** — none of its own; it depends on R5.
**R7** — ratification is done, but its implementation folds into R3, so it inherits R3's list.
**R8** — ~~AB-9~~ — **ruled 2026-08-30** ([`DECISIONS.md`](DECISIONS.md) §Sequencing rulings); R8
carries **no open ballot**. As with R4, that is not the same as buildable: the ruling *adds* two
pieces of work — the `feat/reflection` merge as its own rung before R8 (measured at 7 conflicted
paths, one of them a source file at 2 hunks / 23 lines), and engine-crate reflection opt-in, whose
route (sweep vs derive-default) is R8's own design pass and was deliberately not decided. R8's
`Depends` cell also still names **R3**, which carries four open ballots — so "no ballot of its own"
is not "reachable".

**Two blockers on this ladder are NOT ballots and must not be waited on as such:** K8 (a bundle
carrying a `link Entity` field has no honest spawn spelling) and K9 (the kernel `relates`/`related`
macro demands a private collection field plus `retain_empty`, inexpressible on a pub-field Aether
group) are architecture gaps routed to **R3's own design pass**, not to the owner — R3's `bundle`
and `relation` constructs may not be declared done while they stand. Both are recorded in
[`../gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) §Part E.

**Three more items ride NO rung on either ladder — `CG-1`, `CG-2`, `CG-3`.** They are not ballots
either, so no register was carrying them; their single home is
[`../gaia/CAMPAIGN.md`](../gaia/CAMPAIGN.md) §Carrier gap, cited from here because two of the three
are candidates for **this** ladder. **CG-1** (`link` as the shared remap spelling in authored-scene
emission) and **CG-2** (component references riding mandatory explicit `stable_name`, likewise) are
Gaia assertions about **Aether's** emitted scenes, so R3's `scene` surface is a candidate owner.
**CG-3** — GB-6(b)'s conditional-visibility pressure valve — carries an explicit **deadline**
("before the first designer asks") and GB-6(b) states in as many words that it needs "an F-id AND a
rung on the **Aether** ladder"; it has neither. A deadline with no rung can never come due, so it is
not a schedule. **Assigning them is the owner's or the architect's call, not this plan's**; what is
recorded here is that they exist and that this ladder is where two or three of them would land.

Also rungless, and on this side: the **two authored-scene emission fixes** in
[`CONSTRUCTS.md`](CONSTRUCTS.md) §`material`, `scene` (collapsing the per-node `.insert` chain into
one generated extras bundle; grouping same-shaped anonymous nodes through `spawn_batch`). They are
the Aether twins of CG-1/CG-2 — asserted, unowned.

Two Gaia-series ballots also named rungs on **this** ladder. ✅ **Both RULED 2026-08-30**; bodies,
measurements and rejected alternatives are on the Gaia side
([`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29).

- **GB-9** — the `Or`-over-dense generated-code ban is **DELETED with a record** (option b). Its
  ground was R0's fixed defect; the fallback ground was **D4's coupling**, and the ruling measured
  it and rejected it — **D4 reserves the Aether *surface* `or(...)` while the ban governed
  *generated code*, and Gaia's ratified GN2 says the baker emits no Rust**. ⚠ Nor could it be
  re-grounded on **KE13**: `Or` folds `NEEDS_CHANGE_DETECTION` over its members and
  `EcsMaster::query<D, F>()` const-refuses any such `F`, so the banned shape cannot reach a
  `QueryView` at all. **D4 itself is untouched** — what is settled is that it cannot be lent out as
  a second document's ground.
- **GB-8** — ruled, and **its placement on this ladder did not survive**: the **id** census widens
  corpus-wide at **Gaia G0** (a one-constant change, green at zero remediation — the outside-scope
  citations sit in 5 files and every id resolves), while only the **link** census lands here at **R8**,
  with 59 measured dead relative targets under `docs/` as its red-first evidence. **No per-site
  waivers** at either.

> ⚠ **Two readings of GB-9's "before R0 lands" are in the corpus, and they disagree.** This section
> says GB-9 *must be decided before R0 lands*, while
> [`../AETHER-GAIA-REVISION-2026-08-29.md`](../AETHER-GAIA-REVISION-2026-08-29.md) §Work order lists
> R0 under **"buildable now, no ballot in the way"** — and the roll-up three paragraphs above says
> **"R0/R1/R2 — none, buildable now."** Recorded here rather than resolved by edit, but the
> evidence in the corpus points one way, and it is worth stating so nobody stalls a green rung:
>
> - GB-9's own **`Blocks` field names Gaia's `G7`, not R0** (`../OPEN-QUESTIONS.md` §2026-08-29).
> - The revision record's blocked-rung table lists **G7** as waiting on GB-6/GB-7/GB-9; **R0 appears
>   in that table not at all**.
> - R0 is a `boyko_ecs` bug fix whose oracle is a kernel-side red-first test. GB-9 asks what a
>   **Gaia generator that does not exist yet** should emit. Nothing in R0's scope or oracle changes
>   with GB-9's answer.
>
> **So: "before R0 lands" is a staleness deadline on the DECISION, not a build blocker on the
> RUNG.** The thing it protects is real and is the reason the deadline was written — once R0 is
> green, the ban's original ground is gone and a fixture written against the *kernel's* behaviour
> "goes green on that R0 commit and stops guarding anything, without anyone editing it"
> ([`../gaia/DECISIONS.md`](../gaia/DECISIONS.md) §UI bindings, item 8).
>
> ⚠⚠ **THE DEADLINE EXPIRED UNANSWERED — R0 LANDED 2026-08-29 WITH GB-9 STILL OPEN**, and the
> ballot was answered a day late, on 2026-08-30. Measured, not predicted: the R0 row above reads
> "✅ LANDED 2026-08-29", [`KERNEL-BACKLOG.md`](KERNEL-BACKLOG.md) KE1 reads "✅ LANDED (R0,
> 2026-08-29)", and the oracle `crates/boyko_ecs/tests/ke1_or_dense_blindness.rs` is in the tree and
> **8/8 green**. GB-9 did not lapse; it became a ballot whose **original ground can no longer be
> observed in the tree**, and the ruling had to reconstruct option (b)'s "record of why it existed"
> from `gaia/DECISIONS.md` §UI bindings item 8 rather than from a reproducible failure. That is
> exactly the cost the deadline existed to prevent, and it is recorded rather than smoothed away.
>
> **The reading argued above was vindicated and should be reused**: "before R0 lands" was a
> staleness deadline on the DECISION, not a build blocker on the RUNG. R0 was correctly not stalled.
> **The lesson the expiry teaches is the other half**: a deadline with no rung and no owner cannot
> come due. If a future cross-ladder coupling carries a decision deadline, it needs a rung on
> someone's ladder — the note alone did not make anyone act.

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
- Doc claims about engine behaviour cite the symbol, not just a line number — exact `:N` anchors rot
  and are used only where freshly verified. ⚠ **Corrected 2026-08-30 while ruling GB-8**: this line
  used to say "188 of 302 were waived in the last census", a figure **no in-tree gate produces**.
  Run live, `tests/internal_docs_anchors.rs` prints **735 anchors, 116 waived** — and the point is
  sharper than the wrong aggregate implied, because the waiving is not spread: the one document
  admitted under a waiver allowance waives **89 of its 177 (50.3%)**, and a waived anchor asserts
  only that the line number is inside the file.
