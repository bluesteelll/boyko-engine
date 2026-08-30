# Gaia — decision log

Every ruling with its rationale and the rejected alternative. This file is the **AIR-17 carrier**:
the decisions inherited from the scene-format campaign live here as ratified lines with their
original (non-AI) rationale AND the AI-orientation cross-note, so nobody re-derives or re-litigates
them. Sources: the two-pass research of 2026-08-28 (external claims carry URLs in the research
record; engine claims were verified line-by-line in this checkout).

## Inherited pipeline (ratified earlier, re-confirmed at source)

- **Own text → build-time bake → binary; reflection only at bake, behind a default-off feature;
  the shipped load path has zero reflection.** All five claimed runtime mechanisms confirmed:
  `resolve_stable_name` (once per type, cold), POB column blits, `load_archetype`/`load_dense_store`,
  `LoadEntityMap` with loud `UnmappedEntity`, per-component `format_version`. The bake tool PRINTS
  the existing `boyko_serialize` format — a second byte format is forbidden.
- **Canonical printer + round-trip gate.** *(AIR cross-note: this is also AIR-07/16.)* Byte-level
  where bytes are compared, but the load-bearing gate compares on the WORLD side with a generated
  comparator — two in-tree incidents prove a hand-listed comparator goes green over divergence.
- **Stable ids.** *(AIR cross-note: AIR-08.)* See §Identity.

## What the inventory found that the claims did NOT cover

1. **The loader runs no hooks** — reverse indexes (`Children`, `LikedBy`) absent engine-wide after
   a load; asset refcounts not incremented (every mesh of a loaded scene sits at refcount 0,
   retirable mid-game). The same document is correct in the dev loop (Commands spawn fires hooks)
   and broken in the shipped game — the worst divergence class, structural TODAY. → fork **F4**.
2. **`MeshHandle(u32)` / `MaterialHandle(u16)` are POB integers that blit a process-local slot** —
   every loud refusal passes and the reference is meaningless after a restart. The single most
   dangerous finding for scenes. Cure: stable asset ids in the binary + a bake lint banning the raw
   form. → §Identity, GN1.
3. No resource region in the format (→ F2). 4. Entity remap is per-field opt-in — hence `link` is
   mandatory grammar. 5. Ticks reset on every load — a streamed cell is a one-frame salvo of every
   `Changed<T>` consumer; documented as initial-apply semantics. 6. `save_world` requires a live
   world (→ F1). 7. Schema drift is lenient-by-default into counters nobody asserts — ratified:
   **strict at bake, lenient at runtime**.

## Language shape

- **One language, three profiles** (`gaia 1 profile=scene|ui|data`): one lexer, one lossless CST,
  one canonical printer, one diagnostics envelope (AIR-01), one identity scheme, one evaluator.
  The profile picks the root schema and the lowering — nothing else. Rejected: three dialects.
- **KDL-shaped node model** (`name positional key=value { children }`) + `/-` slashdash (comment
  out one component/child — the most frequent scene edit). Literals are engine-native (`Vec3`,
  quaternion, `#RRGGBBAA` / `#RRGGBB`, `Px/Pct/Stretch/Auto`, RON-style enums). **Type is
  always dictated by the target Rust field** — the type-directed rule `.ui` already proved; the
  YAML "Norway problem" class is inexpressible, not diagnosed.

- **Colour: the transfer function belongs to the DESTINATION FIELD, not to the literal**
  *(ballot **GB-2** RESOLVED 2026-08-30 by standing rule; full body and the measurements in
  [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29)*. A hex literal is display-referred
  8-bit; where it lands decides what happens to it, and the GK-4 field table declares which. Three
  carrier classes exist in the engine and they disagree, so one rule for the literal was never
  available:
  1. **LINEAR float colour** (`PointLight::color` and the other light colours,
     `boyko_render/src/light.rs:289-360`, all `/// LINEAR rgb color` under `:114` *"All radiometric
     values are LINEAR"*; `MaterialGpu::base_color`/`emissive`, `material.rs:51-69`) → **the sRGB
     EOTF is applied at bake.**
  2. **8-bit encoded RGBA** (`UiBackground::color`/`border_color`, `boyko_ui/src/components.rs:220-223`;
     `UiText::color`, `text/components.rs:46-47`) → **the identity**, because source and destination
     are the same space. Measured: decode-then-re-encode drifts **0 of 256 bytes**, so the uniform
     rule changes no shipped UI colour. Note that "STRAIGHT RGBA8" in those doc comments means
     NON-PREMULTIPLIED (`components.rs:211` names `premultiply_rgba8` as the next step), not
     "undecoded" — the phrase says nothing about a transfer function.
  3. **Device-encoded packed** (`ParticleEffect::color_keys`, `particle_effect.rs:120-143`, byte
     order `0xAABBGGRR`) → **a coded `GA####` bake refusal.** Its own doc records two shipped
     presets authored wrong by exactly the `0xRRGGBBAA` habit, *"and neither was caught by
     anything"*; Gaia does not become the fourth entrance to that mistake.

  Rejected: raw bytes into a linear field — wrong by up to **12.92×** (byte 3) and **0.287**
  absolute (byte 136), and **1.557× / 3.371×** on green/blue for the corpus's own `#FFB35CFF`, a
  hue shift no consumer attributes to the baker. Also rejected: one global "always decode", which
  corrupts every UI colour in the opposite direction.
  ⚠ **Fixture constraint, and it is load-bearing: the two routes agree at exactly two points, byte
  0 and byte 255.** A colour fixture written with `#FFFFFFFF` or `#000000FF` is a gate that cannot
  fail. Mid-domain only; `128` (raw `0.501961` vs decoded `0.215861`) is the recommended witness.

- **Colour arity is checked against the target's arity, mirroring the shipped Aether rule.**
  Widening 3 → 4 supplies alpha `1.0`; **narrowing 4 → 3 is a coded refusal, never a silent alpha
  drop** (the silent drop is Unity's class, banned by name in §Inheritance below). Blame lands on
  the literal's own span. The precedent is in-tree and shipped: `ColorLit`
  (`crates/aether_lang/src/parse.rs:1177-1213`, `expand.rs:891-905`) does exactly this, down to
  putting the error on the tuple *"because neither the key nor any single component is the thing
  that is wrong"*. Consequence: `#RRGGBB` is the 3-component spelling — an ADDITION to an
  illustrative literal list, not a reopen (the one list this corpus closes says *"the list is
  exhaustive"*, and it is §The logic line's, not this one).

  ✅ **RULED BY THE OWNER, 2026-08-30 — and the ruling dissolves the reopen question rather than
  answering it.** Owner: *"`#RRGGBB` is just `#RRGGBBAA` where `AA` is maximal."* So it is not a
  second kind of literal at all; it is the same literal with a **defaulted field**. There is no
  surface to widen, and AIR-10's reopen bar does not engage.

  ✅ **And the arity direction the clause did NOT cover is ruled too: REFUSE AT BAKE.** Owner,
  2026-08-30. The rule above says what 6 digits mean at a 4-component field. The engine also has the
  opposite shape — `PointLight.color` is `[f32; 3]` — and an 8-digit literal there must be a
  **coded bake refusal naming the field**, never a silent alpha drop. Ground: dropping it is a
  silent wrong answer, the author wrote a transparency and the baker ate it; and it is the same
  disposition the owner ratified for **AB-11** on the same day, for the same reason — a refusal that
  dissolves a class beats a documented sharp edge.

  ⚠ *Superseded record, kept because it is why the question reached the owner at all:* this ruling
  was delegated, and TWO independent reviewers both read the `#RRGGBB` clause as widening a ratified
  surface rather than annotating an illustrative one. The ruling's ground for treating it as an addition is stated above and is not
  frivolous: the corpus marks exactly one literal list as closed, and it is a different list. But a
  delegated ruling that exempts itself from the reopen bar **on its own reading of which list it
  touches** is settling a fork by proximity, which is the defect this whole ballot list exists to
  prevent. **The question put to the owner is narrow:** does adding a 3-component `#RRGGBB` spelling
  beside the ratified `#RRGGBBAA` count as a reopen requiring AIR-10's trigger-and-measurement bar?
  Everything else in GB-2 — the sRGB decode at bake, verified against every colour-typed field —
  stands regardless of the answer and is not reopened by this mark.
- **`.ui` is absorbed, not coexisted with.** It already violates "never a second format" (own
  version constant, an inverted float rule, a printer losing 10 of 19 components under a gate
  structurally blind to the loss, a vocabulary that cannot express a visible pixel). What survives
  verbatim: the reconcile-by-name+ordinal hot-reload, the type-directed leaf parsers, the bind
  path, the bar quantization, the action seam. Hot reload becomes the `gaia_dynamic` backend
  behind a default-off feature with a ship-absence gate (the reflect-campaign precedent: "links
  nothing" is proven by a gate, not a sentence). Its measured defects share ONE cause — four
  hand-maintained lists of one vocabulary — cured by a single generated vocabulary manifest.

## The logic line: total, eager, closed

The evaluator is **total by construction** (no recursion form exists), **eager** (every contract
runs on every bake whether or not the field is read — Jsonnet's lazy assertions are "a gate that
cannot fail" promoted into language semantics), **closed** (anything bake cannot fully resolve is a
bake error, never a fallback — the qmltc contract, reachable because the surface is designed for
the compiler rather than the other way around).

Allowed (the list is exhaustive): native literals · closed records against the Rust type (unknown
key = authored-span error; CUE-closedness without the lattice, free because the schema is a known
type) · defaults + a **closed priority ladder** `base < variant < tuning < debug` (named layers,
never free numbers — the `!important` race; two writes of one field on one layer = error with both
files; bake is order-independent and byte-identical in any file order — and see §Layers and depth
below for what the ladder does NOT rank) · typed templates with
declared named holes expanded over the typed AST — never text substitution (Paradox `$P$` and SC2
`^token^` are the counter-precedents); `abstract` erased by bake (RimWorld) · `for` over literal
lists and integer ranges with `if` guards and `let`; no functions, no recursion (CUE and Dhall
converged here from opposite sides) · arithmetic/interpolation over same-document values; imports
are literal paths, eager, content-root-only (the Dhall rules that make the dependency graph an
exact fact) · **curves as a first-class asset kind + the `scalable` field type** (curve id ×
multiplier — the GAS `FScalableFloat` valve that kills expression-language pressure; baked into
flat keyframe arrays) · contracts from a closed predicate vocabulary over the record's own fields,
blame = file+span of the violating VALUE · **bake budgets** per file (expansion steps, spawned
fields, template depth, output bytes) — "not Turing-complete" is not the safety property
(billion-laughs was pure substitution).

**One committed red fixture PER AXIS, not one for the set.** A single fixture proves at most one
axis and lets the other three ship unguarded while the row reads green. Each of the four fixtures
asserts the `GA####` code that names ITS OWN axis (a fixture satisfied by any budget diagnostic is
a gate that cannot distinguish the thing it guards), and each budget's numeric value is recorded
beside its own fixture rather than in prose here, so the value and its guard cannot drift apart.
The expansion-steps axis carries the loop-refusal wording
([PENDING](PENDING-SYNTAX-PLAN.md) §Tier 2, the `for … if …` clause) as its diagnostic. The
fixture set is mirrored into G4's Gate column in CAMPAIGN.md.

### Layers and depth — two axes, resolved in that order

*(Ballot **GB-1** RESOLVED 2026-08-30 by standing rule; body in
[`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29. The ladder above is untouched — no rung
added, removed, renamed or reordered.)*

- **A template EXPANSION occupies NO ladder layer.** It is one step on the **inheritance-depth**
  axis, which this file already carries separately: *single parent, no diamonds*; *a patch target
  resolves against the immediate base*; and `template depth` as one of the four bake budgets above.
  A template application ranks exactly where `extends` ranks — **below the applying node's own
  writes, at the same layer**.
- **Resolution is depth first, then ladder.** Within one layer a node's own write beats what it
  expanded; across layers the flattened node from the lower layer loses field-wise to the higher.
- **An UNLABELED write occupies `base`** — the layer whose name already means "the thing itself",
  and the bottom, so authored content stays overridable by every pack above it.
- **The two-writes-one-field error therefore means same DEPTH and same LAYER.** This is what keeps
  the commonest authoring act in the language — apply a template, override one field — from being a
  bake error, which is finding **K2**, now dissolved rather than diagnosed.

**Rejected: a ladder rung for expansion** (`template < base < …`). Not a taste call — it is not
expressible. `template depth` is a **budgeted** quantity, and a budget bounds something otherwise
unbounded: a template applying a template needs one rung per level, and a closed four-name enum
cannot carry an unbounded count. It would also break the ladder's closure, which AIR-10 protects.
**Rejected: an implicit fifth "unlabeled" rung** — an unranked write makes *which write won* a
computation over file order, which turns G4's own `bake(files) == bake(shuffle(files))` gate either
red or vacuously green.

**Recorded, deliberately NOT decided here:** in a non-`base` pack every line must repeat
`layer=tuning`, and a forgotten `layer=` lands silently at `base`. A file-level layer default is the
candidate cure and is a **grammar** question owned by **G4**.

**Spelling-independent:** the ruling is about precedence, not words. It holds under either **GB-4**
option and under whatever anchor the template-application fix takes (PENDING Tier 1 proposes
`apply`). It is scoped to **template** expansion; whether a **bundle** expands at all is **F10**,
the owner's, and untouched — if F10 rules "expand", the depth rule applies to it unchanged.

**References to Aether/engine items are by NAME, resolved by bake; unresolvable = bake error.**
Components by mandatory explicit `stable_name` (bake REFUSES the default module-path name — it is
refactor-brittle). Actions, run-conditions, systems, machines — by exported Aether symbol. A
condition in a Gaia file is a **token reference, never an expression** — that is the whole answer
to "how data references logic without becoming code".

**GN1 (adversarial pass, adopted; `N1` before the id-namespace pass):** in the baked binary every reference form is a **name hash**
resolved once at load, cold — the exact analogue of `resolve_stable_name` one level down. Raw
build-local ordinals (`ComponentId` mint order, derive-order `u8`s, `Actionlike::index()`,
`FontId`, `Assets` slots) are **unrepresentable in the file**; a bake lint over the closed list of
unstable field types is mandatory. The precise class criterion: offsets GATED by
`layout_fingerprint` are legal (fail loud, cured by rebake); name hashes are legal (survive
reorder); the crime is **ungated ordinals that fail silently**. Template patches never reach the
binary at all — the binary carries final flattened rows.

## Inheritance / variants

Single parent, no diamonds ("sword AND container" is component composition, which an ECS already
is). Instance = variant = derived asset = one construct (base ref + sparse per-field patches)
applied by bake in ladder order. Both forms shipped and named apart: `extends` (live link) and
`copy` (bake-time snapshot) — RimWorld and Factorio each shipped one and their ecosystems invented
the other. Patch targets bind to stable object ids, never name-paths; a patch whose target vanished
is a bake error, never a silent drop (Unity's silent drop is exactly our measured class).
**First-class `remove`** (component / child) from day one. Encapsulation closed by default;
overriding deeper than declared props needs a recorded `open` keyed by stable id. Variant chains:
a patch target resolves against the **immediate** base — one sentence plus one pinned test (the
ambiguity Unity left undocumented). Bake emits a **provenance sidecar** (asset → field → ordered
(pack, file, span) list) — a dev/CI artifact absent from release.

## Identity and references

*(= AIR-08 ratified, with one conscious inversion.)*

- **Asset id**: author-declared, pack-namespaced, written **IN the file** — not a sidecar: Gaia
  owns its format, so the id must be inseparable from content by any file operation (closes both
  lost-`.meta` and copy-mints-a-clone). The path is a human hint, never identity. For NON-Gaia
  binary assets (`.png`, `.glb`) the sidecar returns, with a hard fail on absence — recorded,
  not fully designed here.
- **Object id**: file-local, author-visible, stable under edit/reorder; never positional, never
  minted by bake or an importer (Godot's importer-minted ids change on every reimport). Minting:
  `gaia fmt --assign-ids` writes ids INTO the text; bake refuses a referenced node without one.
  **An anonymous node can never be the target of a cross-file reference, an override, or a patch**
  — ordinals re-key on insertion (the Terraform-count class).
- **Two reference kinds, two syntaxes** *(ratified)*: value references (templates, `let`) are
  lexical, copy-semantics; entity references (`@asset/object`) are identity, remapped at load. No
  config language in the survey has object identity at all — one spelling would invite authors to
  assume one behaviour.
- ⚠ **Open ballot GB-3 — the taxonomy is short one kind.** An **asset** reference (a curve, a
  string table, a style record, a mesh) is neither of the two above: it is not copied lexically,
  and it is not an entity id remapped by `LoadEntityMap`. Today it has no ruled spelling, so the
  two-kind rule does not tell an author which behaviour to assume for the case the language uses
  most. Recording this as an **extension consistent with the ruling's own one-spelling-one-
  behaviour rationale, NOT a reversal of it** — but it still amends ratified text, so it goes to
  ballot rather than being written in. The question, in three parts:
  1. **Asset-ref spelling** — (a) its own sigil, (b) a typed head, or (c) bare strings. Option (c)
    needs a separate answer for the dangle check (what refuses a reference to an asset that is not
    there), because a bare string carries no marker for the checker to key on.
  2. **Which kind GN1's name-hash covers, stated explicitly.** GN1 resolves every reference form in
    the binary to a name hash at load — WITHOUT remap. That is the asset kind's behaviour, and the
    ratified text never says so.
  3. **Style references** (K15) — whether they are the asset kind or a fourth thing.
  **Settle jointly with the `$hole` sigil (K3) and the style-reference disposition (K15)**, so the
  reference rule is rewritten exactly once and syntax ruling
  [**PENDING R4**](PENDING-SYNTAX-PLAN.md) is regenerated once — the Gaia ruling series, **not**
  Aether rung R4. Blocks **G2, G3** and [PENDING](PENDING-SYNTAX-PLAN.md) Tiers 1–2.
- `link` is mandatory grammar for entity-reference fields — the difference between a loud
  `UnmappedEntity` and a silently stale id.
- Content hashes are integrity/cache only, never identity; the freeze/cache key is a hash of the
  **normalized baked image**, never source bytes (the Dhall semantic-hash rule) — closes our
  measured raw-byte-hash-is-a-checkout-hash class by construction. EOL normalization happens in
  the lexer, before the CST.
- One id space across scenes and UI documents (UI already holds cross-document entity references).
  Bake resolves every cross-file reference offline — `UnmappedEntity` lifted from load to bake.

## UI bindings — the zero-cost lowering

The answer is already shipped and gated at zero allocations in this repo; Gaia generalizes it:

1. A binding bakes into a **POD bind-record component**: object id → `Entity` (load remap),
   `stable_name` hash → `ComponentId`, field-name hash → `u8`, template → an id in the baked
   template table. All four resolutions at load, once, cold (GN1). This also closes the two
   by-name forms the UI dispatch code itself lists as unbuilt — the largest concrete win over `.ui`.
2. **Constants are not bindings**: a provably-constant expression emits component bytes and ZERO
   bind records (Slint's const-propagation + remove-unused); `once` is first-class syntax.
3. **The arrow is sink→source**: no subscriber lists, no dependency nodes; a change-gated system
   asks each sink "did my source change" via the per-row tick the ECS already pays for. The entire
   runtime reactivity of a document = one 4-byte `last_run` tick per bind system.
4. **GN2 (adopted over survey 5; `N2` before the id-namespace pass):** per-binding monomorphized systems are impossible for a
   runtime-loaded binary; the honest ceiling is a **bindable-type set closed at engine compile
   time** (`register_bindable::<C>`) with open binding INSTANCES through the already-built
   type-erased fn-pointer arm. The alternative (bake emits Rust into the game build — Slint's
   model) turns every data edit into a recompile and defeats the point of a data format.
5. Structure bakes; only VALUES react (the props-vs-patches law). A structural change is an
   explicit document/subtree swap via an action, not a binding. No reactive conditionals or child
   lists in v1.
6. Every generated sink is **set-if-changed** (documents the real cost of Mut-deref-without-change
   instead of hiding it). Quantization stays in systems.
7. Two-way = a second opposed system gated on the UI-value tick, schedule-ordered — no shared
   cell, no cycle possible.
8. Forbidden in generated code, with red fixtures — three rows, each with its own ground:
   - **Untracked `Query<&mut T>`.** Ground: `&mut T` stamps no change tick, so a sink written
     through it is invisible to every downstream `Changed<T>`. Untouched by any rung — this row
     stands on its own regardless of what happens to the two below.
   - ~~**`Or<(Changed<A>, Changed<B>)>` over dense.**~~ **DELETED 2026-08-30 by ballot GB-9's
     ruling, option (b) — with the record of why it existed, which is what option (b) required.**

     **Why it existed.** Ground as recorded 2026-08-28: the `Or` filter did not override the dense
     hooks, so a dense arm was silently never true. That was the shipped kernel's real behaviour;
     the ban was the correct call against it, and it is the line that made a *kernel* defect visible
     to the *data* campaign at all.

     **Why it goes.** Its ground is fixed and gated — Aether rung **R0** / backlog **KE1** landed
     2026-08-29 (`crates/boyko_ecs/tests/ke1_or_dense_blindness.rs`, **8/8 green** on 2026-08-30, 7
     of 8 red before the fix); `impl_or_filter_tuple` now folds `HAS_DENSE` and forwards
     `resolve_dense` per arm. Option (a)'s only named candidate ground — **D4's coupling** — was
     measured and does **not** survive contact: **D4 reserves the Aether *surface* `or(...)`, while
     this ban governs *generated code*, and this document's own ratified **GN2** (item 4 above)
     rejects "bake emits Rust into the game build"**. A ban on a shape the generator cannot emit,
     grounded in a reserve on a surface the generator does not write, is a rule with no subject.
     ⚠ **And the ground could not be moved to KE13 either**, which is the answer a careless ruling
     would have reached: `Or` folds `NEEDS_CHANGE_DETECTION` over its members and
     `EcsMaster::query<D, F>()` opens with `const { eval_query_no_change_detection::<D, F>() }`, so
     `Or<(Changed<A>, Changed<B>)>` **cannot reach a `QueryView` at all** — KE13 is a
     `QueryView::get`/`get_mut` defect over a dense `With`/`Without`, a different shape.

     **What replaces it, so the deletion is not a loss.** The hazard this row aimed at is stated
     better by the **third row below** — a bind source or change-gate over a dense (or bitset)
     component — whose ground (`any_changed_since` / `get_component_changed_tick` blind to dense,
     Gaia **GK-2**) is **still live and unfixed in the tree**. This row was always the weaker
     statement of the same hazard: aimed at one filter shape instead of at the storage kind. The
     fixture discipline the ban carried is **kept and re-aimed onto that row** — the red fixture
     asserts the **GENERATOR does not emit a change-gate over a non-signature storage kind**, which
     cannot go green on a kernel commit, the exact failure this list warned about.

     **Filed to G7's codegen rules rather than dropped** — deleting a ban must not delete the
     knowledge. If a generator ever does emit `Or` over dense, these are the shapes with **no
     oracle**, per R0's own landing note: `Query<Entity, Or<..dense..>>` (the empty-include branch),
     `Added<Dense>` inside `Or`, arity > 2 with more than one dense arm, and **KE13** on the point-lookup
     path. `par_iter` / `for_each_chunk` need no rule: they now **compile-refuse** a dense-armed
     `Or`, which is loud, not silent.

     **Cross-links, kept because the coupling worked.** The back-pointers this line asked for were
     written and did their job — [`../aether-v2/KERNEL-BACKLOG.md`](../aether-v2/KERNEL-BACKLOG.md)
     KE1, [`../aether-v2/CAMPAIGN.md`](../aether-v2/CAMPAIGN.md) R0, and the two Aether-side twins
     ([`../aether-v2/DECISIONS.md`](../aether-v2/DECISIONS.md) `D4`,
     [`../aether-v2/CONSTRUCTS.md`](../aether-v2/CONSTRUCTS.md) §`system`). ⚠ **The deadline
     ("decide before R0 lands") did expire unanswered** — R0 landed with GB-9 open — so this ruling
     reconstructs the record from prose rather than from a reproducible failure, and says so. That
     is the cost the deadline existed to avoid, and it is recorded rather than smoothed away.
     Full body, measurements and rejected alternative:
     [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29.
   - **A bind source on a dense (or bitset) component.** `any_changed_since`
     (`boyko_ecs` `component_api.rs:403`) resolves per-archetype pools, and non-signature storage
     owns none (`archetype.rs:389-395`), so the gate is **never true** — the sink never updates and nothing is
     logged. Red fixtures cover BOTH spellings: an explicit `kernel (storage = dense)` component
     and a `table`-derived dense column, since Gaia's own `table` bakes to dense — the second
     spelling is the one an author reaches by accident. Companion doc fix in the same commit:
     the `any_changed_since` doc comment (`component_api.rs:386-389`) claims the scan is bounded
     to hosting archetypes, which is false for exactly this case. The remedy (a bake refusal keyed on the GK-4 storage kind vs
     routing dense through `DenseStore` ticks) is the implementer's engineering choice, per the
     standing rule.
9. **The benchmark is the STILL FRAME** (the failure UMG's polling names): 200 bindings with
   nothing changing must cost like zero. No vendor has published this number. **The gate is a
   COUNT, not a clock**: bind-sink writes executed == 0 (observable because every sink is
   set-if-changed, item 6) and the tick slots read by `any_changed_since` pinned to the fixture's
   expected value; red-first by dirtying exactly one source, after which both counters move.
   Wall-clock delta-subtraction is not falsifiable at this scale and cannot say WHICH work
   disappeared. AIR-12 is cited here as the **precedent for counts-over-exit-code**, not as an
   existing ruling over a runtime bench — no such ruling exists.

   **GB-7 RESOLVED 2026-08-30 by standing rule: NO wall-clock companion.** The count gate stands
   alone. The ground is not "a clock is noisy" — it is that a clock does not measure this gate's
   subject, and that was established at source and then measured:
   - `ui_bind_discovery` (`boyko_ui/src/binding/bind_system.rs:75-89`) makes ONE call, and
     `dynamic_bound_ids` is a **deduplicated set of component TYPES** (`register_bound_id`,
     `:56-60`). `ui_bind_apply` (`:98-105`) returns immediately when `!dirty`, so a still frame is
     discovery only. **The binding count is not an input to the timed loop**: 200 or 2000 bindings
     over the same three types give the same id set and the same scan.
   - Measured at HUD scale (40 archetypes / 3 types / 200 rows; medians of 60 × 2000-call batches,
     two runs agreeing to <1%): the still frame is **332 ns**, and it moves **+0%** for 10× the
     bindings, **+8%** for 2× archetypes, **+46%** for 5× archetypes, **+82%** for 2× rows and
     **+101%** for 2× bound types. A threshold on that fixture tracks everything the fixture does
     not pin and nothing its own title names.
   - `Instant::now()`'s median step here is **100 ns** — the whole still frame is ~3.3 ticks, with a
     15-25× single-call tail (median 200-300 ns, max 4600-5800 ns). The only form reaching ±1.7%
     across processes times **200 000** frames, which is a microbenchmark of `any_changed_since` —
     **GK-2's** subject, not G7's — and even that spread ranged 36%-83% across runs: the noise floor
     is not a constant.
   - The red-first delta this would have to resolve is **0 → 1 sink write**, at a single-frame
     signal-to-noise of **0.05-0.50**, never above 0.5. The count gate resolves it exactly, with no
     instrument. **No honest tolerance exists to quote**, and a loose one (e.g. "under 1 ms",
     3000× the measured cost) is a gate that cannot fail — worse than no clock, because it is
     counted as coverage.

   **When a clock legitimately returns:** over the **scan itself**, with archetype count, bound-type
   count and row count pinned and the number reported per row rather than per frame. That is the
   bench **GK-2**'s design pass needs to justify a per-column tick, and it is not a companion to
   this gate.

## Census discipline — ballot GB-8, ruled 2026-08-30 [delegated]

The entry carries the ballot's id, as the Aether log does for its sequencing rulings: this is a
**precedent about gates**, not a language line, and it touches no ratified item.

**GB-8. (1) No per-site waivers, at any of the four censuses, ever. (2) The AIR-id census widens to
all of `docs/` and lands at G0. (3) The LINK census is a SEPARATE deliverable and lands at Aether
R8.**

*Measured first, because the ballot's cited precedent is not this tree's number.* `188 of 302`
appears in four corpus files and in the census test's doc comment, and **no in-tree gate produces
it** — `tests/internal_docs_anchors.rs` prints neither number. Run live (`cargo test -p boyko-engine
--test internal_docs_anchors -- --nocapture`, 2026-08-30, 5 passed): **735 anchors checked, 116
waived** — `ARCHITECTURE.md` 6/0, `FEATURE_MAP.md` 222/7, `SYSTEMS.md` 330/20,
`MESHLET-VIRTUAL-GEOMETRY-PLAN.md` **177/89**. The aggregate is 15.8%, not 62% — **and the precedent
survives stronger, not weaker**: the waiver did not spread, it concentrated **entirely** in the one
document admitted under the allowance, which now waives **50.3%** of its anchors, and the test's own
head says a waived anchor keeps *"neither shape nor identity"*.

*The census that landed at `01a4436e` is not one scope but four.*
[`tests/gaia_g0_citation_census.rs`](../../tests/gaia_g0_citation_census.rs), 4 tests, green and
non-vacuous when run: the **AIR** census over `docs/gaia` + `docs/aether-v2` (18 definitions, 119
citations across 13 files); the **KE/KM** census **already corpus-wide** over all **352** markdown
files under `docs/` (16 rows, 111 citations); the retired-form census over the same 352 under a
narrowed predicate (34 backlog-referencing lines, 0 offenders); the staleness census over the 1 file
marked `ratified-stale`. **None carries a waiver list.**

*Widening the AIR census is green at zero remediation*: the `AIR-##` citations outside G0's two
directories sit in 5 files (`OPEN-QUESTIONS.md`, `ru/OPEN-QUESTIONS.md`,
`AETHER-GAIA-REVISION-2026-08-29.md`, `FEATURE_MAP.md`, `AETHER-V1-SURFACE-REVIEW.md`) and **every id
cited lies inside the carrier's `AIR-01..AIR-18`**, so none of them is a repair. ⚠ **The count is
deliberately not pinned**, and the reason is this ruling's own footprint: it read 21 before the
ruling was written and 40 after, because the ruling text cites `AIR-06` — and the in-scope count
itself moved 114 → **119** between the two runs of this same pass. **The property is the ruling; the
number is an observation with a timestamp.**

*The LINK half had never been measured and is the half with a cost.* Over `docs/gaia` +
`docs/aether-v2`: 146 relative markdown targets, **0 dead**. Over all of `docs/`: **1687 targets, 59
dead across 11 files, 44 of them in `docs/AUDIT-2026-05-23.md`** (the other 15: 8 in `docs/plans/`,
5 in `docs/archive/`, 2 in `docs/diagnostics/`). `docs/archive/` and `docs/plans/` stay **in scope** —
excluding them would be the scope-statement route and it is not needed, since only 13 of the 59 live
there.

*Where a property is not decidable as written*, the remedy is the one this census file already
practises and states at its own site: **narrow the predicate and print the narrowing in the failure
message** (its retired-form test replaces ~1400 per-site dispositions with one decidable rule). A
**scope statement** ("this census covers directory X") is not a waiver; a per-site skip list is.

*Rejected, with the price.* **Per-site waivers so the whole thing lands in one commit** — priced on
the sibling gate: 50.3% abdication in the document that used the allowance, and `check_anchor`
returns at the waiver branch *before* the shape test, so a waived anchor that is simply **wrong**
about which line holds the symbol still passes. **Give the whole census to R8** — a green, free,
one-constant widening (`G0_DIRS` → `markdown_under("docs")`) waits behind R3 and R8 while the KE/KM
half in the same file already contradicts that placement by running corpus-wide from G0 today.
**Land id and link at one rung** — the free half is held hostage to 59 repairs, which is how a gate
gets deferred until it is convenient.

*Where it lands.* The id half becomes **work on G0**, and Aether **R8** receives the link census with
its red-first evidence already measured. ⚠ This ruling **adds work; it does not unblock a rung** —
G0's row is still held by owner ballots **F1** and **F4**, and neither is touched here.

## Refusals (ratified)

No `gaia!` inline macro twin (Bevy's compile-cost bill is presented; Aether S2) · no interpreter/
reflection/dyn in the shipped load path and no construct with a fallback · no functions, recursion,
out-of-document conditions, world iteration, or reference chains · **no cascade** (styles are named
records by explicit reference, flattened by bake — "which rule won" is a fact, not a computation) ·
no path/name/positional identity · no multiple inheritance · no bespoke patch grammar (AIR-09:
`{node_id, key, value}` edits are applied by a TOOL) · no per-file pragmas that change parsing (the
RON `#![enable]` class) · no comment directives · **no layout at bake** (even Slint solves layout
at runtime; constraints bake into POD components, the solver is a runtime system) · no structural
reactivity · no external-mod pipeline in v1 (the format carries layer names from day one so the
retrofit is additive) · no merge driver (mergeability comes from the format: stable ids, keyed
collections, small files, bake as the post-merge validator) · **no second front-end**.

## Rejected models, for the record

- **CUE's lattice**: its power is reconciling schemas from uncontrolled sources, which Gaia does
  not have (the schema is a known Rust type); its cost is a multi-year evaluator; and commutative
  unification makes "whose write won" a computation rather than a fact. The ordered ladder is
  printable, diffable and resolvable.
- **BSN's closures**: the direct cause of the 121k-char debug symbol and of `.bsn` never shipping
  as a file format — a closure cannot be printed, diffed or round-tripped. Gaia patches are data.
- **Slint's runtime**: the compilation model is the reference; the property graph is not.
- **Sidecar ids for Gaia's own files** (survey 1's position): Unity's sidecar exists because Unity
  does not own foreign formats; Gaia owns its text. In-file wins; the sidecar returns only for
  foreign binaries.

## Disagreement resolutions (kept so they are not re-litigated)

1. Monomorphized-per-binding vs GN2 → **GN2** (runtime-loaded binaries cannot add systems).
2. Bake `(ComponentId, u8)` into the binary vs GN1 → **GN1**; the gated-offsets/name-hashes/
   ungated-ordinals criterion above.
3. Patch precedence ambiguity in Bevy's wording → irrelevant for Gaia: later-wins in ladder order,
   one sentence + one pinned test.
4. `remove` in inheritance: VALUES challenge vs in-v1 → **in v1**, owner veto point (asymmetric
   cost of inexpressibility).
5. AI-ORIENTATION.md's "Gaia's type-level substrate CLOSED by serialization decisions" →
   **refuted** by inventory (hooks/resources/ticks/carriers/opt-in-remap uncovered); the line is
   corrected in that file as of G0 — an unqualified closure claim without a census is the
   gate-that-cannot-fail class, in prose.
