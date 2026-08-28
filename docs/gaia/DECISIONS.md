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
   form. → §Identity, N1.
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
  quaternion, `#RRGGBBAA` → straight-RGBA8, `Px/Pct/Stretch/Auto`, RON-style enums). **Type is
  always dictated by the target Rust field** — the type-directed rule `.ui` already proved; the
  YAML "Norway problem" class is inexpressible, not diagnosed.
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
files; bake is order-independent and byte-identical in any file order) · typed templates with
declared named holes expanded over the typed AST — never text substitution (Paradox `$P$` and SC2
`^token^` are the counter-precedents); `abstract` erased by bake (RimWorld) · `for` over literal
lists and integer ranges with `if` guards and `let`; no functions, no recursion (CUE and Dhall
converged here from opposite sides) · arithmetic/interpolation over same-document values; imports
are literal paths, eager, content-root-only (the Dhall rules that make the dependency graph an
exact fact) · **curves as a first-class asset kind + the `scalable` field type** (curve id ×
multiplier — the GAS `FScalableFloat` valve that kills expression-language pressure; baked into
flat keyframe arrays) · contracts from a closed predicate vocabulary over the record's own fields,
blame = file+span of the violating VALUE · **bake budgets** per file (expansion steps, spawned
fields, template depth, output bytes) with a committed red fixture — "not Turing-complete" is not
the safety property (billion-laughs was pure substitution).

**References to Aether/engine items are by NAME, resolved by bake; unresolvable = bake error.**
Components by mandatory explicit `stable_name` (bake REFUSES the default module-path name — it is
refactor-brittle). Actions, run-conditions, systems, machines — by exported Aether symbol. A
condition in a Gaia file is a **token reference, never an expression** — that is the whole answer
to "how data references logic without becoming code".

**N1 (adversarial pass, adopted):** in the baked binary every reference form is a **name hash**
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
- **Two reference kinds, two syntaxes**: value references (templates, `let`) are lexical,
  copy-semantics; entity references (`@asset/object`) are identity, remapped at load. No config
  language in the survey has object identity at all — one spelling would invite authors to assume
  one behaviour.
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
   template table. All four resolutions at load, once, cold (N1). This also closes the two
   by-name forms the UI dispatch code itself lists as unbuilt — the largest concrete win over `.ui`.
2. **Constants are not bindings**: a provably-constant expression emits component bytes and ZERO
   bind records (Slint's const-propagation + remove-unused); `once` is first-class syntax.
3. **The arrow is sink→source**: no subscriber lists, no dependency nodes; a change-gated system
   asks each sink "did my source change" via the per-row tick the ECS already pays for. The entire
   runtime reactivity of a document = one 4-byte `last_run` tick per bind system.
4. **N2 (adopted over survey 5):** per-binding monomorphized systems are impossible for a
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
8. Forbidden in generated code, with red fixtures: `Or<(Changed<A>, Changed<B>)>` over dense (our
   measured silent never-true) and untracked `Query<&mut T>`.
9. **The benchmark is the STILL FRAME** (the failure UMG's polling names): 200 bindings with
   nothing changing must cost like zero. No vendor has published this number; Gaia gates it by
   wall-clock delta-subtraction.

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

1. Monomorphized-per-binding vs N2 → **N2** (runtime-loaded binaries cannot add systems).
2. Bake `(ComponentId, u8)` into the binary vs N1 → **N1**; the gated-offsets/name-hashes/ungated-
   ordinals criterion above.
3. Patch precedence ambiguity in Bevy's wording → irrelevant for Gaia: later-wins in ladder order,
   one sentence + one pinned test.
4. `remove` in inheritance: VALUES challenge vs in-v1 → **in v1**, owner veto point (asymmetric
   cost of inexpressibility).
5. AI-ORIENTATION.md's "Gaia's type-level substrate CLOSED by serialization decisions" →
   **refuted** by inventory (hooks/resources/ticks/carriers/opt-in-remap uncovered); the line is
   corrected in that file as of G0 — an unqualified closure claim without a census is the
   gate-that-cannot-fail class, in prose.
