# Gaia — the data language: campaign plan

**Aether is for logic, Gaia is for data** (owner, 2026-08-28). Gaia is one language with three
profiles — **scene** (world/level content), **ui** (documents over the ECS-native `boyko_ui`), and
**data** (the DataAsset/DataTable analog: item defs, ability params, loot tables) — authored as
text, **baked at build time** (reflection allowed only there, behind a default-off cargo feature),
shipped as a binary the runtime loads with **zero reflection, zero interpreter, zero fallback**.

**Start here after 2026-08-29:**
[`../AETHER-GAIA-REVISION-2026-08-29.md`](../AETHER-GAIA-REVISION-2026-08-29.md) — the revision
record shared with the Aether v2 campaign: what the engine refuted, which gates were struck as
unfalsifiable, the per-file change summary, and the work order (which ballot blocks each rung).

This directory is the decision record and plan. File map: [`DECISIONS.md`](DECISIONS.md) (every
ruling with its rationale and rejected alternative — the AIR-17 carrier), [`LANGUAGE.md`](LANGUAGE.md)
(the research-stage language sketch), [`PENDING-SYNTAX-PLAN.md`](PENDING-SYNTAX-PLAN.md)
(**NOT APPROVED, NOT COMMITTED — owner-gated**: the reworked syntax and the conflict-audit fix plan.
It holds the **syntax ruling series R1–R7** — spelled like the Aether campaign rungs and unrelated
to them — the **conformance-audit findings** (`M#`/`C#`/`I#`/`G-##`, one prefix per hunting lens and
**not a contiguous range**; their full bodies live outside the repo, so each is summarised at its use
site), the **K1–K16 adversarial-pass record**, and the fix **Tiers 0–4**. Every "PENDING &lt;id&gt;" cite
in this corpus resolves to a use site there, and until the owner approves the file, the rulings it
carries are proposed, not ratified). The commissioning research: two multi-agent passes
(2026-08-28, ~25 systems surveyed with sources; engine inventory verified line-by-line).

## Relation to Aether — separate scopes, ONE body of work

Gaia and [Aether v2](../aether-v2/CAMPAIGN.md) are **one body of work split by subject matter, not
two projects**: *Aether for logic, Gaia for data*. The couplings are concrete and checkable today —
this file's [`DECISIONS.md`](DECISIONS.md) cites the Aether campaign's **`KE#` kernel-backlog ids**
directly; the AI-orientation requirement set that binds Gaia lives in the **Aether** directory
([`../aether-v2/AI-ORIENTATION.md`](../aether-v2/AI-ORIENTATION.md)), where **AIR-08** (stable node
ids) and **AIR-16** (grammar rulings) carry `Gaia spec` in their own Rung column, **AIR-17** carries
`Gaia G0`, and **AIR-09** carries `R8 / Gaia tooling`; `AE####` and `GA####` are one registry
discipline minted by AIR-02; and two ballots of **this** `GB` series name rungs on the **Aether**
ladder (**GB-9** → Aether R0, **GB-8** → Aether R8). ✅ **Both RULED 2026-08-30**, and one of the
two cross-ladder claims did not survive the ruling: **GB-8 no longer names Aether R8 for the whole
census** — its id half lands at **G0, on this ladder**, and only the *link* half rides R8. GB-9's
"→ Aether R0" was always a decision deadline rather than a build dependency, and it is now closed.

**And each campaign finds the other's defects.** Three defects in *shipped* code — the `Or` dense
arm (Aether **KE1** / rung R0), `any_changed_since` over a dense or bitset bind source (**GK-2**
below), and the load path's dropped `requires` closure (**F4(ii)** below) — are **one mechanism**:
code that resolves a per-archetype pool without screening the storage kind, and so is blind to the
kinds that own no pool. **Two of the three were raised here, in kernel and UI code the logic
campaign owns.** The joint account, the couplings table, and the still-owed enumeration of the class
are in [`../AETHER-GAIA-REVISION-2026-08-29.md`](../AETHER-GAIA-REVISION-2026-08-29.md) §One
mechanism, three shipped defects and §Aether and Gaia are one body of work. A change touching any
coupling above updates **both** sides in the same commit — the diverged-pair cost is already
measured in this repo.

## The industry map, in one paragraph

Identity converged on a **pair** (durable file id + file-local object id) — Unity and Godot arrived
independently, Unreal's path-as-identity is the negative control (an entire redirector subsystem).
Composition converged on **patch-not-replace** with per-field granularity; no production system has
multiple inheritance. The most important negative result: **AOT compilation removes the parser, not
the runtime** — Slint still ships a full property graph; Svelte retreated from compile-time-only
reactivity; qmlsc silently degrades to an interpreter. Zero-cost reactivity is achievable only by
CONSTRUCTING the language so bindings are statically enumerable and the arrow is inverted
(sink asks source via ticks) — which is exactly what `boyko_ui`'s shipped bind path already does.
Logic creep has a documented trajectory (Paradox: literals → … → "calculated on every frame,
massive lag"); what stops it is refusal plus a pressure valve (curves), not discipline.

## Rung ladder

| Rung | What | Gate |
|---|---|---|
| **G0** | This directory; ratified decision lines (AIR-17); the AI-ORIENTATION correction (its "Gaia type-level substrate CLOSED" line was refuted by inventory); owner ballots F1 and F4 held — **both block later rungs** | docs exist before any grammar commit **AND** every AIR cross-note on a ratified line in gaia DECISIONS resolves to a defined AIR item (AIR-17's own second clause, restored here in AIR-17's own scope). A file that RESOLVES but is STALE against the ratified syntax does not satisfy a cross-reference — `LANGUAGE.md` is the standing example (PENDING M6). ✅ **GB-8 RULED 2026-08-30, and this row now carries work.** (1) **No per-site waivers**, at any of the four censuses in [`tests/gaia_g0_citation_census.rs`](../../tests/gaia_g0_citation_census.rs), ever; where a property is not decidable as written, the remedy is the one that file already practises — narrow the predicate and print the narrowing in the failure message. ⚠ The "188 of 302" this row used to cite is **not this tree's number and no in-tree gate produces it**: `internal_docs_anchors.rs` run live prints **735 anchors, 116 waived** — but the precedent is *stronger* than that aggregate, because the waiver concentrated entirely in the one document admitted under it, which waives **89 of 177 (50.3%)**. (2) **The AIR census widens to all of `docs/` and it lands HERE, at G0** — a one-constant change (`G0_DIRS` → `markdown_under("docs")`), **green at zero remediation** — the outside-scope citations sit in 5 files and **every id cited resolves inside `AIR-01..AIR-18`**, so none is a repair (⚠ the *count* is not pinned: it was 21 before the ruling was written and 40 after, because the ruling's own text cites `AIR-06`; the property is the ruling, the number is an observation with a timestamp) — and the sibling KE/KM census in the same file already runs corpus-wide from G0. (3) **The LINK census is a separate deliverable and lands at Aether R8**, not here — measured 1636 relative targets under `docs/`, **59 dead across 11 files, 44 of them in `docs/AUDIT-2026-05-23.md`**; landing it with the id half would hold the free half hostage to 59 repairs. Ruling in this campaign's decision log: [`DECISIONS.md`](DECISIONS.md) §Census discipline; body and rejected alternatives: [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29. |
| **G1** | **GK-4**: derive-emitted name-keyed field tables + typed constructors in `boyko_macros` (the macro-time bake route — no reflection needed even at bake) | red-first and TEXT-FREE (G1 predates the grammar, so no gate of G1's may parse anything): `bake_one(<typed ctor calls built from the derive-emitted field table>) == save_world(hand-built world)` byte-for-byte for one component; plus a probe pinning `BindText`'s actual Serializability class |
| **G2** | Language core: lexer (EOL normalization before the CST) → lossless CST → canonical printer | `print(parse(canon)) == canon`; **absorbs the text entrance of G1's gate — `bake_one(text) == save_world(hand-built world)` byte-for-byte** — which cannot be stated at G1; round-trip compared on the WORLD side with a comparator GENERATED from the component registry (two in-tree proofs of why a hand list fails: the green-over-divergence equivalence gate and the 10-of-19 printer loss); parser dispatch + printer list + gate comparator = ONE generated table + coverage census |
| **G3** | Identity: asset/object ids, `@`-references, mandatory `link`, offline bake resolution | red fixtures: dangling ref, duplicate id, anonymous-target refusal, stale patch target; the unstable-field-type bake lint (GN1) |
| **G4** | Composition: templates / `abstract` / patches / the priority ladder / `remove` / bake budgets / provenance sidecar | **file-order independence**: `bake(files) == bake(shuffle(files))` byte-identical over ≥2 shuffled permutations plus the reversed order, on a fixture whose ladder layers span several files; byte-identity scoped to the bake OUTPUT (diagnostics may still list files in walk order); the permutation count is reported by the test. Plus the two-writes-one-layer red fixture (the error must name BOTH files). Plus the Godot-#32179 fixture (diff-at-save eating an intentional override). Plus the per-axis bake-budget fixture set (DECISIONS §The logic line: one committed red fixture per axis). ⚠ The former `bake(1) == bake(W)` form is **struck as vacuous**: `W` has no referent anywhere in this corpus and nothing makes the bake parallel, so the gate could not fail — the exact class this pass repairs. If a parallel bake is ever intended it arrives as its own design line with a defined `W`. |
| **G5** | **data** profile (smallest; needs F2/F3): tables → dense columns, `[DefOf]`-style generated row constants, curves + `scalable`, contracts | per-profile fixtures; named red-first, the **eagerness** fixture: a violated `contract` on a field NOTHING reads must fail the bake, through both entrances — a plain declared field, and a field that a template expanded but no one ever reads — with blame on the file+span of the violating VALUE (Jsonnet's lazy assertions are the negative control) |
| **G6** | **scene** profile (needs the F4 fixup seam + stable-asset-id carrier forms): the cell catalog emitted from day one; `load_cell` + GK-1 as its own rung | reconstruct-and-compare on the catalog |
| **G7** | **ui** profile (prerequisites: a windowed UI pass and a real `UiPlugin` — UI does not reach the screen today; `.ui` absorption per F6) | the STILL-FRAME gate, re-axed onto **counts, not a clock**: still frame over a HUD with 200 bindings — bind-sink writes executed **== 0** (observable because every generated sink is set-if-changed) **and** the tick slots read by `any_changed_since` PINNED to the fixture's expected value; red-first by dirtying exactly one source, after which BOTH counters must move. A wall clock cannot say which work disappeared, and the delta-subtraction form alone was unfalsifiable. **GB-7 RESOLVED 2026-08-30 (standing rule): NO wall-clock companion — the count gate stands alone.** Measured, not asserted: the timed loop's inputs are archetype count, bound-TYPE count and row count, and **not** the binding count (`dynamic_bound_ids` is a deduplicated type set, `boyko_ui/src/binding/bind_system.rs:56-60`), so the still frame is **332 ns at +0% for 10× the bindings** but **+82% for 2× the rows** and **+101% for 2× the bound types** — a threshold here tracks everything the fixture does not pin. `Instant::now()`'s step is 100 ns, so the frame is ~3.3 ticks with a 15-25× single-call tail; the red-first delta (0 → 1 sink write) sits at signal-to-noise **0.05-0.50**, which the count gate resolves exactly and no clock does. Body and full table in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29; the ruling also lands in [`DECISIONS.md`](DECISIONS.md) §UI bindings item 9. A clock over the **scan itself**, with the world pinned, belongs to **GK-2**'s design pass. ✅ **GB-9 RESOLVED 2026-08-30, and this rung's codegen rules change with it** — the ballot this row did not name, whose `Blocks` field read "**G7**'s codegen rules". Ruled **option (b): the `Or`-over-dense emission ban is DELETED with a record** ([`DECISIONS.md`](DECISIONS.md) §UI bindings item 8 carries it). Its ground was fixed and gated by Aether R0; the fallback ground (Aether D4) was measured and rejected — **D4 reserves the Aether *surface* `or(...)` while the ban governed *generated code*, and this campaign's own ratified GN2 says the baker emits no Rust**, so the ban had no subject. ⚠ It could not be re-grounded on KE13 either: `Or` folds `NEEDS_CHANGE_DETECTION` and `EcsMaster::query` const-refuses it, so the banned shape cannot reach a `QueryView` at all. **What this rung now owes instead:** the ban's fixture discipline rides item 8's third row — the red fixture asserts the **generator does not emit a change-gate over a non-signature storage kind** (GK-2's ground, still live) — and the shapes with **no oracle** if a generator ever does emit `Or` over dense are filed here: `Query<Entity, Or<..dense..>>`, `Added<Dense>` in `Or`, arity > 2 with more than one dense arm, and KE13 on the point-lookup path. ⚠ Unreconciled in that same work order: it lists **GB-6** against G7, while GB-6's own body names **G6**; not resolved here. |
| **G8** | Streaming remainder + GK-2/GK-3, each behind its own design pass | defined by each sub-item's own design pass (GK-2, GK-3, streaming remainder); G8 cannot close before those passes exist and name their own red-first oracles. Nothing here pre-commits GK-2's oracle. |

## Kernel requests born from the design (each gets its own design pass)

| | What | Why |
|---|---|---|
| GK-1 | a cross-load map (global object id → Entity) with a declared lifetime | `LoadEntityMap` already keys on an arbitrary u64 — cheap to generalize; unlocks cross-cell refs (F5) |
| GK-2 | a per-column last-changed tick on write | today's still-frame gate `any_changed_since` is an O(live rows) scan documented as "cheap" — a gate one rung below its own documentation; the flecs per-table step is the right middle. **The cost is not the whole rationale: the mechanism is also CORRECTNESS-blind** — `any_changed_since` resolves per-archetype pools, and non-signature storage owns none, so a bind source on a dense (or bitset) component is invisible to it and the gate is never true (DECISIONS §UI bindings, item 8) |
| GK-3 | the post-load fixup seam (F4) | the loader runs no hooks: reverse indexes absent, asset refcounts at 0 — a loaded scene's meshes are retirable mid-game |
| GK-4 | derive-emitted field tables + typed ctors | the bake route that avoids both the rejected EG2 and the unlanded C11 |

## Owner ballots (open; the fork bodies with prices live in [`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) — in TWO dated sections)

The table below is the INDEX only. `DECISIONS.md` has no `§Forks` and must not gain one: it is a
log of RULINGS, and these questions are open — a rulings log that carries open forks is how a fork
gets silently settled by proximity.

**Where each body lives.** **F1–F7** were raised 2026-08-28 and carry their full bodies in
§*2026-08-28 — Gaia: the owner ballots, two of them blocking*. **F8, F9 and F10** were born in the
2026-08-29 corpus audit and have no body in that earlier section at all — theirs are in
§*2026-08-29 — Corpus audit of the aether-v2 + gaia plans*, which is also where the `GB` series
(GB-1…GB-9, cited from the rung ladder above) lives. A reader sent to one section alone finds three
of these ten rows unbacked.

| # | Question | Recommendation | Blocks |
|---|---|---|---|
| **F1** | The bake route into the byte format: the audited-and-rejected EG2 reflection seam, or macro-time GK-4 now with EG2 as a later upgrade? | (b) GK-4 now | **G1** |
| **F4** | What "loaded" means — **WIDENED from hooks to every insert-path mechanism**: (i) hooks (the loader runs none: reverse indexes absent, asset refcounts at 0); (ii) **the `requires` closure — MEASURED, not inferred (PENDING M1): nothing on the load path adds a component the file omitted, and `RequiredCtor` being an `unsafe fn(*mut u8)` makes ctor-form requires unbakeable in principle, so `query<(&Health, &Regen)>` silently skips level-authored entities while gameplay-spawned ones match**; (iii) flag initial state (bitset ids are filtered out of the loaded signature); (iv) relation reverse index; (v) asset refcounts. Options: specified post-load fixup pass (a), loader fires attach hooks (b), or forbid hook-dependent components in baked assets (c — untenable, it forbids `MeshHandle`) | (a) fixup registry + census gate — the census must enumerate all five mechanisms, not hooks alone | **G6, and the engine's load semantics generally** |
| F2 | Where DataAsset data lives at runtime: entity-shaped dense columns + generated row constants (a) or a new resource region in the format (b) | (a); its VALUES half is the sentence "a table is entities" | G5 |
| F3 | Table form: single-file-per-asset only, or also a table file baking N rows into one dense column | both over ONE schema | grammar |
| F5 | Streaming scope: format-ready-loader-later (cell catalog now, `load_cell`/unload/GK-1 later) | (a) — catalog + attribution + persistent id map are ONE design unit | G6 |
| F6 | `.ui` fate after absorption: migrate + delete in the same campaign, or freeze until an owner-eval of a Gaia HUD | (a) — the diverged-pair cost is already measured | G7 |
| F7 | Mods: ratify "out of v1" explicitly, or accept that a text-mod pipeline ships the bake tool to PLAYERS as a separate executable (re-stating the constraint as "reflection never in the game binary") | ratify explicitly either way — silence here is a default decision | — |
| **F8** | Name-vs-id for objects: is the object NAME the id (then the `--assign-ids` machinery is deleted and rename tooling replaces it), or does syntax ruling [**PENDING R4**](PENDING-SYNTAX-PLAN.md) grow a separate id slot beside the name? (**not** Aether rung R4 — the two `R#` series are unrelated) | ⚠ none offered — this **REOPENS the ratified `gaia fmt --assign-ids` identity ruling**, so AIR-10 requires the trigger and the measurement to be stated in the ballot body before it is reopened | TBD — owner scoping |
| **F9** | Flags carrier: a per-entity enable-bit region in the format plus one spelling, or a coded refusal naming Aether's `flags (…)` group | — | **G2, G6** (the only Blocks PENDING states, at M4) |
| **F10** | Bundle: expand at bake vs refuse. **Pre-question first: is this a ballot at all** — PENDING Tier 3 and Part D contradict each other on that point, and the contradiction must be resolved before the question is put | — | TBD — owner scoping |

A veto point (decided, owner may veto): first-class `remove` in inheritance is IN the v1 grammar —
default-inexpressibility costs more later (the Unity nested-prefab lesson in miniature).

## Relations

- **aether-v2**: `scene` in Aether stays dev-bootstrap-only; the Gaia binary is the only shipped
  scene form (the alternative, a shared SceneModel, is priced higher). ⚠ **Open ballot GB-6** —
  this line carried two bare "ratify" imperatives, and neither was classified as ballot or as
  delegated decision; GB-6 classifies both:
  - **(a) the scene-form fork.** If it is a ballot, it **Blocks G6** and gets a body in
    OPEN-QUESTIONS. If it is delegated, the line is relabelled "(decided, owner may veto)" — the
    shape the `remove` veto point above already uses. It cannot stay in between.
  - **(b) the "conditional visibility" pressure valve** (an EnableTag-toggling action idiom).
    It carries a DEADLINE — "before the first designer asks, or the document side caves" — so it
    must get an F-id AND a rung on the **Aether** ladder: a deadline with no rung can never come
    due, and is therefore not a schedule.
  Recorded separately as a **carrier gap**: the two authored-scene emission fixes on this line —
  `link` as the shared remap spelling, and component references riding mandatory explicit
  `stable_name` — ride no rung at all today. They are asserted here and land nowhere; the pass
  that names their carrier is owed and is not part of GB-6.

### Carrier gap — `CG-1`..`CG-3`: asserted here, landing nowhere

**This subsection is their single home.** They were unlosable only as prose inside a longer bullet,
which is how an item with no rung disappears. Ids are minted here so both ladders can cite them; the
`CG` series is otherwise unused in this corpus (`grep -rn "\bCG-[0-9]" docs/` returned nothing
before this line). **None of the three can be assigned a rung by this pass** — that is the owner's
or the architect's call, and none of them is a ballot question, so `OPEN-QUESTIONS.md` is the wrong
register too. What they need is an owner and a rung, and the pass that names one is **owed**.

| id | The item | Why it is stranded | Who could own it |
|---|---|---|---|
| **CG-1** | **`link` as the shared remap spelling** for authored-scene emission — the same keyword doing the same job on both sides of the boundary, so an entity-bearing field is remapped identically whether it was written in Aether or in Gaia | Asserted in §Relations above and in **no rung's deliverable list**, on either ladder. Related but **not** the same item: [`PENDING-SYNTAX-PLAN.md`](PENDING-SYNTAX-PLAN.md) **M3** already owns the *cross-check* between the two carriers (bake refuses a `link` mismatch in either direction, two red fixtures at **G3**) and **M10** owns the bake refusal for an `Entity` field authored without `link`. What has no carrier is the **shared spelling decision itself** in Aether's emitted authored scenes | Aether **R3**'s `scene` surface, or Gaia **G3** beside M3/M10 |
| **CG-2** | **Component references ride mandatory explicit `stable_name`** in authored-scene emission | The `stable_name` *ruling* is ratified and carried ([`DECISIONS.md`](DECISIONS.md) §Identity and references — the bake refuses the default module-path name), and its resolution axis is specified at PENDING **M8**. What rides no rung is applying it to **Aether-authored scene emission**, which is where the assertion was made | same as CG-1 |
| **CG-3** | **GB-6(b)'s pressure valve** — the "conditional visibility" `EnableTag`-toggling action idiom | It carries an explicit **DEADLINE** — *"before the first designer asks, or the document side caves"* — and no rung. The corpus states the consequence itself: **a deadline with no rung can never come due, and is therefore not a schedule.** GB-6(b) classifies the *line*; it does not schedule the valve | the **Aether** ladder (GB-6(b) says so in as many words: it "must get an F-id AND a rung on the Aether ladder") |

⚠ **CG-1 and CG-2 are the two "authored-scene emission fixes" of §Relations, not new work**, and
they must not be confused with the *other* pair of authored-scene emission fixes recorded on the
Aether side ([`../aether-v2/CONSTRUCTS.md`](../aether-v2/CONSTRUCTS.md) §`material`, `scene`: the
one-extras-bundle collapse and `spawn_batch` grouping). Those two also ride no rung. Four items,
two homes, one shared property.
- **AI-orientation**: AIR-08/09/16/17 bind Gaia from the spec; diagnostics ride the AIR-01 envelope
  from the baker's first commit; `GA####` codes from the shared registry discipline.
- **Out of scope**: mod pipeline (F7), an editor, non-Latin text shaping, graph materials.
