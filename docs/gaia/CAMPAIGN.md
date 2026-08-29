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
| **G0** | This directory; ratified decision lines (AIR-17); the AI-ORIENTATION correction (its "Gaia type-level substrate CLOSED" line was refuted by inventory); owner ballots F1 and F4 held — **both block later rungs** | docs exist before any grammar commit **AND** every AIR cross-note on a ratified line in gaia DECISIONS resolves to a defined AIR item (AIR-17's own second clause, restored here in AIR-17's own scope). A file that RESOLVES but is STALE against the ratified syntax does not satisfy a cross-reference — `LANGUAGE.md` is the standing example (PENDING M6). ⚠ **Open ballot GB-8**: whether this census is mechanised corpus-wide, at which rung it lands, and whether it may carry per-site waivers — the anchor-census precedent (waivers licensed silent rot, 188 of 302 abdicated) says no. Not settled here. |
| **G1** | **GK-4**: derive-emitted name-keyed field tables + typed constructors in `boyko_macros` (the macro-time bake route — no reflection needed even at bake) | red-first and TEXT-FREE (G1 predates the grammar, so no gate of G1's may parse anything): `bake_one(<typed ctor calls built from the derive-emitted field table>) == save_world(hand-built world)` byte-for-byte for one component; plus a probe pinning `BindText`'s actual Serializability class |
| **G2** | Language core: lexer (EOL normalization before the CST) → lossless CST → canonical printer | `print(parse(canon)) == canon`; **absorbs the text entrance of G1's gate — `bake_one(text) == save_world(hand-built world)` byte-for-byte** — which cannot be stated at G1; round-trip compared on the WORLD side with a comparator GENERATED from the component registry (two in-tree proofs of why a hand list fails: the green-over-divergence equivalence gate and the 10-of-19 printer loss); parser dispatch + printer list + gate comparator = ONE generated table + coverage census |
| **G3** | Identity: asset/object ids, `@`-references, mandatory `link`, offline bake resolution | red fixtures: dangling ref, duplicate id, anonymous-target refusal, stale patch target; the unstable-field-type bake lint (GN1) |
| **G4** | Composition: templates / `abstract` / patches / the priority ladder / `remove` / bake budgets / provenance sidecar | **file-order independence**: `bake(files) == bake(shuffle(files))` byte-identical over ≥2 shuffled permutations plus the reversed order, on a fixture whose ladder layers span several files; byte-identity scoped to the bake OUTPUT (diagnostics may still list files in walk order); the permutation count is reported by the test. Plus the two-writes-one-layer red fixture (the error must name BOTH files). Plus the Godot-#32179 fixture (diff-at-save eating an intentional override). Plus the per-axis bake-budget fixture set (DECISIONS §The logic line: one committed red fixture per axis). ⚠ The former `bake(1) == bake(W)` form is **struck as vacuous**: `W` has no referent anywhere in this corpus and nothing makes the bake parallel, so the gate could not fail — the exact class this pass repairs. If a parallel bake is ever intended it arrives as its own design line with a defined `W`. |
| **G5** | **data** profile (smallest; needs F2/F3): tables → dense columns, `[DefOf]`-style generated row constants, curves + `scalable`, contracts | per-profile fixtures; named red-first, the **eagerness** fixture: a violated `contract` on a field NOTHING reads must fail the bake, through both entrances — a plain declared field, and a field that a template expanded but no one ever reads — with blame on the file+span of the violating VALUE (Jsonnet's lazy assertions are the negative control) |
| **G6** | **scene** profile (needs the F4 fixup seam + stable-asset-id carrier forms): the cell catalog emitted from day one; `load_cell` + GK-1 as its own rung | reconstruct-and-compare on the catalog |
| **G7** | **ui** profile (prerequisites: a windowed UI pass and a real `UiPlugin` — UI does not reach the screen today; `.ui` absorption per F6) | the STILL-FRAME gate, re-axed onto **counts, not a clock**: still frame over a HUD with 200 bindings — bind-sink writes executed **== 0** (observable because every generated sink is set-if-changed) **and** the tick slots read by `any_changed_since` PINNED to the fixture's expected value; red-first by dirtying exactly one source, after which BOTH counters must move. A wall clock cannot say which work disappeared, and the delta-subtraction form alone was unfalsifiable. ⚠ **Open ballot GB-7**: whether a wall-clock companion is kept beside the count gate, and at what tolerance / run count / noise floor. |
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
- **AI-orientation**: AIR-08/09/16/17 bind Gaia from the spec; diagnostics ride the AIR-01 envelope
  from the baker's first commit; `GA####` codes from the shared registry discipline.
- **Out of scope**: mod pipeline (F7), an editor, non-Latin text shaping, graph materials.
