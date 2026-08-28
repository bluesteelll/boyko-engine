# Gaia — the data language: campaign plan

**Aether is for logic, Gaia is for data** (owner, 2026-08-28). Gaia is one language with three
profiles — **scene** (world/level content), **ui** (documents over the ECS-native `boyko_ui`), and
**data** (the DataAsset/DataTable analog: item defs, ability params, loot tables) — authored as
text, **baked at build time** (reflection allowed only there, behind a default-off cargo feature),
shipped as a binary the runtime loads with **zero reflection, zero interpreter, zero fallback**.

This directory is the decision record and plan. File map: [`DECISIONS.md`](DECISIONS.md) (every
ruling with its rationale and rejected alternative — the AIR-17 carrier), [`LANGUAGE.md`](LANGUAGE.md)
(the research-stage language sketch). The commissioning research: two multi-agent passes
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
| **G0** | This directory; ratified decision lines (AIR-17); the AI-ORIENTATION correction (its "Gaia type-level substrate CLOSED" line was refuted by inventory); owner ballots F1 and F4 held — **both block later rungs** | docs exist before any grammar commit |
| **G1** | **GK-4**: derive-emitted name-keyed field tables + typed constructors in `boyko_macros` (the macro-time bake route — no reflection needed even at bake) | red-first: `bake_one(text) == save_world(hand-built world)` byte-for-byte for one component; plus a probe pinning `BindText`'s actual Serializability class |
| **G2** | Language core: lexer (EOL normalization before the CST) → lossless CST → canonical printer | `print(parse(canon)) == canon`; round-trip compared on the WORLD side with a comparator GENERATED from the component registry (two in-tree proofs of why a hand list fails: the green-over-divergence equivalence gate and the 10-of-19 printer loss); parser dispatch + printer list + gate comparator = ONE generated table + coverage census |
| **G3** | Identity: asset/object ids, `@`-references, mandatory `link`, offline bake resolution | red fixtures: dangling ref, duplicate id, anonymous-target refusal, stale patch target; the unstable-field-type bake lint (N1) |
| **G4** | Composition: templates / `abstract` / patches / the priority ladder / `remove` / bake budgets / provenance sidecar | `bake(1) == bake(W)` byte-identical; the Godot-#32179 fixture (diff-at-save eating an intentional override) |
| **G5** | **data** profile (smallest; needs F2/F3): tables → dense columns, `[DefOf]`-style generated row constants, curves + `scalable`, contracts | per-profile fixtures |
| **G6** | **scene** profile (needs the F4 fixup seam + stable-asset-id carrier forms): the cell catalog emitted from day one; `load_cell` + GK-1 as its own rung | reconstruct-and-compare on the catalog |
| **G7** | **ui** profile (prerequisites: a windowed UI pass and a real `UiPlugin` — UI does not reach the screen today; `.ui` absorption per F6) | the STILL-FRAME gate: a HUD with 200 bindings and nothing changing must cost like a HUD with zero (wall-clock delta-subtraction; no vendor ever published this number) |
| **G8** | Streaming remainder + GK-2/GK-3, each behind its own design pass | — |

## Kernel requests born from the design (each gets its own design pass)

| | What | Why |
|---|---|---|
| GK-1 | a cross-load map (global object id → Entity) with a declared lifetime | `LoadEntityMap` already keys on an arbitrary u64 — cheap to generalize; unlocks cross-cell refs (F5) |
| GK-2 | a per-column last-changed tick on write | today's still-frame gate `any_changed_since` is an O(live rows) scan documented as "cheap" — a gate one rung below its own documentation; the flecs per-table step is the right middle |
| GK-3 | the post-load fixup seam (F4) | the loader runs no hooks: reverse indexes absent, asset refcounts at 0 — a loaded scene's meshes are retirable mid-game |
| GK-4 | derive-emitted field tables + typed ctors | the bake route that avoids both the rejected EG2 and the unlanded C11 |

## Owner ballots (open; the fork bodies with prices live in DECISIONS.md §Forks)

| # | Question | Recommendation | Blocks |
|---|---|---|---|
| **F1** | The bake route into the byte format: the audited-and-rejected EG2 reflection seam, or macro-time GK-4 now with EG2 as a later upgrade? | (b) GK-4 now | **G1** |
| **F4** | What "loaded" means: the loader runs no hooks — specified post-load fixup pass (a), loader fires attach hooks (b), or forbid hook-dependent components in baked assets (c — untenable, it forbids `MeshHandle`) | (a) fixup registry + census gate | **G6, and the engine's load semantics generally** |
| F2 | Where DataAsset data lives at runtime: entity-shaped dense columns + generated row constants (a) or a new resource region in the format (b) | (a); its VALUES half is the sentence "a table is entities" | G5 |
| F3 | Table form: single-file-per-asset only, or also a table file baking N rows into one dense column | both over ONE schema | grammar |
| F5 | Streaming scope: format-ready-loader-later (cell catalog now, `load_cell`/unload/GK-1 later) | (a) — catalog + attribution + persistent id map are ONE design unit | G6 |
| F6 | `.ui` fate after absorption: migrate + delete in the same campaign, or freeze until an owner-eval of a Gaia HUD | (a) — the diverged-pair cost is already measured | G7 |
| F7 | Mods: ratify "out of v1" explicitly, or accept that a text-mod pipeline ships the bake tool to PLAYERS as a separate executable (re-stating the constraint as "reflection never in the game binary") | ratify explicitly either way — silence here is a default decision | — |

A veto point (decided, owner may veto): first-class `remove` in inheritance is IN the v1 grammar —
default-inexpressibility costs more later (the Unity nested-prefab lesson in miniature).

## Relations

- **aether-v2**: `scene` in Aether stays dev-bootstrap-only; the Gaia binary is the only shipped
  scene form (ratify — the alternative, a shared SceneModel, is priced higher). `link` is the
  shared remap spelling. Component references ride mandatory explicit `stable_name`. The
  "conditional visibility" pressure valve needs its Aether half ratified BEFORE the first designer
  asks (an EnableTag-toggling action idiom), or history says the document side caves.
- **AI-orientation**: AIR-08/09/16/17 bind Gaia from the spec; diagnostics ride the AIR-01 envelope
  from the baker's first commit; `GA####` codes from the shared registry discipline.
- **Out of scope**: mod pipeline (F7), an editor, non-Latin text shaping, graph materials.
