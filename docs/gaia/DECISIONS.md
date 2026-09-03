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
- **THREE reference kinds** *(the two-kind rule as ratified 2026-08-28, extended by ballot **GB-3**:
  the third kind was **ADOPTED BY THE OWNER 2026-08-30** and the four-part rewrite ruled the same
  day [delegated]).* The extension is consistent with the original rationale — *one spelling would
  invite authors to assume one behaviour* — and does not reverse it. What makes it a THIRD kind is
  the column the other two do not have: an **asset** reference is the only one whose referent can
  **stop being valid after load**, because streaming retires slots — which under F5's ruling is the
  normal case, not an edge.

  | kind | sigil | at bake | at load | after load |
  |---|---|---|---|---|
  | **lexical / copy** — templates, `let`, styles, inheritance bases | `$` | substituted; **no reference survives into the binary** | nothing to resolve | nothing |
  | **entity** — an authored object's identity | `@` | resolved offline; `UnmappedEntity` lifted from load to bake | **remapped** through the load map | stable for the load's lifetime |
  | **asset** — mesh, curve, string table, font, sound | one glyph, **not yet minted** (below) | resolved to a **path-name hash** | **looked up, not remapped** (`PathIndex`) | **refcounted and revalidated every frame** |

  **Part 1 — the asset-ref spelling is a SIGIL**, applied uniformly and position-independently. The
  invariant bought: **a bare word is never a reference**, anywhere, which makes the dangle check a
  single lexical pass instead of a grammar walk. Rejected: bare strings (a string literal is already
  a non-reference value in this language, and the check would key on a destination field that three
  reference positions do not have) and a typed head (the vocabulary is unclosable — the ground that
  already killed widget sugar). ⚠ **THE GLYPH IS NOT MINTED HERE, and must not be:** it is a **G2
  deliverable, closed JOINTLY with Gaia's arithmetic operator vocabulary**, because the ratified
  rule that whitespace is never significant makes any glyph that can open a binary operator
  ambiguous. Recommendation `~`; the joining is part of the ruling.
  **The dangle check, named:** every sigil-marked token must resolve at bake, and unresolvable is a
  coded `GA####` refusal naming the pack and the path, blamed on the reference's own span.

  **Part 2 — style references are the FIRST kind** (lexical/copy), not the third and not a fourth.
  §Refusals already ratifies *styles are named records by explicit reference, flattened by bake*, and
  a thing flattened by bake leaves no reference in the binary, which is the definition of kind 1. The
  ballot's own example list, which included *"a style record"* under the asset kind, is dropped.

  **Part 3 — `$hole` STAYS, reclassified.** The ballot's premise (*a third reference sigil where two
  are ratified*) is false: templates and `let` are already the **lexical** kind, so `$power` was kind
  1 all along. `$` marks the lexical kind — template parameters **and** `let` bindings — **in every
  position**, which is what lets a generator pick the sigil from the KIND alone with no positional
  knowledge.

  **Part 4 — "a declared node ⇒ `@`" is WITHDRAWN**, replaced by one question with three
  mechanically decidable answers — *what does bake do with this reference?* (1) substitutes it ⇒
  lexical, `$`; (2) records an object id for load remap ⇒ entity, `@`; (3) records a path hash for
  load lookup + refcount ⇒ asset, the new glyph. ⚠ This rule is **F8-neutral and F10-neutral by
  construction**: it never asks whether a node's anchor is a human name or a minted id, and says
  nothing about whether a bundle expands at bake.

  ⚠ **The ballot's GN1 premise is REFUTED.** It asserted that GN1 resolves every reference form to a
  name hash *without* remap, and therefore describes the asset kind. GN1's own first resolution is
  *object id → `Entity` (**load remap**)*, so GN1 is the uniform cost-and-representation law over
  **all three** kinds — kind 1 has nothing in the binary to resolve, kind 2 resolves by remap, kind 3
  by lookup. **Only kind 3 is lookup-without-remap.**

  ⚠ **Riding lines this ruling OWES and does not make here:**
  [`PENDING-SYNTAX-PLAN.md`](PENDING-SYNTAX-PLAN.md) R4/R5/R7 and Tiers 1–2 are regenerated by it,
  and [`LANGUAGE.md`](LANGUAGE.md) still spells a lexical inheritance base as `extends @iron_sword`.
  Both edits belong to **G2**. ⚠ And one FOLLOW-ON FINDING that is not a ballot and is not closed:
  the asset-ref dangle check is structurally unreachable downstream of the sigil pass as the engine
  stands — `AssetServer::load` logs `E0801` and returns a LIVE handle in the `Failed` state which it
  inserts into the path index, and `validate_asset_refs` early-returns unless `free_epoch` advanced,
  which a never-loaded asset never makes happen.
  *Ruling recorded here 2026-09-03 from the working branch; the full ground, the measured glyph
  census and the rejected alternatives live at `feat/threadpool-ke16` `gaia/DECISIONS.md`
  §Identity and references.*
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
     hooks, so a dense arm was silently never true. That was the shipped kernel's real behaviour, the
     ban was the correct call against it, and it is the line that made a KERNEL defect visible to the
     DATA campaign at all.

     **Why it goes.** Its ground is fixed and gated by Aether rung **R0** / backlog **KE1**. Option
     (a)'s only named candidate ground — D4's coupling — was measured and does not survive contact:
     **D4 reserves the Aether *surface* `or(...)`, while this ban governed *generated code*, and this
     document's own ratified GN2 rejects "bake emits Rust into the game build"**. A ban on a shape the
     generator cannot emit, grounded in a reserve on a surface the generator does not write, is a rule
     with no subject. ⚠ It could not be re-grounded on KE13 either: `Or` folds
     `NEEDS_CHANGE_DETECTION` and `EcsMaster::query` const-refuses it, so the banned shape cannot
     reach a `QueryView` at all.

     **What replaces it, so the deletion is not a loss.** The hazard is stated better by the third row
     below — a bind source or change-gate over a dense (or bitset) component — whose ground is still
     live and unfixed. The fixture discipline this ban carried is **kept and re-aimed** onto that row:
     the red fixture asserts the **GENERATOR does not emit a change-gate over a non-signature storage
     kind**, which cannot go green on a kernel commit — the exact failure this list warned about.
     Filed to G7's codegen rules rather than dropped: if a generator ever does emit `Or` over dense,
     the shapes with **no oracle** are `Query<Entity, Or<..dense..>>`, `Added<Dense>` inside `Or`,
     arity > 2 with more than one dense arm, and KE13 on the point-lookup path.

     ⚠ **The deadline this row used to print — "decide before Aether rung R0 lands" — EXPIRED
     UNANSWERED: R0 landed with GB-9 open.** So the ruling reconstructs the record from prose rather
     than from a reproducible failure, and says so. That is the cost the deadline existed to avoid,
     and it is recorded rather than smoothed away — the work order in the same commit that carried the
     deadline already declared R0 *"buildable now, no ballot in the way"*.
   - **A bind source on a dense (or bitset) component.** `any_changed_since`
     (`boyko_ecs` `component_api.rs:403`) resolves per-archetype pools, and non-signature storage
     owns none (`archetype.rs:389-395`), so the gate is **never true** — the sink never updates and nothing is
     logged. Red fixtures cover BOTH spellings: an explicit `kernel (storage = dense)` component
     and a `table`-derived one. ⚠ **The premise of that second spelling was CORRECTED 2026-08-30 by
     F2's ruling and is dated here:** Gaia's `table` bakes to **`StorageKind::Table`**, not to dense,
     so a table row is not the accidental entrance this row assumed. The row's hazard is unchanged
     and still live — a bind source on a dense OR bitset component is invisible to the gate — and the
     dense spelling is reached deliberately, through `kernel (storage = dense)`, or by a row
     component that is ALSO carried by entities outside the table, which is the one case F2 leaves
     to `Dense`. Companion doc fix in the same commit:
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
   subject, and that was established at source and then measured. `ui_bind_discovery` makes ONE
   call and `dynamic_bound_ids` is a **deduplicated set of component TYPES**
   (`register_bound_id`, [`bind_system.rs:56-60`](../../crates/boyko_ui/src/binding/bind_system.rs)),
   so **the binding count is not an input to the timed loop**: 200 or 2000 bindings over the same
   three types give the same id set and the same scan. Measured at HUD scale, the still frame is
   **332 ns** and moves **+0% for 10× the bindings**, **+82% for 2× the rows** and **+101% for 2×
   the bound types** — a threshold on that fixture tracks everything the fixture does not pin.
   `Instant::now()`'s step is 100 ns, so the whole frame is ~3.3 ticks with a 15-25× single-call
   tail, and the red-first delta this would have to resolve (0 → 1 sink write) sits at
   signal-to-noise **0.05-0.50**, never above 0.5. The count gate resolves it exactly, with no
   instrument; **no honest tolerance exists to quote**, and a loose one is a gate that cannot fail.
   **When a clock legitimately returns:** over the **scan itself**, with archetype count,
   bound-type count and row count pinned and the number reported per row rather than per frame —
   which is the bench **GK-2**'s design pass needs, not a companion to this gate.

## Refusals (ratified)

No `gaia!` inline macro twin (Bevy's compile-cost bill is presented; Aether S2) · no interpreter/
reflection/dyn in the shipped load path and no construct with a fallback · no functions, recursion,
out-of-document conditions, world iteration, or reference chains · **no cascade** (styles are named
records by explicit reference, flattened by bake — "which rule won" is a fact, not a computation) ·
no path/name/positional identity · no multiple inheritance · no bespoke patch grammar (AIR-09:
`{node_id, key, value}` edits are applied by a TOOL) · no per-file pragmas that change parsing (the
RON `#![enable]` class) · no comment directives · **no layout at bake** (even Slint solves layout
at runtime; constraints bake into POD components, the solver is a runtime system) · no structural
reactivity · **no external-mod pipeline in v1** — *ratified by the owner 2026-08-30 (ballot F7), and dated
here because until that date this line was a ratified refusal answering half of an OPEN ballot,
which is the settle-by-proximity shape [`CAMPAIGN.md`](CAMPAIGN.md) forbids on this page. The
ruling makes it correct; it did not make it legitimate at the time.* The constraint is recorded
in its NARROW form — no reflection in the GAME BINARY — so a later mod campaign is not blocked
by a sentence that never meant to block it; the format carries layer names from day one, so the
retrofit is additive · no merge driver (mergeability comes from the format: stable ids, keyed
collections, small files, bake as the post-merge validator) · **no second front-end**.

## GB-5 — RULED BY THE OWNER, 2026-08-30: **permit as SEED**

*Recorded on this branch 2026-09-03. The ruling landed on `feat/threadpool-ke16` in `b6c41237`
(2026-08-31), a commit that touched that branch's `gaia/DECISIONS.md` and nothing else — which is why
every index, on both branches, went on printing GB-5 as OPEN. The full ground is at that branch's
`gaia/DECISIONS.md` §GB-5.*

**The ruling.** A scene document **may** declare an engine-derived field. The authored value is the
**initial** value; the engine takes it over if and when its condition holds. The owner's ground, and
it is stronger than the ballot's framing: *"the third is the most logical — it is simply a starting
point in space; obviously this data exists to be manipulated and will not be static."* ⇒ A field the
engine derives is, by definition, a field that changes; "initial value" is its honest semantics, not
a concession.

**Rejected, with prices.** *Refuse* — would also forbid the cases where the authored value works,
which is every entity lacking the driving component. *Permit silently* — the silent-wrong-answer
class this campaign exists to remove. *Refuse conditionally* — requires the baker to reason about the
entity's other components, and for two of the twelve about their **runtime values**, which bake
cannot do.

**Why it generalises across all twelve measured members.** Spatial fields take a seed as a starting
pose, and the engine already ships that exact semantics — a spot light's `direction` is documented as
a SEED that `light_reconcile` overwrites
([`light.rs:1207`](../../crates/boyko_render/src/light.rs)). For `ContentSize.width`/`.height` a seed
is the ONLY correct answer, because until the font loads there is no measurement at all. And it
dissolves the case a refusal could not answer: **`Transform` and `RigidBody` are a polarity pair**
whose author-owned side flips on `Simulated`, a bit gameplay toggles at RUNTIME — under *refuse* the
baker would have to guess which to reject and would be wrong half the time.

**What the ruling still requires, and the form it must NOT take.** An author writing such a field
should be told it is a seed, which is why the derive-emitted field table needs a disposition column —
the thing G1 was blocked from minting until this ballot was answered. ⚠ **It must not be a boolean:**
every one of the twelve is derived CONDITIONALLY, so "this field is engine-derived" is a false
statement about most entities that carry it. ⇒ **The column records the CONDITION and the WRITER**,
not a verdict: *this field may be taken over by `light_reconcile` when the entity has
`GlobalTransform`*. That claims nothing about a particular entity, so it cannot be false.

⚠ **The owner's acceptance condition, recorded as a requirement on the implementation rather than an
assumption:** *"if the marking costs nothing at runtime and is purely for convenience, then yes."*
The column must therefore sit behind the same default-off bake feature as the rest of the bake
machinery — the ratified pipeline is *zero reflection in the shipped load path*. **If it turns out
the column cannot be kept out of the game binary, the condition is not met and this returns to the
owner.**

**Unblocks:** the **G1 field-table freeze**, which was the only thing GB-5 was holding. G1 is
unstarted — `field_table|FieldTable|field_by_name` greps empty across `boyko_macros` and `boyko_ecs`
— so the cost today is one attribute and one column with **zero consumers to update**, and it only
rises.

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
