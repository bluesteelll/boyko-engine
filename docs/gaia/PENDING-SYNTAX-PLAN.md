# PENDING — syntax rulings and the conflict-audit fix plan

> **STATUS: NOT APPROVED, NOT COMMITTED.** Owner instruction 2026-08-28: nothing is committed
> without explicit approval. Owner instruction 2026-08-29: *update all the plans and write them
> down* — this file is written, not committed.
>
> **The repo's Gaia syntax is still pre-R1.** [`LANGUAGE.md`](LANGUAGE.md) shows
> `abstract template`, `entity @gate_01`, colon-less `Transform pos=`, `row @iron_sword`,
> `@health_bar bar`. That is the diverged-pair state this repo's own rules call worse than a
> missing doc — it is finding M6 below. The M6 rewrite is **owner-gated**; only interim
> drift-reduction annotations have landed in `LANGUAGE.md` so far.
>
> **Revision history.** REV 1 (2026-08-28) recorded the review rulings. REV 2 (2026-08-29) folds in
> two adversarial passes: a conformance audit of worked examples (35 findings surviving refutation,
> merged to 21) and a corpus revision (57 findings surviving refutation). Two of REV 1's own claims
> were **engine-refuted** and are corrected below, marked ⚠️.

## Part A — Gaia syntax rulings settled in review (with audit amendments)

**Id namespace.** `R1`–`R7` in this table are **Gaia syntax rulings**. They collide by spelling with
the **Aether campaign rungs** `R0`–`R8` in
[`../aether-v2/CAMPAIGN.md`](../aether-v2/CAMPAIGN.md), which this corpus also cites: an unqualified
`R4` would resolve to "declaration bare, reference `@name`" here and to "parallel event emission"
there. A bare `R#` is legal only in the table that DECLARES it — this one, and the Aether rung
ladder. Every **cite** carries its series name at the site, and a cite that crosses from one
directory into the other is a defect without it.

**Finding ids.** `M#`, `C#`, `I#` and `G-##` in the Audit-status column are **labels from the
2026-08-29 conformance audit**, one prefix per hunting lens. Their full bodies are **not carried in
the repo** — only the summary at each use site here. Two consequences, both deliberate: a reader
cannot follow them anywhere, so each use site must stand alone; and `C2` in this table is an audit
finding, **not** one of the `C1`–`C8` construct rulings in
[`../aether-v2/DECISIONS.md`](../aether-v2/DECISIONS.md), which the spelling would otherwise
resolve to. Promoting the audit record into the repo, or renumbering these to a `PS-` prefix, is
bookkeeping deferred until the M6 rewrite lands.

| # | Ruling | Audit status |
|---|---|---|
| R1 | **Anchors**: a component/node head is `Name:`, a field is `name=`. A colon means "fields follow"; heads whose payload is not fields carry no colon (`remove`, `open`, `use`, `entity`-with-children) | **AMEND (M13)** — false four ways in its own nine-line example (`MeshRef: "…"` single value; `bind: value source=…` positional-then-fields; `template T(x: f32)` type ascription; `patch …=…: …=…` two field namespaces). Amend to: payload shape is enumerated **per head** in the generated construct table (AIR-11); the typed-parameter colon stays as the pair's shared `name: Type = default` spelling |
| R2 | **No commas between fields.** Commas only inside value tuples and in `contract` | **AMEND (C2)** — restate the pair-wide law: *commas separate items inside parentheses; Aether's typed field-declaration braces take commas; node/group/statement braces never do.* Convert Gaia `contract` to a paren list. A comma between Gaia fields is a coded refusal |
| R3 | **Whitespace never significant.** Braces only for containers with children; a multi-line component needs no braces | HOLDS. Needs the field-run termination rule written down (M17: `a=1 -2` has two parses — parenthesize arithmetic, refuse bare binary expressions at value position) |
| R4 | **Declaration = bare name; reference = `@name`** (cross-file `@alias#name`). `&` rejected | **AMEND (M16) + BALLOT GB-3** — `@` currently covers BOTH ratified reference kinds, and the corpus needs **three** (see Part D). Identity keeps `@`; lexical/copy positions go bare (`extends sword_base`). Separator drift (`/` in DECISIONS vs `#` in review) and the left operand (`use`-bound alias vs inline `@asset/object` path) must both be settled in writing |
| R5 | **One anchor per node**: a name if the node has identity, a keyword if not. Hence `row` deleted, `abstract template` → `template`; `entity` and `key` survive | **AMEND (M20)** — `abstract sword_base` is the double anchor R5 deletes; amend to "anchors are counted over identity; a modifier keyword is legal". Consequence: `abstract` **stays** on `template` too — one amendment must read the same on both constructs (G-09) |
| R6 | **Case as a mechanical gate**, three classes: UpperCamel = component type, lowercase = closed-list operation, snake_case = object name | **REFUTED as stated (M12)** — the classes are not disjoint (snake_case IS lowercase); UpperCamel covers non-components (`Px(240)`, `PauseGame`, `Torch`, `SwordDef`); RON-style enum values are UpperCamel. Replace: case becomes a **lint** (Aether's own model); the parse gate is **position + a generated reserved-word table** with a quoted-identifier escape; inside a defs block, every head is a row by position, never by case |
| R7 | The closed operation list | **AMEND (I2, M12)** — missing `asset`, `as`, `in`, `euler`, `profile`, the ladder values; widget words would make it unclosable. Generate the list from the parser dispatch table (AIR-06), never prose. ⚠️ **Precondition**: AIR-06 needs the single-dispatch-table refactor, which is a rung *before* **Aether rung R3** ([`../aether-v2/CAMPAIGN.md`](../aether-v2/CAMPAIGN.md) §Rung ladder — an Aether rung, not a ruling in this table). Regenerate the list after ballot GB-4 settles the instance-linkage words |

## Part B — fix plan, ordered by what blocks what

### Tier 0 — semantic holes; land before G1/G2 freeze anything

| | Finding | Fix |
|---|---|---|
| **M1** | **`requires` closure is not delivered to Gaia-loaded entities — MEASURED.** Nothing on the load path adds a component the file omitted; `RequiredCtor` is an `unsafe fn(*mut u8)`, so ctor-form requires are unbakeable in principle. Result: `query<(&Health, &Regen)>` silently skips level-authored entities while gameplay-spawned ones match | GK-4's field tables also emit the require closure; bake expands it; ctor-form requires on Gaia-reachable components emit a const POD from the derive or are refused. Red fixture: bake `Health` alone, load, assert `Regen`. **F4 is widened** to enumerate every insert-path mechanism (requires, flag initial state, relation reverse index, refcounts), not just hooks |
| **M7** | ⚠️ **CORRECTED.** REV 1 said "a partial component record has no source for missing fields" and cited `Transform` as the witness, claiming it derives no `Default`. **The engine refutes the witness**: `boyko_scene::Transform` has `impl Default` returning `Transform::IDENTITY`. The hole is real but sits one level down — at **FIELD granularity**. A closed, total, build-time evaluator cannot spell the neutral of an individual omitted field, and no per-field default table exists. Whole-struct `Default` does not close it either: a component with a non-defaultable field (`Health`'s `link last_attacker: Entity`) or a field with no meaningful neutral (`PointLight::range`) has no `Default` to consult, and **consulting Rust's `Default` trait at bake is out of scope by ruling** — the bake is build-time-reflection-only over the GK-4 tables | **The single best alignment buy**: give Aether `component` fields the `name: Type = default` form that `attributes` and Gaia `template` headers already share; the derive lowers per-field defaults into the GK-4 table; bake fills from that table and refuses a missing field with no declared default. Must land **before G1 freezes the table shape**. Two red fixtures: (1) a record omitting a field with no declared default → coded refusal naming the field; (2) a record omitting a field that HAS one → the table-sourced value lands in the baked bytes |
| **M5** | **`pillar_$i` is the ratified negative control** — ids minted from a loop index: edit the guard and `@cell_07#pillar_3` still resolves, to a different pillar. No dangle, no diagnostic, wrong world (the Terraform-count class our own DECISIONS bans by name) | Forbid `$` in the name slot; loop-generated nodes get AIR-08 base-id+index compound ids and are refused as cross-file targets. **Decide name-vs-id once** (ballot F8): if the name IS the id, delete the `--assign-ids` machinery and add rename tooling; if not, R4 grows an id slot. ⚠️ F8 **reopens** a ratified identity ruling — AIR-10 requires the trigger and the measurement to be stated. Add the guard-edit red fixture |
| **M4** | **ZST tags and flags — the commonest scene content — have no Gaia spelling**, and flag initial state has no format carrier (bitset ids are filtered out of the loaded signature on the load path) | Bare UpperCamel head, no colon, no body = attach ZST tag (generalizes R1's no-colon class). Flags: ballot **F9** — a per-entity enable-bit region in the format plus one spelling, or a coded refusal naming Aether's `flags (…)` group. **Before G2/G6** |
| **M3** | **`link` has no cross-check between its two carriers.** Gaia `link` on a field not declared `link` in Aether → an entity id `LoadEntityMap` never remaps — the stale id the keyword exists to prevent, *with the safety word written*. Omitted on an `#[entities]` field → the keyword is decorative | Bake refuses **both** directions against the GK-4 field table; diagnostics name the Aether declaration site by `stable_name`. Two red fixtures at G3 |
| **M6** | **The repo's only Gaia syntax is pre-R1 on essentially every line, and R1–R7 exist nowhere.** An agent that reads the spec generates wrong-on-every-line output (AIR-17 + AIR-13 classes at once) | Write R1–R7 (as amended) into [`DECISIONS.md`](DECISIONS.md) and rewrite `LANGUAGE.md` **in the same commit**; add a grammar epoch to the header so pre-R1 forms get coded `GA####` migration diagnostics. Owner-gated. Interim: `LANGUAGE.md` carries drift-reduction annotations only |

### Tier 1 — false friends and silent misparses

| | Fix |
|---|---|
| `key` | **Rename the GAIA side to `keyframe:`** — NOT Aether to `fk`. Aether's `key` is ratified (S5) with a rejection record; AIR-10 forbids relitigating ratified words without measurement; `fk` names the mechanism, the exact ground `remap` was rejected on. `point:` is refused — it would collide with Aether's scene `point` light head. Also: state that a Gaia-authored relationship FK is spelled `link`, and widen AIR-10's audit to the pair's **joint** vocabulary (⚠️ that widening is itself a scope change to a ratified requirement — ballot AB-10) |
| `text:` | **Delete from v1.** `text` is in the closed list, so `text: "Pause"` typed as a field (the Rust colon habit) **silently misparses** as the button-text head. `UiText:` already exists |
| Gaia `table` | **Rename** (`defs` / `catalog`). Aether's `storage = table` means archetypal/non-dense; Gaia's `table` bakes to **dense** — a reader learns an anti-fact. Aether's is the kernel's own word and stays. Interim: bake refusal on the contradictory pairing |
| template use | **No syntax exists for USING a template**, and `Torch: intensity=2.0` is lexically a component attach. Add a keyword anchor: `apply Torch: …`. Blocks G4. ⚠️ Which ladder layer the expansion occupies is **unruled** — ballot GB-1 |
| `use` ×2 | Style application becomes **`apply <style>`**. ⚠️ **REV 1's justification is withdrawn**: it read "a style is a declared node, so its reference carries `@`", which contradicts R4-as-amended on the same page — R4's criterion is identity-vs-copy, **not** declared-vs-not. A style is flattened by bake, so it is a copy and the reference goes **bare**. The "declared node ⇒ `@`" rule is deleted. Final sigil disposition rides ballot **GB-3** |
| widget sugar | **Drop lowercase `bar`/`button`/`label` from v1.** The vocabulary is unclosable — every new widget steals a legal object name. If sugar returns, it is engine-shipped UpperCamel templates riding the same expansion anchor as the template fix |
| `PointLight` field names | ⚠️ **ENGINE-REFUTED, and the defect is in this file too.** The sketch and this plan both wrote `intensity=`; the struct field is **`power`** (luminous flux Φ in lumens; `boyko_render::PointLight`). `range` has no default and a ctor requires it. `position` is **engine-derived** — `light_reconcile` computes it from `GlobalTransform`, so it must be undeclarable in the scene profile (ballot GB-5). And `color` is `[f32; 3]` documented **LINEAR rgb**, against a four-component `#RRGGBBAA` literal: an arity mismatch *and* a transfer-function question (ballot GB-2). Fix every occurrence here before the M6 rewrite re-emits the defect in the new syntax |

### Tier 2 — rules to restate

`contract` → paren list (C2) · patch modifiers into the head parens, `patch(@sconce_a.PointLight, layer=variant): power=2.5` — also fixes the comma law and matches `apply`/`copy` (M11) · `..` in `contract` requires `..=` or `[a,b]`, pinned with both-endpoint fixtures · **`instance … from …` is withdrawn** — `from` was minted by REV 1 and appears in no ratified vocabulary; the re-spelling goes to ballot **GB-4** · curves get names and bake-checked references (⚠️ REV 1's `@`-for-curves is **retracted** pending GB-3 — only the sigil is in question, not the naming) · two symmetric coded refusals for the brace/colon crossing (`Name { name=… }` and `Name: { … }`), with `gaia fmt` printing one canonical multi-line layout · `for … if …` guard form wins; `if` leaves head position; the bake-budget refusal says "this loop runs at BAKE time and materializes N nodes" and is the **expansion-axis** diagnostic of the four-axis budget fixture set

### Tier 3 — Aether-internal (found by the same audits)

`state` twice inside one `machine` (group vs chart node) → rename the group `fields { }` · `after` twice (order edge vs machine timer) · `when` twice (run-condition vs polled transition) · **`on` three times** (`machine … on entity`, `on E => T`, `flags (X = on)`) · singular/plural pairs where the plural is not the plural: `flag/flags`, `tag/tags`, `set/sets` — `sets → in` is the cheap rename; ⚠️ **`flags → initial` is WITHDRAWN** — `initial` is already the machine's initial-state keyword, so the rename recreates the exact defect it fixes. The value vocabulary and the three `on` positions go to ballot **AB-13** · `mut` in three positions · `requires (X = expr)` vs `flags (X = on)` — same shape, disjoint value spaces; pre-check both so the span lands on the author's token · group placement (before/after body) differs per construct — state the law and say which side `each` is on · **M2**: scope the `hooks` and `relation` claims in CONSTRUCTS.md to hook-fired spawns; until F4 lands, a hook-dependent component in a baked asset is a coded bake warning naming F4 · **M8**: specify the `stable_name` resolution axis once (authored text uses the Rust type name; bake resolves through the same-build manifest; rename = loud bake error listing affected files) · **M10**: bake refusal for any `Entity`-carrying field authored without `link` (the R-RES twin) · **M14**: the derive emits bundle types into the manifest so `Pawn:` is refused with the truth, not "unknown component" about a type that exists (ballot **F10** — and Part D vs this row disagree on whether M14 is a ballot at all; that pre-question is the ballot's first item) · **M19**: **AIR-14 is unimplementable for Aether as specified** — `//` comments are gone at lex time before `aether_lang` sees the TokenStream, so the shared comment-survival oracle is red by construction or silently Gaia-only. Split it: Gaia carries it via the CST; Aether's half is priced separately or scoped out

### Tier 4 — cheap wins

Header divergence (`aether! {` vs `gaia 1 profile=scene`) — pick one version-token rule · int literals valid in Gaia, type errors in Aether's sugar positions — normalize int→float in Aether's emitted tokens; write Gaia's float-print rule citing the two opposed in-tree conventions · seed the unknown-operation refusal with the Aether construct keywords and a mirror set ("`resource` is Aether's; declare it there and reference it here") — a dozen table rows converting the wrong-language push into one redirect · colour transfer function unstated (ballot **GB-2**; the engine already has a measured member of this class in the normal-map convention) · `/-` vs `//`: converting a slashdash on a multi-line node to `//` leaves the body live one level up — formatter warning on a `//` line ending in an unbalanced `{` · event participant contexts are statically checkable at bake — the router `debug_assert` gets a free static twin

## Part C — alignments that are already right; do not break

`link` as the shared spelling · UpperCamel = type / snake_case = instance in both · `//` comments · `=` means "assign a value" everywhere · Gaia `use … as` matching Rust's import semantics · **and the owner's own ruling: the two languages SHOULD look different** — commas, braces, bracelessness and overall shape are loud-on-error differences that reinforce "you are in the other language". C3 (`for` bake vs runtime), C4 (brace content model), C6 (`entity` type vs keyword), C7 (bracelessness) are all **document-the-difference, do not align** — forcing alignment there damages a correct design.

## Part D — what is NOT decided

**Ballot bodies live in [`../OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md)** (one destination; the
[`CAMPAIGN.md`](CAMPAIGN.md) table is the index). Gaia-side ballots reaching this file:

| Ballot | Question | Blocks |
|---|---|---|
| F1 | bake route into the byte format | G1 |
| F4 | what "loaded" means — **widened** to every insert-path mechanism, with M1's measured `requires` fact as its second bullet | G6 |
| F2 · F3 | DataAsset runtime home · table file shape | G5 |
| F8 | name-vs-id for objects. ⚠️ **reopens** a ratified identity ruling | G3 candidate — owner scoping |
| F9 | flags carrier vs coded refusal (M4) | G2 / G6 |
| F10 | bundle expand-vs-refuse (M14) — first item: *is this a ballot at all?* Tier 3 and this table disagreed | owner scoping |
| **GB-1** | which ladder layer a template expansion, and an UNLABELED write, occupy (K2) | G4 |
| **GB-2** | colour transfer function: `#RRGGBBAA` sRGB-decoded at bake, or raw into a LINEAR field (K12) | G2 / G5 |
| **GB-3** | the reference taxonomy's **third kind** — asset-ref spelling, style-ref kind, `$hole` disposition, "declared-node ⇒ `@`" withdrawn (K3 + K15 + K16). Settle jointly so R4 is rewritten **once**. ⚠️ amends ratified §Identity (an extension consistent with its own rationale, not a reversal) | G2 / G3, and Tiers 1–2 here |
| **GB-4** | `instance` linkage: (a) linkage-in-slot `instance wall_east extends\|copy "…"` vs (b) linkage-in-head (`instance` ≡ live link, `copy` its own head). `from` is deleted either way (K14) | G4 / Tier 2 |
| **GB-5** | are engine-derived fields (`PointLight::position`) declarable in the scene profile? | G1 table freeze |

## Part E — the K-record

The sixteen findings the two adversarial passes established, and where each now lives.

| | Finding | Disposition |
|---|---|---|
| K1 | `Transform` fields are `translation`/`rotation`/`scale`; `pos`/`rot` are unknown keys on a closed record | Determinate. Fixed here (M7) and annotated in `LANGUAGE.md` |
| K2 | template application double-writes a field on one unnamed ladder rung | → **GB-1** |
| K3 | `$hole` is a third reference sigil where two are ratified | → **GB-3** |
| K4 | event `entity` participants need a context, `entity(C)` | Already conformant; the checker's scope was narrowed (the context is `debug_assert`'d only in a machine `inbox` router) |
| K5 | the kernel schedule set is closed `{startup, update, fixed}`; `schedule render` does not exist | No corpus contradiction |
| K6 | id-vs-handle seam: query yields `EntityId`, participants want `Entity` | No contradiction — `Entities` + `Query::get/get_mut` are the ratified closure, already on **Aether rung R1** ([`../aether-v2/CAMPAIGN.md`](../aether-v2/CAMPAIGN.md)), not ruling R1 above |
| K7 · K10 | `without` over a flag is a silent no-op, **including inside CONSTRUCTS.md itself** | Determinate; fixed. ⚠️ polarity corrected: `with Flag` matches *nothing*, `without Flag` excludes *nothing*. Whether v2 refuses the spelling → **AB-11** |
| K8 | a bundle carrying a `link Entity` field has no honest spawn spelling | **Unrouted design debt** — belongs to the **Aether rung R3** design pass ([`../aether-v2/CAMPAIGN.md`](../aether-v2/CAMPAIGN.md)), not an owner ballot. That rung's `bundle` construct must not be declared done while it stands |
| K9 | the kernel `relates`/`related` macro demands a private collection field and mandates `retain_empty` — inexpressible on a pub-field Aether group | **Unrouted design debt** — same disposition. This is the desync `relation` (O4) was ratified to make unwritable |
| K11 | `MACHINES.md` scenario 2 orders after a `regen_mana` it never declares | Determinate; declared in the fragment |
| K12 | colour transfer function unstated | → **GB-2** |
| K13 | `each par` — two kernel drivers with opposite tick behaviour, one word | Recorded determinately (drivers, `BatchingStrategy::default()`, the 1024-row inline floor, the pinned `par_for_each_chunk_entities` signature); the **choice** → **AB-8** |
| K14 | `instance`/`from` vs `extends`/`copy` are two orthogonal axes | **Recorded, not applied — owner instruction 2026-08-29.** → **GB-4** |
| K15 | Tier 1's `@`-for-styles contradicts R4 on the same page | Withdrawn here; sigil → **GB-3** |
| K16 | the reference taxonomy names two kinds and needs three | → **GB-3** |
