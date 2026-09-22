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

  ✅ **F1 RULED BY THE OWNER, 2026-08-30 — the bake route is macro-time GK-4, taken NOW.** Owner:
  *"Do it properly right away. But bear in mind the world must support streaming."* Both halves
  bind. **The route:** derive-emitted name-keyed field tables + typed constructors in
  `boyko_macros` (rung **G1**) — not the audited-and-rejected EG2 reflection seam, and not
  sequenced behind it. Gaia detaches from EG2 and from the unlanded C11 rather than waiting on
  either. **The constraint:** no part of the bake design may foreclose streaming, which is why
  **F5** lands *with* the scene profile instead of after it (§Load semantics below).
  **Rejected: decide EG2 first, because it unblocks more than Gaia.** Price: G1 waits on a seam
  this campaign does not need, and the ballot's own framing conceded the wait was the whole
  question; the owner declined it in as many words.
  ⚠ **One inference the ballot drew from `RequiredCtor` does not survive the ruling, and it matters
  downstream.** The ballot argued that `unsafe fn(dst: *mut u8)` makes ctor-form `#[require]`
  unbakeable *in principle*. Measured after the ruling: a GK-4 baker is a **Rust program linked
  against the derive-emitted tables**, so it resolves and calls that fn pointer trivially — a
  downstream crate did exactly that through today's public `required_ctor_in_set`
  (`crates/boyko_ecs/src/ecs/core/serialize/mod.rs:51`) and got the ctor's value. What cannot call
  a fn pointer is the *Gaia evaluator over text*, which is a different claim about a different
  program. §Load semantics takes the corrected version as its ground.
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

  ✅ **F6 RULED BY THE OWNER, 2026-08-30: option (a) — MIGRATE the existing `.ui` documents to the
  Gaia `ui` profile and DELETE the old format in the same campaign.** Rejected: freeze `.ui` and
  decide after an owner-eval of a real Gaia HUD. Price of the rejected option, and it is already
  measured in this repository rather than argued: two authoring formats for one subsystem is the
  diverged-pair state — the reader cannot tell which is current and finds out by acting on the
  stale one (`docs/ru/` carries the same rule for the same reason). Price of the ruling, stated so
  it is not discovered later: **the migration is done blind.** G7's prerequisites — a windowed UI
  pass and a real `UiPlugin` — do not exist, so nothing renders a Gaia HUD to judge before the old
  format is gone. That price rides **G7**, and G7's row records it.

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

### The instance spelling — ballot GB-4, ruled 2026-08-30 [delegated]

*(Body in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29. The owner's word on this
ballot was **"давай"** — delegation, not a selection: he named no option and the ballot text
carries no recommendation for the word to point at. It is therefore decided under the standing
perf/architecture rule and said so here, rather than attributed to him.)*

**Option (a): linkage-in-slot, and the linkage word is MANDATORY.** One head keyword, `instance`;
the linkage word occupies a fixed modifier slot with no default:

```
instance <name> extends|copy <base-ref>
```

`from` is **deleted**. Field order (name, then linkage, then base) is endorsed but is **G2**'s
grammar work, not this ruling's — under §Identity's sigil rule the slots are disambiguated by
sigil, not by position.

**Ground 1 — (b) mints two grammars for one ratified construct.** This section already ratifies
*"Instance = variant = derived asset = one construct"*. Option (b) gives that one construct two
heads (`instance` for live, `copy` for snapshot), which is the shape **AIR-15** rejects by name —
*"two grammars per construct — the `at` lesson"* — and it rejects it on the AI-orientation axis,
the axis this ballot was to be judged on.

**Ground 2 — (b) turns a modifier change into an anchor mutation.** Under (a), "make `wall_east` a
snapshot" rewrites **one token in a fixed slot**; the head and the name are untouched, so a patch,
a diff or a tool edit keyed on *the `instance` node named `wall_east`* still resolves afterwards.
Under (b) the same semantic change rewrites the node's **head keyword**, and **AIR-09**'s ratified
patch model is `{node_id, key, value}` edits applied by a tool — a head is neither a key nor a
value, so (b) puts linkage outside the model's reach.

**Ground 3 — (b) has no refusal for the omitted case; (a) does.** Under (a) a missing linkage word
is a coded refusal at a known span over a closed two-way choice. Under (b), `instance …` alone is
legal and means *live*, so an author who meant a snapshot and forgot gets a silent wrong-linkage
node — this repo's standing defect class, and the identical consequence §Layers and depth already
bought once for a forgotten `layer=`. Defaulting is also unavailable on the section's own ground:
*RimWorld and Factorio each shipped one and their ecosystems invented the other*, so neither form
is the natural default and a default is a coin flip made on the author's behalf.

**Ground 4 — the two axes stay orthogonal, and the slot is already load-bearing at a second
construct.** The data profile already spells a linkage word in a slot on `row`. Under (a),
`extends|copy` is **one** modifier slot reused at `instance`, at `row`, and at whatever construct
comes next; under (b), `copy` is a *head* in the scene profile while `extends` stays a *slot word*
in the data profile — one word in two grammatical roles, the "one word, N positions" class the
syntax plan already catalogues against Aether. The closed operation list (which must be
**generated from the parser dispatch table**) then grows per construct × linkage under (b), and by
two entries total under (a).

**Rejected, with its price stated rather than dismissed.** Option (b) is genuinely cheaper to read
in the common case — no linkage word when the link is live — and maximally loud on scan. **(a)'s
price is one mandatory word on every instance line**, including the case an author would have been
happy to leave implicit. The trade is taken because a generator emits a required token in a fixed
slot from a two-item closed set essentially without error, while a *silent* default is a class of
error nothing catches. **On the generator axis specifically:** under (a) both wrong answers are
visible (the token is there and it is the other one); under (b) one of the two wrong answers is
invisible — emitting `instance` when it meant `copy` produces a well-formed node with the wrong
semantics and no diagnostic, and omission is the commonest generator error.

**`from` — measured, with the qualification that strengthens the deletion.** A raw grep counts
prose, so the measurement is over **code fences only**: extract fenced blocks from `docs/gaia/*.md`
and match `\bfrom\b` → **1 hit in 44 fenced lines across 4 files**, and it is
`bind value from=@player/unit …` — `from=` as a **field key on `bind`**, not the head-position
`from` this ruling deletes. So the head keyword's occurrence count is **zero** and the ballot's
ground holds. `grep -rn '"from"' crates/aether_lang/src/` exits 1 — not a keyword on the Aether
side either. The qualification: had head-`from` survived, `from` would have carried two
grammatical roles across the two profiles — Ground 4's class. ⚠ **A riding line falls out of it:**
the syntax plan's own R1 amendment spells that construct `bind: value source=…`, so the corpus
contains an undeclared `from=` → `source=` rename. GB-4 deletes head-`from`; the surviving role
must not be left half-renamed. That edit is **owed to G2** and is not made here.

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
- **THREE reference kinds, three sigils** *(the two-kind rule as ratified 2026-08-28, extended by
  ballot **GB-3** — third kind **ADOPTED BY THE OWNER 2026-08-30**, the four-part rewrite ruled the
  same day [delegated]; body in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29)*. The
  extension is consistent with the original rationale — *one spelling would invite authors to
  assume one behaviour* — and does not reverse it. **The third row's last column is what makes it a
  third kind, and it is the column the other two do not have:** an asset reference is the only one
  whose referent can **stop being valid after load**, because streaming retires slots — which under
  the owner's F5 ruling is the normal case, not an edge.
  *Duplicate reduced by the merge `merge/ke16-into-render`, 2026-09-10.* `feat/multi-paradigm-render`
  carried the same paragraph headed **THREE reference kinds** without the sigil count and without
  the [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29 body pointer; it asserted no fact
  the bullet above does not. Both sides' bullets are kept: the first is the 2026-08-28 two-kind
  ratification, which `feat/threadpool-ke16` had replaced rather than kept.

  | kind | sigil | at bake | at load | after load |
  |---|---|---|---|---|
  | **lexical / copy** — templates, `let`, styles, inheritance bases | `$` | substituted; **no reference survives into the binary** | nothing to resolve | nothing |
  | **entity** — an authored object's identity | `@` | resolved offline; `UnmappedEntity` lifted from load to bake | **remapped** through the load map | stable for the load's lifetime |
  | **asset** — mesh, curve, string table, font, sound | one glyph, closed jointly with the operator list (below) | resolved to a **path-name hash** | **looked up, not remapped** — `PathIndex::lookup(hash: u64) -> Option<(u32, u32)>` = `(slot, generation)`, `crates/boyko_ecs/src/ecs/core/asset/path_index.rs:112` | **refcounted and revalidated every frame** — `validate_asset_refs` compares `try_generation(slot)` against the `MeshRefGen` / `MaterialRefGen` lanes and disables a stale row (`crates/boyko_render/src/asset_refcount.rs`) |

  Both collapses are refuted by that table, at source. It **cannot** be the entity kind: there is no
  `Entity`, no load-map row, and `PathIndex` is first-insert-wins with one entry per hash —
  **globally interned, not per-load**. It **cannot** be the lexical kind: a lexical reference does
  not survive into the binary at all, and this one must. ⚠ **And the asset kind already has the
  cross-cell mechanism the entity kind lacks:** two cells referencing the same path resolve to the
  *same* `(slot, generation)`, the refcount keeps it alive while either cell holds it, and unloading
  one cell decrements without disturbing the other. That is a positive reason to keep the kinds
  apart — folding assets into `@` would take the one reference kind that already works across a
  cell boundary and give it the lifetime of the one that does not (§Load semantics, GK-1).

  **Part 1 — the asset-ref spelling is a SIGIL** (option (a)), applied uniformly and
  position-independently. The invariant bought: **a bare word is never a reference**, anywhere.
  That invariant replaces the case-gate the syntax plan's R6 lost when M12 refuted it, and it makes
  the dangle check a single lexical pass instead of a grammar walk.
  - **Rejected: (c) bare strings**, on this section's own ratified rationale. A string literal is
    already a **non-reference value** in this language (`text "…"`, contract text, `stable_name`),
    so (c) puts kind 3 into the same spelling as a plain value — a worse collision than the one the
    rationale was written against. Its second price is the dangle check: (c) keys on the
    destination field's GK-4 row, which works for component fields (the GB-2 precedent) but **not**
    for the positions that carry asset references and have no destination field — the instance base
    slot, a valve argument, and a file's own `asset` declaration. Three mechanisms where the sigil
    needs one, and three places the check can be forgotten for a position nobody enumerated.
  - **Rejected: (b) a typed head**, on the measured ground that already killed widget sugar: *"the
    vocabulary is unclosable — every new widget steals a legal object name"*, and **R7** requires
    the operation list be closed and generated, which an open-ended per-kind head vocabulary cannot
    be. It also buys type information the system already holds twice.
  - ⚠ **The glyph is CONSTRAINED and is not independently choosable — this is a finding, and it is
    why no glyph is minted here.** Measured over Gaia code fences (extract fenced blocks from
    `docs/gaia/*.md`; 44 fenced lines, 4 files): `/` 21, `@` 10, `:` 4, `#` 3, `|` 2, `-` 2, `$` 1,
    `;` 1, `+` 1, `>` 1; **absent: `!  %  &  *  <  ?  \  ^  ` ~`**. Constraints: not `@`, `#`, `$`
    (taken — entity, colour literal, lexical); not `&` (ratified-rejected at R4, and it reads as
    *borrow* to Rust eyes, misstating an owned refcounted handle); not `?` (reads as
    fallible/optional, which states the opposite of §Refusals' *no construct with a fallback*); not
    a backtick (measured-hostile in this toolchain, and Gaia text lives inside markdown throughout
    this corpus); and **not any glyph that can open a binary operator** — R3 ratifies that
    whitespace is never significant, so `(a %b)` and `(a % b)` must parse identically, and `%` is
    the glyph the ratified `for … if` guard wants for its commonest predicate. **Therefore the
    asset sigil cannot be chosen before Gaia's arithmetic operator vocabulary is closed** (§The
    logic line ratifies *arithmetic/interpolation over same-document values* and never enumerates
    the operators, while M17 already requires that list to exist). They are one question, and
    choosing the glyph first is how the ambiguity gets minted. **Recommendation: `~`** — absent
    from every Gaia fence and from `aether_lang`, never binary in any plausible operator set,
    unused in Rust, carrying no prior that misstates a behaviour. **The final glyph is closed by
    G2 jointly with the operator list, and that joining is part of this ruling.**
  - **The dangle check, named.** Every sigil-marked token must resolve at bake; unresolvable is a
    coded `GA####` refusal naming the pack and the path, blamed on the reference's own span. The
    resolver is the bake-time analogue of the shipped `PathIndex`: a name-hash index over the
    pack's assets, first-insert-wins, so there is exactly one entry per hash and never an ambiguous
    first match. This is the same lift §Identity already ratified for the entity kind
    (*`UnmappedEntity` lifted from load to bake*), now stated for the asset kind, which the
    ratified text never did. Because it is one lexical pass over sigil-marked tokens, the check
    **cannot be structurally unreachable** for a position someone forgot to enumerate — the
    specific failure (c) carries.

  **Part 2 — style references are the FIRST kind (lexical/copy), not the third and not a fourth.**
  §Refusals already ratifies *"no cascade (styles are named records by explicit reference,
  **flattened by bake** — 'which rule won' is a fact, not a computation)"*, and a thing flattened by
  bake leaves no reference in the binary, which is the definition of kind 1. ⚠ **A live
  contradiction inside the ballot's own text is resolved here rather than carried forward:** the
  ballot listed *"a style record"* among the asset kind's examples, contradicting §Refusals on the
  same page. §Refusals is ratified; the parenthetical was not, and it is **dropped** — the example
  list above carries no style record. Streaming check: a copy carries **no cross-cell obligation at
  all**, which makes it the safest kind under F5. Its price is ratified rather than chosen here —
  flattening multiplies bytes across N cells and a style edit rebakes every cell that used it, and
  the provenance sidecar is what keeps that debuggable.

  **Part 3 — `$hole` STAYS, reclassified.** The ballot's premise (*"a third reference sigil where
  two are ratified"*) is false: templates and `let` are already named as the **lexical** kind, so
  `$power` is kind 1 and was ratified all along. What `$` marks is not a kind but a **substitution
  from a lexically-bound name at value position**, and it is doing work nothing else can do: R6's
  case-gate was refuted (M12), so case is a lint only, and at value position the grammar admits
  bare lowercase words that are **not** references (RON-style enum values, the ladder names
  `base|variant|tuning|debug`, `rarity=common`). `power=power` is genuinely ambiguous, and a
  template with a parameter named `common` would silently flip `rarity=common` from an enum value
  into a substitution. **Ruled disposition:** `$` marks the lexical kind — template parameters
  **and** `let` bindings — **in every position**, including the reference slots R4-as-amended
  currently sends bare (`extends $sword_base`, `apply $Torch`). That regularity is what lets a
  generator pick the sigil from the **kind alone**, with no positional knowledge. This overrides
  R4's *"lexical/copy positions go bare"*, which is legitimate: R4 is an unratified syntax ruling
  in an owner-gated file, GB-3 regenerates it by construction, and ratified §Identity fixes `@` for
  the entity kind while saying nothing about the lexical spelling. **Price:** `extends $sword_base`
  is one character noisier than the bare form, bought against a whole class of value/reference
  ambiguity. M5's ban is intact — `$` is legal at value position only and forbidden in the name
  slot (`pillar_$i` stays refused) — and `$x` where `x` is not a declared parameter or `let`
  binding is a coded refusal naming the declared list.

  **Part 4 — "a declared node ⇒ `@`" is WITHDRAWN**, and replaced by a positive rule. The ground is
  the one the syntax plan already states — the criterion is *behaviour*, not declaredness — and the
  corpus shows the damage: `LANGUAGE.md` spells a **lexical** inheritance base as
  `extends @iron_sword`, using the identity sigil for a copy. **What replaces it is one question
  with three mechanically decidable answers — *what does bake do with this reference?***
  1. **substitutes it** → lexical, `$` (templates, `let`, styles, inheritance bases);
  2. **records an object id for load remap** → entity, `@`;
  3. **records a path hash for load lookup + refcount** → asset, the new glyph.

  Decidable from the GK-4 field table at field positions and from the head/valve grammar elsewhere.
  ⚠ **This rule is F8-neutral and F10-neutral by construction**: it never asks whether a node's
  anchor is a human name that bakes to an id or a minted id, and it says nothing about whether a
  bundle expands at bake.

  ⚠ **Riding lines this ruling OWES and does not make here** (recorded so they are not lost):
  [`PENDING-SYNTAX-PLAN.md`](PENDING-SYNTAX-PLAN.md) R4/R5/R7 and Tiers 1–2 are regenerated by this
  ruling, and [`LANGUAGE.md`](LANGUAGE.md) carries `extends @iron_sword` and bare-string asset refs.
  Both files are already marked — PENDING as owner-gated and uncommitted, LANGUAGE.md as
  `ratified-stale` in its own head, gated by
  [`tests/gaia_g0_citation_census.rs`](../../tests/gaia_g0_citation_census.rs) — so the divergence
  is *marked*, not silent. The edits belong to **G2**.

  ⚠ **One FOLLOW-ON FINDING kept by the merge `merge/ke16-into-render`, 2026-09-10, from
  `feat/multi-paradigm-render`, which recorded this ruling on 2026-09-03 — it is not a ballot and
  is not closed:**
  the asset-ref dangle check is structurally unreachable downstream of the sigil pass as the engine
  stands — `AssetServer::load` logs `E0801` and returns a LIVE handle in the `Failed` state which it
  inserts into the path index, and `validate_asset_refs` early-returns unless `free_epoch` advanced,
  which a never-loaded asset never makes happen.
  *That branch's restatement of this ruling is otherwise reduced to this note. Its own closing line
  sent the reader to `feat/threadpool-ke16` `gaia/DECISIONS.md` §Identity and references for the
  full ground — which is the text above, and is now in this file.*

- **GN1's coverage, stated per kind — and the ballot's premise about it is REFUTED.** The ballot
  asserted *"GN1 resolves every reference form in the binary to a name hash at load — WITHOUT
  remap. That is the asset kind's behaviour."* GN1's own four resolutions (§UI bindings item 1)
  open with *"object id → `Entity` (**load remap**)"*, so GN1 spans a remapping resolution and is
  not the asset kind's behaviour. **GN1 is the uniform cost-and-representation law over all three
  kinds** — nothing in the binary is an ungated ordinal, and every resolution is paid once, cold,
  at load. Per kind, which the ratified text never said: kind 1 has **nothing in the binary to
  resolve** (GN1 is vacuous for it); kind 2 resolves by **remap**; kind 3 resolves by **lookup**,
  then refcounts and revalidates. **Only kind 3 is lookup-without-remap.**
- `link` is mandatory grammar for entity-reference fields — the difference between a loud
  `UnmappedEntity` and a silently stale id.
- Content hashes are integrity/cache only, never identity; the freeze/cache key is a hash of the
  **normalized baked image**, never source bytes (the Dhall semantic-hash rule) — closes our
  measured raw-byte-hash-is-a-checkout-hash class by construction. EOL normalization happens in
  the lexer, before the CST.
- One id space across scenes and UI documents (UI already holds cross-document entity references).
  Bake resolves every cross-file reference offline — `UnmappedEntity` lifted from load to bake.

## Load semantics — what "loaded" means (ballot F4, ruled 2026-08-30 [delegated])

*(Body in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-28 and its index row in
§2026-08-29. This is the **GK-3** design line; it also carries the owner's **F5** streaming ruling,
because F5 rewrites what the fixup must survive.)*

**RULING: option (a) — a specified post-load fixup pass, in the form SUPPRESS-THEN-FIXUP, on the
clone path's own precedent. Option (b) is not rejected wholesale: its MECHANISM (fire the real
hooks) is adopted as sub-pass 4; its TIMING (during the load) is refuted by measurement.** Fired at
the right moment, (b) **is** (a).

**The five mechanisms, re-measured in this checkout.** All five reproduce as red at `6f75ee9e`
(`cargo test -p boyko-serialize --test gaia_f4_load_path_fixups -- --ignored --test-threads=1` →
`0 passed; 5 failed`, with the two controls green). The census in one number, with a command that
cannot match a comment — `grep -rEn "trigger_on_[a-z]+\("` — is **0** hook-dispatch call sites
across `crates/boyko_ecs/src/ecs/core/serialize/` + `crates/boyko_serialize/src/`, against **38**
across `crates/boyko_ecs/src/ecs/core/commands/` + `.../ecs_master/`. Two corrections the
measurement forces on the ballot's own list:

- **(ii) the `#[require]` closure is NOT a load-path defect** — the loader merely *shares* it with
  the dynamic by-id entrance. Measured on `EcsMaster::create_entity(arch, &[(ComponentId, bytes)])`,
  the only entrance a baker holding `(id, bytes)` pairs from text can use: `create_entity` fires 1
  hook and produces **no** required component, where `Commands::spawn` of the same bundle produces
  it. `create_entity` is public, shipped, and used by `boyko_physics/tests/bundles_s6.rs`, whose own
  comment already concedes the caller must hand-supply the require closure. So the silent wrong
  answer exists **in the live engine one call from gameplay code** — and a GK-4 baker built on
  `create_entity` would bake the defect *into the file*, which the "build a live world, then
  `save_world`" route was assumed to prevent.
- **(iii) flag state SPLITS IN TWO, and the ballot conflates them.** (iii-a) *declared* initial
  state (`FLAGS_DIRECT` / `apply_flags_declared_by`,
  `crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs:275`) is a pure function of the
  component set and is reconstructible **with no format change at all**. (iii-b) *authored or
  mutated* per-entity state is genuinely unrepresentable, and it is **F9**'s, not F4's — and it is
  a shared defect of two of the three non-spawn entrances, since the clone path drops the enable
  bit too.

(iv) and (v) are consequences of (i), not independent — with one addition the census file does not
carry: `mesh_handle_on_insert` (`crates/boyko_scene/src/render_caps.rs`) is
`if let Some(deltas) = dm.resource_mut::<RefcountDeltas>()`, so it **silently no-ops when the
resource is absent**. "Fire the hooks" only contributes a refcount in a world that already booted
the render plugin.

**The four ordered sub-passes. Order is load-bearing, not stylistic.**

| # | pass | mechanism it uses | closes |
|---|---|---|---|
| 1 | **require closure**, at PARSE time — widen the id list with `for_each_required_id_excluding` **before** the `load_archetype` call and emit the existing `LoadColumn::Construct` per added id | already in the loader (`crates/boyko_ecs/src/ecs/core/serialize/load_writer.rs:147,512`) | (ii) |
| 2 | **declared flags** — `flags_direct_for(owner)` per loaded id, then `set_enable_bit` | `enable_tag_api.rs:275` verbatim | (iii-a) |
| 3 | *(existing)* `remap_loaded_entities` | unchanged | — |
| 4 | **hooks** — `on_add` / `on_insert` per (entity, component), then drain deferred commands to a fixpoint | the same dispatch the insert path performs | (i), (iv), (v) |

Sub-pass 1 is the cheap surprise: **`LoadColumn::Construct` already exists and already takes a
`RequiredCtor`.** It is simply never reached for a component the file does not mention, because
column classification runs per *file column descriptor*. Widening the id list closes (ii) with
**zero new kernel machinery and zero migration** — no archetype churn, no second write. Requires
**must** precede hooks (sub-pass 4 reads lanes sub-pass 1 supplies). Hooks **must** follow the
remap. During sub-passes 1–3, relationship linking is suppressed through the existing
`relationship_link_suppressed` bracket
(`crates/boyko_ecs/src/ecs/core/relationship/mod.rs:97`).

**Why the timing is a measurement and not a preference.** `DeferredEcsMaster` statically withholds
every structural-change method, so a hook *cannot* "mutate the world mid-load" — the kernel already
made that impossible, and the ballot's stated price for (b) is therefore wrong. The real hazard is
**ordering against the entity remap**, which runs as a separate whole-world pass *after* every
archetype. `relationship_on_insert`
(`crates/boyko_ecs/src/ecs/core/relationship/generic_hooks.rs`) reads a **raw saved FK** and guards
with `if !view.is_alive(target)`; measured id collision across a normal round trip is
`saved=[0..7] fresh=[0..7]` — **overlap 8/8, 100%**. The guard does not fire, so the hook takes the
*link* branch and builds a reverse index pointing at **a live but wrong entity** — strictly worse
than today's empty index, because an empty index is detectably empty and a wrong one answers
plausibly. **This bug already has a name in this kernel: BUG-EDGE-CLONE-1** (`generic_hooks.rs:104`
— *"during a deep clone the FK is a VERBATIM copy still pointing at the ORIGINAL (un-remapped)
target"*), and the clone path's fix is exactly the thread-local suppression bracket plus a relink
that runs **after** the FK is remapped. The clone path also already reconstructs missing required
components. **The kernel has already answered F4 once, on the sibling path, and it answered (a).**

**What this costs to build: three `pub` promotions and a driver, not a redesign.** Measured as
compile errors from a downstream crate: `for_each_required_id_excluding` is private
(`component_registry/required.rs:404`), `DeferredEcsMaster::from_world` is private, and
`EnableTagId(pub(crate) ComponentId)`
(`component_registry/tags.rs:93`) is **one-way** — forward conversion is public, the reverse has no
path, so a pass holding a resolved flag id cannot call `enable_id`. Everything else the fixup needs
is **already public**: `required_ctor_in_set`, `flags_direct_for`
(`component_registry/flags.rs:156`), `get_hooks`, the `ComponentHooks` fields, `HookFn`, the
`FlagDirectEntry` fields.

**Rejected, with prices.**

- **(b) as written — fire hooks DURING the load.** Price: a 100%-rate silent mis-link of every
  relation (measured above); refcount hooks that no-op against a world without `RefcountDeltas`;
  `on_replace`-shaped hooks reading a lane sub-pass 1 has not yet supplied; and it still leaves
  (ii) and (iii) open, so the coverage census is owed regardless. Rejected on measurement, not
  preference.
- **(c) forbid load-incomplete components in baked assets.** **Verified rather than inherited, and
  it is worse than "untenable".** An attribute-line census over `crates/*/src/`, hand-audited to
  drop test-fixture and doc-string hits: **10 require-bearing shipped components** — three light
  types, `ParticleEffectHandle`, three camera types, `MeshHandle`, `MaterialHandle`, and
  `UiWorldProjection`'s owner — plus **3 explicit hook-bearing** and the whole
  `Relationship`/`RelationshipTarget` family. (c) forbids **every mesh, material, light, camera and
  particle effect**; a scene profile that cannot express a mesh is not a scene profile.
  `MeshHandle` alone is load-incomplete on **three** counts at once (requires ×3, `on_insert`,
  `on_replace`), and it is the one component no scene can omit.
- **Fix it at bake instead — make the baker spawn through `Commands`.** Price: refuted for the
  *dynamic* set a text document produces (the `create_entity` measurement above), and it cannot
  work in principle for (i)/(iv)/(v) — the bake-time world has no `RefcountDeltas` and no renderer,
  so a bake-time refcount is meaningless, and reverse-index state would have to survive the format.
  **Bake-time is the wrong side of the boundary for every runtime-state mechanism.** Sub-pass 1 is
  the only part that could move to bake; keeping it in the loader costs one `Construct` column and
  buys immunity to a stale bake.

**The coverage census is part of the ruling, not a follow-up.** It enumerates **all five**
mechanisms, not hooks alone — the ballot's own recommendation, and the reason the widened form of
F4 was put at all.

### Streaming scope — F5, ruled by the owner 2026-08-30: EVERYTHING FROM THE START

Owner: *"Все сразу грамотно по списку с самого начала."* ⇒ **option (b)** — the one nobody had
written down. `load_cell` / `unload_cell`, **GK-1's cross-load map with a declared lifetime**, and
cross-cell reference resolution ship **with** the scene profile (**G6**), not after it at G8.
**Rejected: (a) format-ready-loader-later** (catalog + attribution + id map now, loader at G8).
Price of the rejected option as the ballot itself priced it: a catalog with no loader is a datum
nothing consumes until G8 — this repository's own recurring defect class. **Price of the ruling,
stated because it is the larger one:** G6 absorbs the hardest half of the design, since a per-load
`LoadEntityMap` cannot express references BETWEEN chunks (measured), and the scene profile cannot
land until that is solved. That price is now G6's, and G6's row says so.

**GK-1 — six requirements, each from a measured blocker.** `LoadEntityMap` today
(`crates/boyko_ecs/src/ecs/core/serialize/mod.rs`) is `entries: Vec<(u64, Entity)>` keyed on the
**saved `EntityId.0`** — a *file-local* value — with the contract insert-all → `finalize` (sort
once) → `get` (binary search) and `debug_assert!(!self.sealed, "insert after finalize")`.
`LoadEntityPolicy` (`crates/boyko_serialize/src/load.rs:72-77`) has exactly **one** variant,
`Remap`.

1. **The key becomes a GLOBAL object id, not a file-local `EntityId.0`.** Two independently baked
   cells both contain saved ids 0,1,2… — they collide outright. The key is the industry pair the
   campaign already names: durable asset/file id + file-local object id.
2. **Insert-after-`finalize` must become legal.** Cell *N+1* inserts into a map cell *N* already
   sealed; today that is a debug panic and, in release, an **unsorted tail that `binary_search`
   silently misses** — a wrong answer, not an error. Keep the sorted `Vec` (never keyed on an
   untrusted value; memory `O(entries)`) and make sealing incremental: sort the appended tail and
   merge.
3. **`unload_cell` needs removal — which the map has no method for at all — and a REVERSE edge**
   ("who still references me"), which it does not carry. Without it, unloading cell A leaves cell
   B's `@` references dangling with no diagnostic.
4. **Lifetime: world-owned (a `Resource`), not a per-call local.** Today it is a stack local of
   `load_world`, destroyed at return. F5's declared lifetime is "as long as any loaded cell can be
   referenced".
5. ⚠ **The fresh-world contract must be lifted, and this is the sharpest one.** `load_dense_store`
   carries `debug_assert!(store.is_empty(), …)` whose message reads *"fresh-world-load contract
   violated — merge load is unsupported"* (`load_writer.rs:703`), with the reason in its own doc: a
   remapped fresh id could collide with an existing member and **silently corrupt the store**.
   `load_cell` **is** a merge load by definition, and the guard is a `debug_assert!` — **it
   vanishes in the shipping build**, so today `load_cell` would corrupt the dense store silently in
   release and loudly only in dev.
6. **`LoadEntityPolicy` gains its second variant** (`MergeInto`), which is where the streaming
   contract is written down rather than implied.

**The F4 ruling survives all six**, because every sub-pass is **per-entity and idempotent**:
sub-passes 1–2 are pure functions of one entity's component set, and sub-pass 4 is the same
dispatch the insert path performs per entity. Running the fixup over *only the newly loaded
entities of one cell* is the same code with a narrower iteration domain. ⚠ **But GK-1 and GK-3 must
land together or the cross-cell half of F5 is silently half-present:** sub-pass 4's hook dispatch is
what makes a cross-cell reference *observable* — a `ChildOf` across a cell boundary only enters the
reverse index when the link hook runs, which is today's F4(iv) one scope up.

### The stable asset-id carrier — F4's G6 addendum, ruled with it

**The carrier is a stable asset NAME in the file, resolved to the existing `MeshHandle(u32)` at
load — no new component type.** Three measurements decide it:

- **`MeshHandle(pub u32)` (`crates/boyko_scene/src/render_caps.rs:143`) blits a process-local slot,
  and it is worse than the ballot states**: `Handle<T>` is `{ index: u32, generation: u32 }` and
  `MeshHandle` drops even the generation. That omission is *why* `MeshRefGen` exists as a separate
  lane.
- **`MeshRef` does not exist as a type.** `grep -rn "\bMeshRef\b" crates/` → **0**; the 7 hits in
  the repository are all corpus prose. `grep -rnE "MeshRefGen|MaterialRefGen" crates/` → **96**, a
  **generation counter**. The name is not merely unlanded; it is one suffix from a live type
  meaning something else. **Do not mint `MeshRef`.**
- **There is no name- or path-keyed asset lookup anywhere in the engine.**
  `grep -rnE "get_by_name|by_path|load_path|handle_for_name|name_to_handle"` over
  `crates/boyko_ecs/src/ecs/core/asset/` + `crates/boyko_render/src/` → **0**. The registry the
  carrier would key into does not exist either; **GB-3's rewrite must absorb that**, and it is a
  build item, not a spelling item.

This is not an invention — it is the discipline the engine already applies to *component* identity,
and the format already implements it: `EnableTagId`'s own doc says the numeric value is
first-call-order process-unstable and **the name is the stable serialization key**, and
`load_world`'s doc says the loader *"resolves the file's stable names against already-registered
ids"*. Asset identity gets the same two-layer treatment: **stable name on disk, process-local slot
in memory, resolution at load.** The runtime carrier stays `u32` — `#[repr(transparent)]`, and the
GPU draw-indirect path reads the column raw, a property that must not be spent. Concretely this
makes the asset reference **GB-3's third kind** in the format, resolved at the sub-pass 1 / sub-pass
4 boundary: the fixup resolves name → `Assets<T>` slot and writes `MeshHandle`, and sub-pass 4's
`on_insert` then contributes the `+1` that closes (v). One mechanism, both halves. ⚠ This names a
**carrier form**, not a name-vs-id ruling: **F8** stays free to decide whether an authored object's
name is its id.

### What F4's ruling does NOT settle, stated so nothing is settled by implication

- **F9** owns (iii-b). The ruling closes (iii-a) with no format change and hands F9 a strictly
  smaller question, but F9 **cannot** be answered "the format already carries it".
- **F8** and **F10** are untouched.
- ⚠ **The `create_entity` require-drop is an ENGINE defect independent of Gaia and needs its own
  carrier.** It is not F4's to fix, and filing it under F4 would let a Gaia-scoped fix leave the
  gameplay-facing hole open.
- ⚠ **`remap_loaded_entities` is O(whole world), per load** (`load_writer.rs:919` collects **every**
  archetype, then walks **every** live row of each remappable column — not only the freshly loaded
  ones). Under F5 this runs on every `load_cell`. Pre-existing, not created here; it belongs to the
  F4/F5 build and is a further reason a data table is loaded once and pinned rather than per-cell
  (§Data tables).
- ⚠ **An `F#` id-namespace collision, recorded because the two series are semantically adjacent.**
  [`../ASSET-STREAMING-PLAN.md`](../ASSET-STREAMING-PLAN.md) declares its own `F1`–`F8`, and engine
  doc comments cite it *inside the very files this ruling stands on*
  (`crates/boyko_scene/src/render_caps.rs` cites "asset-streaming plan F2" and "F5"). Asset-streaming
  **F4** is `PathIndex` while Gaia **F4** is this ruling, whose fifth mechanism is asset refcounts;
  asset-streaming **F5** is `MeshRefGen` / `validate_asset_refs` while Gaia **F5** is the streaming
  scope just ruled. This is the class the syntax plan already fixed once for `R#` — *a bare `R#` is
  legal only in the table that DECLARES it* — and the same discipline is owed to `F#` at every cite.
- ⚠ **A latent item in a sibling plan becomes live with Gaia, and its own next sentence is a warning
  against option (b) executed in the wrong order.** [`../ASSET-STREAMING-PLAN.md`](../ASSET-STREAMING-PLAN.md)
  item (e) already filed *"`load_archetype` … does NOT run `#[require]` expansion → a `MeshHandle`
  row … would lack `MeshRefGen` and be AND-filtered out of `validate`'s query (silent, no panic).
  **Latent (no such save in-tree)**"*. Gaia is the thing that ends "no such save in-tree".

## Data tables — where a DataAsset's rows live (ballots F2 and F3)

### F2, ruled 2026-08-30 [delegated]: (a) rows are ENTITIES — amended in three ways the ballot did not carry

**(i) Storage is `StorageKind::Table`, not dense — and that is the answer to "data that doesn't
have to be dense".** The ballot's own words were *"a dense-column archetype"*, and that phrase
**does not denote anything in this engine**: a dense id is signature-excluded, so
`get_or_create_archetype(&[DenseRow])` returns **the empty archetype** (measured:
`dense_arch == empty_arch → true`), and under the ballot's wording 4000 item rows would land in the
world's component-less bucket alongside every bare entity in the game. For a table whose rows all
carry the row component and nothing else, the `Table` archetype holds **exactly one
`ComponentPool`, and all rows are already contiguous in it** — dense's one-global-column property
buys nothing while costing an `s2e` column, an `e2s` map sized to *the maximum entity id ever
inserted*, a live bitmap, a free list and a per-archetype bitset. **The mechanical rule for which
kind a given table gets: is the row component carried by entities OUTSIDE the table?** No (a pure
item catalogue) → `Table`. Yes (an in-world instance carries the row inline, so the component
spreads across gameplay archetypes) → `Dense`. It is a property of the schema, decidable at bake,
and one derive attribute either way.

**(ii) Rows become entities EAGERLY, AT LOAD — never lazily.** Four grounds, descending:
**determinism of the world** (`save_world` sums live archetypes, so under lazy materialization a
save records which rows *happened to have been touched* — the same game state saves to different
files and a round trip is no longer one); **every insert-path mechanism F4 enumerates fires on
insert**, so lazy materialization fires them at an arbitrary later frame from inside whichever
system first touched the row; **the derive-emitted row constants exist to delete a lookup**, and a
constant that must first check residency *is* the lookup; and **Principle 1** — a residency branch
on every row access with no measurement behind it.

**(iii) A table is loaded by its own explicit load and PINNED; `unload_cell` never touches it.** Its
entities live in their own archetype, which belongs to no cell's entity set, so the cell-unload path
never enumerates them. **The runtime handle is an `Entity` captured at load, never a row index** —
`Archetype::swap_remove` moves rows and loads append into dedup'd archetypes, so a row index is not
a stable handle in this engine. ⚠ That is stated **without touching F8**: whatever the *authored*
identity turns out to be, the *runtime* handle is an `Entity`, and it is captured, not computed.

**Why the ruling holds under F5's streaming, answered by measurement rather than assertion.**
`load_world` is **additive** — it creates fresh entities into the existing world and never clears
it — so a table loads *by itself*, into a live world, at any time; there is no full-world-load
prerequisite. And `load_archetype` **dedups and appends** (`start_row` is the archetype's current
index; capacity reservation is additive), so repeated cell loads append into existing archetypes
rather than minting new ones. Both are properties the streaming half needs anyway; (a) consumes
them rather than adding to them.

**What the ballot's headline price actually is, measured.** For 4000 rows of a 40-byte row type, as
table entities, marginal against a running world: `ItemRow` pool data 262 144 B + `added` ticks
65 536 + `changed` ticks 65 536 + `Archetype.entity_ids` 65 536 + inland store 64 000 =
**522 752 B ≈ 511 KiB** (payload 160 000 B). The 4000 ids consume **0.006%** of the inland store's
67 108 864-slot ceiling. Per-frame cost of those idle entities is **one extra archetype mask test
per query**, not 4000 row visits — queries are signature-filtered.

**Rejected: (b) a new RESOURCE region in the byte format.** It buys **129 536 B** on that table
(25%), or 326 144 B (62%) **only if a new tick-free VM primitive is also written** — because
`VmColumn<T>::new` **panics** unless `size_of::<T>()` divides the commit granule
(`crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149`), and 40 does not, nor do 48, 56 or 72. So a
resource-owned flat column cannot use the engine's bare column primitive at all; it must reuse
`ComponentPool` — which requires a registered `ComponentId`, i.e. the row type is a `Component`
anyway, and which unconditionally lays out `[data | added | changed]`. Against **0.13 MB**, the
costs are: a format version bump and a third header growth; **≥ 607 production lines + ≥ 444 test
lines** by the dense region's own precedent (`d043ad8f`, which added the v1→v2 dense region), for a
region whose payload shape — unlike dense's — has **no existing analogue**; and **the entire
per-`ResourceId` serialize seam from scratch**, because `grep -rniw "resource" crates/boyko_serialize/src/`
returns **0 occurrences of the word anywhere in the crate, comments included**, and the resource
registry has no stable name, no fingerprint and no fn-pointer table (components get all of that from
an 847-line `SerializeInfo` carrier). It also forfeits queryability: a table that is a resource
cannot be joined against anything, so *"which recipe uses this item"* becomes hand-written iteration
over a side structure — the shape Principle 0 exists to refuse.

**Rejected without a ballot, and still rejected: a `HashMap<Name, Row>` side store** — a parallel
data system, forbidden outright and mechanically caught by `clippy.toml`'s `disallowed-types` on the
existing `-D warnings` gate. **Rejected: `StorageKind::Dense` as the default for table rows** —
measured to collapse into the empty archetype, and to add `s2e` + `e2s` + a live bitmap + a free
list for a contiguity the single-archetype `Table` case already has.

⚠ **Two corpus corrections this ruling makes in passing.** `load_archetype` / `load_dense_store` are
**not** in `boyko_serialize` — they are `pub fn` in
`boyko_ecs::ecs::core::serialize::load_writer`, while `boyko_serialize`'s own parser halves are
`load_one_archetype` and `load_dense_region`; the ballot's "a second load path in `boyko_serialize`
beside `load_archetype`/`load_dense_store`" straddles two crates. And two engine docs contradict
each other on the W4 anchor — `crates/boyko_serialize/src/load.rs` says loads go into *freshly
created* archetypes while `load_writer.rs` says `create_archetype` **dedups** and *"the W4 anchor is
therefore relaxed"*. Neither is Gaia's to fix; both are recorded so the next reader does not inherit
them.

### F3, ruled by the owner 2026-08-30: ONE schema for both file shapes

Single-file-per-asset **and** a table file baking N rows are both admitted, and they bake through
**one schema**: the table file is a **spelling**, not a second type system. Rejected: a table
dialect — which the one-language-three-profiles ruling already refuses in the large. The owner asked
for a recommendation on the details; that is answered separately and is not this line.

## Flags in the byte format — ballot F9 (PARTLY ruled 2026-08-30; one VALUES question remains the owner's)

*(Body in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md) §2026-08-29. The research settles three
things beyond doubt and declines to settle the fourth, which is a values call about the authoring
surface — recorded here rather than decided.)*

**The two-sided measurement.** A probe with two `storage = "bitset"` flags on a POB-carrying entity
— one declared through `FLAGS_DIRECT`, one raised by hand — round-trips through `save_world` /
`load_world`: the entity survives, the POB field restores, and **both** flag bits come back
`false`; the file names no bitset column at all (`types_bitset_skipped = 0`, so the load-side guard
was never even exercised), while a **respawn in the same loaded world** sets the declared flag
`true`. Both sides reproduce.

⚠ **The F4 census's stated SAVE-side mechanism is wrong, and the correction changes what a carrier
must defeat.** The census says a bitset id is excluded from every archetype signature "so the saver
never emits a column for it". But `save_world` iterates **`archetype.component_ids()`**
(`crates/boyko_serialize/src/save.rs:174`), which is the **raw unfiltered** list — measured: an
archetype minted over `[POB, bitset]` reports both ids. The guard that actually stops the saver is
one match later — `component_pools().get_pool(component_id) { Some(p) => p, None => continue }`
(`save.rs:190-193`) — and it runs *before* the type is interned, so a flag's type never reaches the
file's type table. **Conclusion unchanged, mechanism different: the save side is guarded by
POOLLESSNESS, not by the signature filter** — which means a carrier **never has to touch
`filtered_signature_mask`**, and the archetype-fragmentation invariant stays exactly as it is.

**RULED (1): `FLAGS_DIRECT` must NOT be fired on the load path.** `apply_attach_flags_all` /
`apply_attach_flags_for` have **8 call sites in 4 files**, all spawn or migration — command:
`grep -rn "^[[:space:]]*\(self\|world\)\.apply_attach_flags_\(all\|for\)(" crates/boyko_ecs/src/ | wc -l`
→ **8**. `load_archetype` calls neither, and KE10's own header says *"NOT covered, and deliberately
so: clone and load."* Firing it would be a **new** defect, not a fix: `apply_flags_declared_by`
writes `set_enable_bit(entity, flag, entry.initial)` **unconditionally**, and the kernel's own doc
states *"`false` is NOT a no-op: it clears a bit an earlier attach may have set"*. `load_archetype`
serves **two semantics with one function** — *born* (a Gaia bake: the file is the initial state) and
*resumed* (a savegame: the file is the current state). FLAGS_DIRECT is right for the first and wrong
for the second; on the very round trip the red fixture asserts, it would overwrite every saved bit
with its attach-time value — **the door the player unlocked comes back locked.**

**RULED (2): the refusal option is inconsistent with a ratified owner ruling and may not be
chosen.** **AB-6** placed the flag refusal *in the derive, not in Aether*, under the owner's
principle that a language *"may not refuse what the derive accepts"*. F9's option (a) is that exact
shape one layer out: `EcsMaster::enable` is public, sanctioned and O(1), and the clone path's own
doc calls enable-bit propagation a *"v1.1 follow-up"*, not a prohibition. The three refusals are
also not the same shape — **AB-11** and **AB-6** each leave the capability intact and point at a
working spelling, while F9's option (a) has no honest did-you-mean: "make it a component" costs the
author the archetype fragmentation the flag exists to avoid. **Consistency with AB-11/AB-6 is broken
by the refusal, not preserved by it.**

**RULED (3): if a carrier lands, its spelling is `flags (X = true)` — AB-13's ratified vocabulary,
shared VERBATIM with Aether, as a per-object group.** No second spelling is minted (the same
argument the corpus already applies on the `link` axis). And the carrier is **F4's GK-3 carrier, not
a new one**: the flags region's load side is one entry in the fixup pass, and it must be keyed on
**GK-1's global object identity**, never on a file-local row index — a flag raised in cell A on an
entity owned by cell B is a cross-cell write, and row-index keying forecloses it.

**What a carrier would cost, measured so the open question is priced.** Format: the v1→v2 dense
region is the exact precedent in the same file — a 16-byte header descriptor plus a trailing region
with an explicit 0%-gate; a flags region is that shape again (`flags_table_off` /
`flags_block_count`, header 80 → 96 B, format version 2 → 3). Bump cost **today** is near zero and
measured — `find . -name "*.sav" -not -path "./target/*"` returns nothing, and the only version
assertions read the `FORMAT_VERSION` *symbol* — and it will not stay near zero once a savegame
ships. Region size: one block per (archetype block × tag with a column), `u32 type_index +
ceil(n/8)`; 10 000 entities × 4 tags = **5 016 B**. Save side: **one `pub` accessor, no new
algorithm** — `EnableStore::read_row_bits`
(`crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs:702`) is the per-row snapshot and
`enabled_runs` (`:385`) the bulk walk, both `pub(crate)`, and the migration path already does
snapshot-here/restore-there across archetypes. Load side: **no new writer at all** — the loader
already holds the fresh `Entity` per row and `enable_id` is public and O(1); the one gap is the
missing reverse `EnableTagId::from_component_id(ComponentId) -> Option<Self>`, which screens
`StorageKind::Bitset` and is therefore the *right* shape anyway. The load-side W1 bitset skip stays
and must stay — it guards the *column* path, whose failure mode is a poolless-id panic — and the new
region needs its own hardening in the same style. ⚠ `load_fuzz.rs` sums the skip counters into its
bound; **a new counter must join that sum or the fuzz bound silently stops being a bound.**

**Rejected, with prices.** *(a) a `GA####` refusal*: contradicts AB-6, has no honest did-you-mean,
and leaves F4(iii) permanently red — savegames lose flag state forever. Cheapest to build, most
expensive to live with. *(b) FLAGS_DIRECT on load, no carrier*: measured wrong semantics for the
savegame half of the same function, and it cannot express per-entity variation, which is the case
that raised F9. *(c) carrier now, spelling later*: a format region nothing writes — a dead datum,
the class this corpus has catalogued five times.

⚠ **STILL THE OWNER'S, and it is one sentence:** *is Gaia's flag surface allowed to set a flag on an
individual authored object, or may an entity's flag state only be declared by a component it
carries?* If per-object authoring is allowed, the carrier is required and both (a) and (b) fail. If
it is not, FLAGS_DIRECT covers the declared half and the carrier is needed only for F4(iii)'s
savegame fidelity — a smaller, separately schedulable claim. **This is a VALUES call about the
authoring surface**, which is why it is escalated rather than decided. It blocks **G2** (the
grammar) and nothing else; the three rulings above are not contingent on it.
⚠ One inconsistency spotted in passing, not F9's to fix: the ballot list records AB-11's
did-you-mean as pointing at `enabled`/`disabled`, while AB-6's ruling text records the did-you-mean
as pointing at `flags (X = true)`. Different refusals, different targets — worth one reconciling
line.

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

     ⚠ *Kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/multi-paradigm-render`'s
     shorter statement of the same ruling — the one clause that side carried and this one does not:*
     the work order in the same commit that carried the deadline already declared R0 *"buildable
     now, no ballot in the way"*.
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

   *Duplicate reduced by the merge `merge/ke16-into-render`, 2026-09-10.* `feat/multi-paradigm-render`
   carried a shorter restatement of this same GB-7 ground, written by its 2026-09-03 sync. Every
   number in it — 332 ns, +0% / +82% / +101%, the 100 ns timer step, the 0.05-0.50
   signal-to-noise, and the clock's legitimate return over the scan itself — is above, so it is
   reduced to this line rather than repeated.

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
✅ **Superseded on the ballot half, 2026-08-30: F1 and F4 were both answered that day** (F1 by the
owner, F4 delegated and ruled — §Inherited pipeline and §Load semantics). **G0 now carries no open
ballot; it still carries this ruling's work**, and the widening edit has not landed. The sentence
above is kept rather than rewritten because "no open ballot" is not "done", and this row is the
example.

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

✅ **F7 RATIFIED BY THE OWNER, 2026-08-30: mods are NOT supported for now — option (a), stated
explicitly.** The ballot existed because **silence was itself a decision**: the wide reading of the
reflection refusal would have calcified by never being contradicted, and a later mod campaign would
have been blocked by a sentence that never meant to block it. It is no longer silent. **The
constraint is recorded in its narrow form, which is what the explicit ratification buys:** what is
ratified is *no reflection in the GAME BINARY* (§Inherited pipeline), not *no bake tooling in a
player's hands at all* — a text-mod pipeline would ship the bake tool as a **separate executable**
and the game would still load only bytes. Rejected: **(b) accept the separate-executable pipeline
now and design the format's stability guarantees for it from the start.** Price of the rejected
option: v1's format gains a compatibility surface for a consumer that does not exist, which is the
dead-datum class. Price of the ruling: if a mod campaign is ever taken, the stability guarantees are
retrofitted rather than designed in — accepted knowingly, and the "for now" in the owner's answer is
what makes that a schedule rather than a prohibition.

## GB-5 — this branch's 2026-09-03 note; the ruling itself is at §GB-5 below

> **Merge note, 2026-09-10 (`merge/ke16-into-render`).** Both branches carried a `§GB-5` section.
> `feat/threadpool-ke16`'s is the ruling written where it was taken, and it is the primary text:
> it stands **below**, unedited, as the last section of this file. `feat/multi-paradigm-render`'s
> was its 2026-09-03 sync restating the same ruling from a distance, and it is reduced to this
> pointer plus the one thing it recorded that the primary does not — its provenance paragraph,
> kept verbatim here.

*Recorded on this branch 2026-09-03. The ruling landed on `feat/threadpool-ke16` in `b6c41237`
(2026-08-31), a commit that touched that branch's `gaia/DECISIONS.md` and nothing else — which is why
every index, on both branches, went on printing GB-5 as OPEN. The full ground is at that branch's
`gaia/DECISIONS.md` §GB-5.*

⚠ **2026-09-10: that ground is now IN THIS FILE**, at §GB-5 below, brought here by the merge
`merge/ke16-into-render`. The paragraph above is the record of why the 2026-09-03 sync was written,
not a live pointer at another branch.

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

---

## 2026-08-30 — owner directions recorded for a later research pass (NOT launched)

Four directions the owner gave after the F4/F2 rulings landed. **Recorded, not acted on**, on his
instruction: *"record it, we'll run the additional research later."* The standing priority he set
governs all of them: **runtime resident memory and runtime performance are what matter; disk size
does not.**

### D-1 — move the post-load fixup to BAKE time wherever it can go

Owner: *"I think where possible the post-load pass should be done before runtime, i.e. baked
properly if that is possible. Maximum performance."*

**This is more available than F4's ballot assumed.** The ballot held that the `requires` closure is
unbakeable in principle because `RequiredCtor` is `unsafe fn(dst: *mut u8)`. F4's research
**refuted that by measurement**: the ctor is capture-free by construction, and the baker is a Rust
program linked against the derive-emitted tables, so it calls the pointer trivially — demonstrated by
resolving and calling one from a downstream crate. The unbakeable claim was about the *language*
evaluator over Gaia text, not about the baker.

The four sub-passes therefore split unevenly, and the research owes a per-mechanism verdict rather
than one answer:

| sub-pass | bakeable? | note |
|---|---|---|
| 1 — `requires` closure | **fully** — the baker constructs the component and writes it as an ordinary column; runtime cost goes to zero | already measured callable |
| 2 — declared flags | **yes in principle** — `flags_direct_for` is a pure function of the component set, which the baker knows | ⚠ blocked by **F9**: flag state is *not representable in the format at all* today, failing on both sides of the round trip |
| 3 — entity remap | **no** — ids are assigned at load, by construction | |
| 4 — hooks | **partially** — a relationship reverse index is derivable from the forward FKs, so the baker can write it; asset refcounts need a live `AssetServer` and are irreducibly runtime | this is the expensive sub-pass (per entity × component, with a drain to a fixpoint), so partial removal is still the largest win available |

⇒ **The research question is not "can it be baked" but "which effects are functions of the file
alone".** An effect computable from the file's own content is bakeable; one that needs a live world,
a runtime id, or a runtime service is not. That test, applied to all five F4 mechanisms, is the
deliverable.

### D-2 — split baked assets across several files

Owner, on load spikes: *"maybe it makes sense to split baked assets into several files so they load
separately, and then there would be less reserved empty space."*

**Correct instinct, but the benefit is not primarily the one stated, and saying so is the point of
recording it.**

* **Resident memory — conditional.** `POOL_MIN_SLAB`'s 64 KiB floor is paid per non-empty column of
  an **archetype**, not per file. Splitting the file does not reduce the archetype count once
  everything is loaded, so the floor is unchanged. The saving is real only if chunks are
  **unloaded** — i.e. it is the streaming half, which the owner already ruled in (F5, everything
  from the start).
* **Load spikes — direct and larger.** F4 sub-pass 4 is work proportional to entities × components
  with a fixpoint drain. Splitting the world into cells breaks one long spike into many short ones,
  and that holds **whether or not anything is ever unloaded.**

⇒ The honest framing for the research: **splitting is an amortisation mechanism first and a memory
mechanism second**, and the memory half is a consequence of unloading rather than of splitting.

### D-3 — the resident floor itself

`POOL_MIN_SLAB = 64 KiB` (`constants.rs:103`) is deliberate and its comment states the intent — *"the
floor that keeps sparse archetypes cheap (a 1-row archetype commits 3 × 64 KiB per pool, not
megabytes)"*. It is also the **OS commit granule**, so it cannot simply be lowered.

⇒ The available direction is **sub-granule packing** — several small columns sharing one commit
slab. That is real kernel work, and it pays beyond Gaia: every sparse archetype in the engine
currently pays the same floor. The F2 finding (twenty small tables ⇒ ~5 MiB resident for 40 KB of
payload) is one symptom of an engine-wide property, not a data-language problem.

Owner: *"if it can somehow be fixed, it should be fixed."*

### D-4 — the priority that governs all of the above

Owner: *"runtime RAM and performance are the priority. Disk space is not so important."*

⚠ **This changes an evaluation axis, not just a preference.** A format decision that trades bytes on
disk for less work or less resident memory at runtime is now the preferred trade, and any ruling
argued on file size must be re-read against it.

---

## GB-5 — RULED BY THE OWNER, 2026-08-30: **permit as SEED**

**The ruling.** A scene document **may** declare an engine-derived field. The authored value is the
**initial** value; the engine takes it over if and when its condition holds. Owner's ground, and it
is stronger than the ballot's framing: *"the third is the most logical — it is simply a starting
point in space; obviously this data exists to be manipulated and will not be static."*

⇒ A field the engine derives is, by definition, a field that changes. "Initial value" is therefore
its honest semantics, not a concession.

**Rejected, with prices.** *Refuse* — would also forbid the cases where the authored value works,
which is every entity lacking the driving component; and it would force the baker to decide a
question it cannot see. *Permit silently* — the silent-wrong-answer class this campaign spent the
day removing. *Refuse conditionally* — requires the baker to reason about the entity's other
components, and for two of the twelve about their **runtime values**, which bake cannot do.

### Why the ruling generalises across all twelve, checked field by field

The seed reading was tested against the measured list rather than assumed from the spatial example
it was reasoned from:

* **Spatial** (`PointLight.position`, `SpotLight.position`/`.direction`,
  `DirectionalLight.direction`, `Transform.translation`/`.rotation`, the two whole-`Transform`
  camera cases, `RigidBody.position`/`.rotation`) — a seed is a starting pose. Direct fit, and the
  engine already ships this exact semantics: `SpotLight::new`'s `direction` is documented as *"only
  a SEED: `light_reconcile` overwrites it"*
  ([`light.rs:1207`](../../crates/boyko_render/src/light.rs)).
* **`ContentSize.width`/`.height`** — this is where seed stops being a concession and becomes the
  only correct answer: **until the font loads there is no measurement at all**, so the authored
  value is the only value there is, and it is what prevents a layout pop.
* **`UiValue.0`, `UiTextBuffer`** — the value before the binding first fires. Same argument.
* **`UiLayout.width`/`.height`** — the extent before the `Bar` fill takes the axis.

### The case the ruling dissolves rather than answers

**`Transform` and `RigidBody` are a polarity pair.** Which side is author-owned flips on
`Simulated` — a bitset bit gameplay toggles **at runtime**. Under *refuse* the baker would have to
guess which of the two to reject and would be wrong half the time. **Under seed there is nothing to
guess: both are permitted, both are initial values, and the polarity stops being a question the
baker has to answer.** This is the strongest argument for the ruling and it was not in the ballot.

### What the ruling still requires — and the form it must NOT take

Permitting is not the same as staying silent. An author writing such a field should be told it is a
seed, which means the derive-emitted field table needs a disposition column, which G1 was blocked
from minting until this ballot was answered.

⚠ **It must not be a boolean.** GB-5's own analysis refuted that mechanism: *"a per-field boolean
pins a predicate that is false for every case it covers"* — every one of the twelve is derived
**conditionally**, so "this field is engine-derived" is a false statement about most entities that
carry it.

⇒ **The column records the CONDITION and the WRITER, not a verdict**: *this field may be taken over
by `light_reconcile` when the entity has `GlobalTransform`*. That claims nothing about a particular
entity, so it cannot be false, and it is exactly what a diagnostic needs in order to say something
true to the author.

⚠ **The owner's acceptance condition, recorded as a requirement on the implementation rather than an
assumption:** *"if the marking costs nothing at runtime and is purely for convenience, then yes."*
The column must therefore sit behind the same default-off bake feature as the rest of the bake
machinery — Gaia's ratified pipeline is *zero reflection in the shipped load path*. **If it turns
out the column cannot be kept out of the game binary, the condition is not met and this returns to
the owner.**

**Timing, and why it was cheap to answer now.** `grep -rn "field_table\|FieldTable\|field_by_name"`
over `boyko_macros` and `boyko_ecs` is **empty** — G1 is unstarted, so "before the freeze" meant
before the table's shape was designed. Cost now: one attribute at the component definition site, one
column in the derive-emitted table, **zero consumers to update**. Cost later: a hand-maintained side
list keyed by (component, field) — an object this corpus has already priced, since four
hand-maintained lists of one vocabulary were the measured cause of `.ui`'s defects, and a printer
losing 10 of 19 components under a gate structurally blind to the loss.

**Unblocks:** the **G1 field-table freeze**, which was the only thing GB-5 was holding.
