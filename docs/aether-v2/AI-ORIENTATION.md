# AI-orientation — the requirement set (rung R8)

Closes the owner directive of 2026-08-28 (CAMPAIGN.md §AI-oriented) with a research-grounded,
oracle-carrying requirement set. Evidence quality is flagged throughout; everything marked
MEASURED was reproduced in-session on this checkout with **probe crates that were never
committed** — they exist only in the 2026-08-28 session transcript, and **no probe fixture is
present in this repository or in its history**. Every provenance claim elsewhere in this file
agrees with this sentence; where one did not, it was a false claim and is corrected below.

## The load-bearing asymmetry

The literature on *which grammar LLMs write well* is thin and mostly unablated. The literature on
the **loop around the language** — compiler feedback, edit formats, target addressing — is strong,
replicated, with large effects: a no-pretraining language goes 0–1% zero-shot but ~39%→96% under
iterative local compiler errors (Idris/GPT-5, as-claimed); aider measured 26%→59% from nothing but
switching to a pretraining-familiar diff format; constrained JSON decoding measurably *hurts*
reasoning (76.6%→49.3% on GSM8K). **Budget order therefore: diagnostics > introspection >
formatter > grammar tweaks.** A second filter: tools that compensate model weakness depreciate as
models improve (SWE-agent's edit micro-interface → mini-SWE-agent's bare bash); tools that provide
information the model cannot derive (what exists, what fired, what is canonical) never depreciate.

## Scorecard — what existing decisions already close

- **Grammar regularity: CLOSED, before the directive existed.** S1/S2 (one spelling, keyword
  payload, no optional brackets) and the unwritable-over-diagnosable rulings were taken for human
  reasons a day earlier; they are exactly the AI rulings. The campaign now says so explicitly.
- **Machine-readable diagnostics: the expensive half is FREE and already works (MEASURED).**
  Every Aether error arrives through `cargo check --message-format=json` as a first-class rustc
  diagnostic — verbatim message, file/line/column, **byte offsets**, source-line echo — because
  `diag::err` never falls back to `Span::call_site()`. Warm edit→diagnostic cycle: **561 ms**
  (floor: 465 ms cargo no-op); pure `expand_block`: 0.054 ms.
- **Gaia's type-level substrate: partially pre-paid by serialization decisions** (`stable_name`,
  `format_version`, loud `UnmappedEntity`, determinism suites — adopted for other reasons).
  *Corrected at Gaia G0*: the original "CLOSED" wording was refuted by the Gaia inventory —
  hooks-on-load, resources-in-format, tick reset, handle carriers and opt-in remap are NOT covered
  by those decisions (see `docs/gaia/DECISIONS.md`, disagreement 5). An unqualified closure claim
  without a census is the gate-that-cannot-fail class, in prose.

## Measured defects (all reproduced in-session with probe crates; none committed yet — see the provenance line above)

⚠ **This heading previously read "all with committed red-first repros", and two oracle cells below
named probes C and E as already-committed repros. No such artifact exists in this repo or in its
history.** That is this corpus's own rule — *a gate that cannot fail is not a gate* — failing on its
own gate: the requirement set whose whole point is red-first evidence asserted evidence it does not
have, and the assertion would have been believed by every later reader. Committing probes C and E
as trybuild fixtures under `crates/aether_tests/tests/ui/` is an explicit **R3 deliverable**
(CAMPAIGN R3); until they land, AIR-04 and AIR-05 name an oracle that cannot be run.

| | Defect |
|---|---|
| AD1 | block-level checks return on FIRST violation — a block with two independent block defects reports one (MEASURED); each hidden defect = one wasted round-trip |
| AD2 | recovery resync SWALLOWS a typo'd construct keyword after a broken construct (`scen lab {}` diagnosed alone, silently eaten after a broken neighbour) — exactly the compound AI failure |
| AD3 | a two-span defect arrives as TWO unlinked JSON records — an agent double-counts and cannot pair "first … is here" |
| AD4 | **the root cause of most gaps**: `aether_lang` was deliberately built engine-free "for tooling" (decision A2), then given exactly ONE public entry — `expand_block` — which destroys structural errors into `compile_error!` token soup. The architecture is right; the API is one function short |

**Renumbered `G4/G5/G6/G3` → `AD1..AD4` on 2026-08-29.** The old ids collided head-on with Gaia's
rung ladder `G0..G8`, which this same corpus cites (see "Corrected at Gaia G0" above — that one is
a real Gaia rung). The rename also exposed a second defect: **two ids of the original series, `G1`
and `G2`, were never defined anywhere in this file**, yet AIR-01 claimed to close them. Whether
those two dropped defects exist in the session record — and should return as `AD5`/`AD6` with their
own repros — is the **non-blocking owner query AB-12** below; the renumbering and the trimmed
closure claim are correct under either answer.

## The requirement set (verdict · oracle · rung)

| # | Requirement | Oracle (red-first where marked) | Rung |
|---|---|---|---|
| AIR-01 | `check_block(TokenStream) -> Vec<AetherDiagnostic>` beside `expand_block` (which becomes its renderer — goldens hold) + an `aether check --format=json` bin; the Gaia baker owns the same envelope from its FIRST commit, spans always into author TEXT. Closes **AD4** (the one-public-entry root cause) and **AD3** (paired multi-span records) in one piece — the two defects in this file it actually reaches. It is high leverage per unit of work, but the earlier "**the best buy in the set**" ranking is withdrawn: that ranking was a count of four closed items, and two of the four (`G1`, `G2`) were never defined here (AB-12) | red-first: did-you-mean fixture yields non-null `code` + `suggestions[0].replacement`; a duplicate-name defect yields ONE record with primary+secondary (today: two) | R8 |
| AIR-02 | Stable diagnostic codes `AE####`/`GA####` over the existing refusal taxonomy; on the rustc channel the code rides as a pinned `[AE0107]` text prefix; `aether explain` resolves each. The discipline is already built in this repo — `boyko_log` `codes!{}` with orphan/premature-emitter checks — port the pattern | census over every `diag::err` site; a registered-but-never-emitted code is red | R8 |
| AIR-03 | The refusal checklist, mandatory for every new refusal of both languages: code · narrowest author-token span · exhaustive legal vocabulary tied to the dispatch table · did-you-mean ≤2 as DATA · one error per defect · expert-written wording, NO generated explanatory prose (the one controlled study of enriched errors is negative) · trybuild golden · multi-span joined on the check channel | **a per-refusal census over every `diag::err` site (160 today)**, landing red-first on a non-conforming fixture and **reporting counts**, not an exit code. Per site it requires: a registered `AE####` code · **≥1 trybuild golden containing that code** · suggestions as structured data, ≤2 · **single-carrier vocabulary** — a message that enumerates a legal vocabulary must be built from the same array the dispatch consults, and `parse.rs`:567 vs :669-673 is the census's **first red**. Pre-existing goldenless sites ride a **committed shrinking-only baseline** with a per-line rationale. Span narrowness and expert wording are **REVIEW items, not the gate** — neither is machine-checkable | policy now, R8 |
| AIR-04 | Block defects ACCUMULATE (fixes measured **AD1**) | red-first: probe block **E** — one block carrying two independent block-level defects — pins error **COUNT == 2**. ⚠ The source exists only in the 2026-08-28 session transcript; the oracle cannot run until it is committed as a trybuild fixture under `crates/aether_tests/tests/ui/` (R3 deliverable) | R3 |
| AIR-05 | Resync stops swallowing typo'd construct keywords (fixes measured **AD2**) | red-first: probe block **C** — a broken construct followed by `scen lab {}` — pins error **COUNT == 2**. ⚠ Same provenance: transcript only, not committed; lands under `crates/aether_tests/tests/ui/` as an R3 deliverable | R3 |
| AIR-06 | Introspection, TWO artifacts never mixed: (a) grammar schema GENERATED from the parser dispatch tables into a committed versioned JSON manifest + regenerate-and-compare test; (b) project schema (what THIS workspace declares) as CLI/build artifact only — Aether as an expansion by-product, hand-written derives via `boyko_reflect`, Gaia free from the baker. Closes the failure the BMW case measured as heaviest (cross-file semantics, not syntax). **PRECONDITION, with its own rung before R3:** restore the single dispatch table — AETHER-LANG-PLAN §6.1's `const CONSTRUCTS: &[(&str, fn(...))]`. Today the construct-keyword set has **THREE carriers** (the dispatch table, `diag::CONSTRUCT_KEYWORDS`, `Stub::for_keyword`) and "generated from the dispatch tables" is therefore not yet a true statement; the census must span all three and **supersede the existing spot-check test, whose name overpromises what it covers**. (b) additionally depends on `boyko_reflect`, which is **not a workspace member** — sequencing is ballot **AB-9** | byte regenerate test; red-first: a probe-crate component must appear in the dump; three-carrier census green before the manifest is trusted | R8 / Gaia |
| AIR-07 | Canonical formatter as a hard gate, both languages: `aetherfmt` over the `aether_lang` AST (canonical group order fixed by spec; parser stays order-free — write freely, normalize on save); Gaia's canonical printer in the grammar from day one. Permitted claims: semantic diffs, corpus equality oracle, "one canonical form" becomes executable. FORBIDDEN claim: model accuracy (frontier models vary <1.6% on stripped input; no study shows canonical form improves LLM edits). Overrides the v1 `aetherfmt` non-goal — recorded | **quantified over PERTURBED input**, which is where a formatter can actually fail: for each canonical corpus file, generate group-order permutations, line-break variants and trailing-comma variants, and assert `fmt(perm_i(x)) == x` for **every** i, **reporting the permutation count**. `fmt(fmt(x)) == fmt(x)` is demoted to smoke — it holds of any idempotent-by-construction printer and cannot fail on the property this requirement is about. `parse(print(parse(x))) == parse(x)` stays. Comment survival **SPLITS per finding M19 of [`docs/gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) §Tier 3** (that file's own `M#` finding series — **not** this corpus's `M1`–`M9` machine rulings in [`DECISIONS.md`](DECISIONS.md), which have no `M19`): Gaia's half rides the CST; **Aether's half is priced separately or scoped out — `//` is already gone at lex time**, so a single shared comment-survival test would be false for one of the two languages | R8 |
| AIR-08 | Stable node ids in Gaia — the one decision with no cheap second chance: author-visible file-local node-id + global asset-id (Unity fileID/GUID precedent); baker emits id→text-range manifest; NO unaddressable nodes (batch-spawned anonymous nodes get base-id + index). Same identity closes the recorded streaming hazard — one work, two payoffs | red-first: reorder/reformat/rewrite fixtures with all cross-refs resolving; dangling id = coded diagnostic naming the target; round-trip preserves ids byte-exact | Gaia spec |
| AIR-09 | Edit formats: REJECT any bespoke patch grammar. Agents ride unified diff and exact search/replace; all leniency lives in the APPLICATOR; an ambiguous anchor is a refusal, never a guess (Diff-XYZ: loosened hunk headers make apply WORSE; aider: disabling lenient apply = 9× edit failures). For Gaia, `{node_id, key, value}` edits applied BY A TOOL with canonical re-print — a tool, never language syntax | id-addressed edit lands after an unrelated reformat+reorder; ambiguous text anchor refuses | R8 / Gaia tooling |
| AIR-10 | Familiarity + false-friend audit for every NEW keyword: a familiar word with unfamiliar semantics is presumed worse than a novel word until measured; measurement protocol = N generations per candidate against the trybuild corpus (the harness exists). Risk list before R3 hardens: `set`, `flag` vs `tag`, `each`, `attributes`, `relation`. Ratified words are not relitigated without measurement. **Now SCHEDULED** — this item previously had a deadline and no carrier: it is written into CAMPAIGN R3's oracle ("+ AIR-10: a DECISIONS familiarity/false-friend line per risk-list keyword, present before the R3 surface hardens") and into AIR-18's assignment line below, which sorted every AIR item but this one | a DECISIONS line per new keyword with audit or measurement — written now for `set`, `attributes`, `relation` (wholly absent from DECISIONS) and as a familiarity-axis extension of C1/C5 for `flag`-vs-`tag` and `each`; the lines record the audit and keep the ratified spellings (no rename on audit alone). Whether the measurement **script** is also required when the audit branch is taken is ballot **AB-10** | before R3 hardens |
| AIR-11 | ONE compact generated gated surface file per language (~150 lines: EBNF spliced from the dispatch tables between sentinels + construct table + one exemplar block verbatim from a COMPILING gate test + refusal-code table). Kills the current duplication (two drifting cheat-sheet copies, neither gated). The 2725-line reference stays for humans; this is the documented thing an agent loads (~100 lines sufficed for a 98-production language; overload is actively harmful via lost-in-the-middle). **Carries AIR-06's single-dispatch-table PRECONDITION**: "spliced from the dispatch tables" is not yet true of a three-carrier keyword set | byte regenerate test; file in GATED_DOCS; **exemplar taken from a compiling gate test THAT CONTAINS NO FORBIDDEN FORM** — compiling is not enough, since the forms this campaign bans compile silently. Seeded forbidden-form list: `with`/`without` over a `flag`; any dense arm inside `Or<...>`. The seeding is itself gated by a red fixture — the same shape gaia DECISIONS item 8 already ratifies. A **runtime match-set oracle is REJECTED**: it would make doc generation world-dependent | R8 |
| AIR-12 | Check-loop speed as a RECORDED budget with conditions/date/machine: warm edit→diagnostic ≤1 s (measured 561 ms), `aether check` ≤100 ms/file, `gaia check` separable from full bake. House rule binds every added checker: lands red-first on a known-bad fixture and reports WHAT it checked (counts), not just exit code. Not a wall-clock CI gate (timing gates flake). Today's only recorded number is the pessimistic 31.7 s cold — actively misleading | per checker: a committed red fixture; counters in output; both numbers in the campaign doc | R8 |
| AIR-13 | v1→v2 migration diagnostics — the sharpest AI-specific pivot risk: every agent with prior exposure will emit v1 forms, which today die as unknown-key noise; the version header is the ONE unrecoverable position. Ruling: the header becomes load-bearing for v2 blocks; a v1 form in a v2 block gets a MIGRATION diagnostic with the exact v2 spelling; header failures recover like any construct | red-first: `tag X(bitset);` in a v2 block → exactly ONE coded migration diagnostic naming `flag X;`; a broken header leaves neighbours expanding | R3 |
| AIR-14 | The reasoning lane: first-class comments in both grammars, surviving the formatter; no construct requires a computed value before a position where the derivation can be written | shared with AIR-07's comment-survival test; grammar-review item | R8 / Gaia spec |
| AIR-15 | Tolerant input (BAML-style near-miss normalization): **REJECTED** — the round-trip it would save costs ≤561 ms and ~0 under `aether check`, while the price is a relapse of the measured S2 class (two grammars per construct — the `at` lesson). The same repair arrives without forking the grammar: machine-applicable did-you-mean (AIR-01) + applicator leniency (AIR-09). **Owner-visible**: if overridden, the only admissible form is asymmetric (parser-accepts-wider, formatter-emits-one, every normalization a coded note) | negative goldens stay goldens; no test is re-blessed from refusal to acceptance without an owner line | — |
| AIR-16 | Gaia grammar rulings, on the spec from day one: delimited whitespace-insensitive text · author text optimized for WRITE reliability, never token compactness (the binary owns size) · ONE syntax for in-file and in-code forms · schema version in the file · cross-reference resolution as the MAIN gate with coded diagnostics naming targets (grammar conformance is the cheap gate) · many small files, whole-file rewrite stays viable · first-class patch/override semantics | AIR-17 + red-first per item once the baker exists | Gaia spec |
| AIR-17 | The repo carrier for Gaia's inherited decisions (canonical printer, byte round-trip gate, stable ids live ONLY in session memory today — the measured "nobody knows this rung exists" class): Gaia's DECISIONS.md carries them as ratified lines with the original rationale AND the AIR cross-note, before the first grammar commit | the lines exist before any grammar lands; cross-refs resolve | Gaia G0 |
| AIR-18 | The directive gets a rung: **R8 — AI-orientation tooling** (AIR-01/02/03/06/07/11/12/14), depends on R3; **AIR-04/05/10/13 fold into R3** — AIR-10 was previously assigned to no rung at all despite carrying a deadline, and now rides R3's own oracle; Gaia items bind through AIR-16/17 | every oracle demonstrably red before its fix. ⚠ **Probes C and E are NOT committed repros** — the earlier claim that they were is false; they are transcript-only, and committing them under `crates/aether_tests/tests/ui/` is a precondition for AIR-04 and AIR-05 to be gates at all | CAMPAIGN |

## What AI-orientation is NOT (equally binding)

- **N1** — no runtime price: all of it is tooling or compile/bake-time; zero bytes in the shipped
  game; symbol-census oracle (the `boyko_reflect` precedent — cited here as **methodological
  precedent only: the technique is portable, the dependency is not taken**. `boyko_reflect` is not
  a workspace member; see KERNEL-BACKLOG §Substrate outside the workspace).
- **N2** — no softened refusals: AI-orientation STRENGTHENS refusals; unwritable-over-diagnosable
  matters MORE for a generator, because a generator samples the writable-but-wrong space.
- **N3** — no second syntax: no AI dialect, no tolerant grammar (AIR-15), no compact machine form.
  S2 *is* the AI decision, said explicitly.
- **N4** — no constrained-decoding concessions: the checker/baker is the guarantee; forced
  decoding measurably harms reasoning.
- **N5** — no LSP/MCP server as campaign deliverables: the evidence base is vendor-thin, and
  bare-bash agents are the live counterargument to heavy scaffolds. The asset is CLI+JSON; MCP is
  a wrapper later, if earned.
- **N6** — no verbosity "for AI": the verbose-keywords claim is unablated; nothing gets wordier on
  AI grounds.
- **N7** — no tree-sitter as a precondition: `aetherfmt` rides the `aether_lang` AST; a
  tree-sitter grammar is one deferred post-R8 investment, never counted as two wins.

## Open ballots raised by this requirement set

These are OPEN. Nothing here is decided in this document; each row states the question, the
alternatives, and what it blocks. Bodies live in
[`docs/OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md).

| Ballot | Question | Alternatives | Blocks |
|---|---|---|---|
| **AB-9** | `boyko_reflect` sequencing for AIR-06(b) | R8 waits on the `feat/reflection` merge / AIR-06(b) is descoped to its reflection-free halves (which still satisfy its oracle) / the merge is pulled forward into the campaign | R8 |
| **AB-10** | AIR-10 residue | (a) is the measurement **script** required when the audit branch is taken, or does a DECISIONS audit line suffice? (b) is the widening of AIR-10 to the joint Aether+Gaia vocabulary proposed in [`docs/gaia/PENDING-SYNTAX-PLAN.md`](../gaia/PENDING-SYNTAX-PLAN.md) §Tier 1 (`key`) ratified? ⚠ (b) is a scope change to a ratified requirement and must not arrive as a side effect of an unrelated edit | R3's keyword surface (the audit lines themselves are written now, so R3 is not stalled on this) |
| **AB-12** | *(non-blocking owner query)* do the two dropped measured defects — the old `G1` and `G2`, never defined in this file — exist in the 2026-08-28 session record? | (a) they exist → restore as `AD5`/`AD6`, each with its repro, and AIR-01's closure claim widens again; (b) they never existed → the closure claim stays trimmed as written and the count is closed at four | nothing. The `AD1..AD4` renumbering and the trimmed AIR-01 claim are correct under either answer |

## Unverified (kept explicit so later docs cannot launder it into fact)

Per-principle contributions of the one designed-for-LLM DSL study (unablated; it lost its own
"hard" category); ANY measured agent delta for SARIF, canonical formatting, schema introspection,
error codes, LSP/MCP, llms.txt (adoption + mechanism only, everywhere); the exact Idris table
numbers (direction firm, one surveyor could not re-extract them); keyword-level false-friend
effects (no literature — hence the AIR-10 protocol); "a stable proc-macro cannot fill rustc's
`code` field" (undisputed but untested directly; every sketch routes around it); all in-session
timings are one machine, one block shape, a mid-edit workspace — re-measure on a clean tree before
citing as constants.
