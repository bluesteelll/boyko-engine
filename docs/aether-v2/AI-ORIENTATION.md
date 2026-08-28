# AI-orientation — the requirement set (rung R8)

Closes the owner directive of 2026-08-28 (CAMPAIGN.md §AI-oriented) with a research-grounded,
oracle-carrying requirement set. Evidence quality is flagged throughout; everything marked
MEASURED was reproduced in-session on this checkout with probe crates.

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

## Measured defects (all with committed red-first repros)

| | Defect |
|---|---|
| G4 | block-level checks return on FIRST violation — a block with two independent block defects reports one (MEASURED); each hidden defect = one wasted round-trip |
| G5 | recovery resync SWALLOWS a typo'd construct keyword after a broken construct (`scen lab {}` diagnosed alone, silently eaten after a broken neighbour) — exactly the compound AI failure |
| G6 | a two-span defect arrives as TWO unlinked JSON records — an agent double-counts and cannot pair "first … is here" |
| G3 | **the root cause of most gaps**: `aether_lang` was deliberately built engine-free "for tooling" (decision A2), then given exactly ONE public entry — `expand_block` — which destroys structural errors into `compile_error!` token soup. The architecture is right; the API is one function short |

## The requirement set (verdict · oracle · rung)

| # | Requirement | Oracle (red-first where marked) | Rung |
|---|---|---|---|
| AIR-01 | `check_block(TokenStream) -> Vec<AetherDiagnostic>` beside `expand_block` (which becomes its renderer — goldens hold) + an `aether check --format=json` bin; the Gaia baker owns the same envelope from its FIRST commit, spans always into author TEXT. Closes G1/G2/G3/G6 in one piece — the best buy in the set | red-first: did-you-mean fixture yields non-null `code` + `suggestions[0].replacement`; a duplicate-name defect yields ONE record with primary+secondary (today: two) | R8 |
| AIR-02 | Stable diagnostic codes `AE####`/`GA####` over the existing refusal taxonomy; on the rustc channel the code rides as a pinned `[AE0107]` text prefix; `aether explain` resolves each. The discipline is already built in this repo — `boyko_log` `codes!{}` with orphan/premature-emitter checks — port the pattern | census over every `diag::err` site; a registered-but-never-emitted code is red | R8 |
| AIR-03 | The refusal checklist, mandatory for every new refusal of both languages: code · narrowest author-token span · exhaustive legal vocabulary tied to the dispatch table · did-you-mean ≤2 as DATA · one error per defect · expert-written wording, NO generated explanatory prose (the one controlled study of enriched errors is negative) · trybuild golden · multi-span joined on the check channel | checklist in the DX list; census for codes | policy now, R8 |
| AIR-04 | Block defects ACCUMULATE (fixes measured G4) | red-first: probe block E yields two errors | R3 |
| AIR-05 | Resync stops swallowing typo'd construct keywords (fixes measured G5) | red-first: probe block C yields two errors | R3 |
| AIR-06 | Introspection, TWO artifacts never mixed: (a) grammar schema GENERATED from the parser dispatch tables into a committed versioned JSON manifest + regenerate-and-compare test; (b) project schema (what THIS workspace declares) as CLI/build artifact only — Aether as an expansion by-product, hand-written derives via `boyko_reflect`, Gaia free from the baker. Closes the failure the BMW case measured as heaviest (cross-file semantics, not syntax) | byte regenerate test; red-first: a probe-crate component must appear in the dump | R8 / Gaia |
| AIR-07 | Canonical formatter as a hard gate, both languages: `aetherfmt` over the `aether_lang` AST (canonical group order fixed by spec; parser stays order-free — write freely, normalize on save); Gaia's canonical printer in the grammar from day one. Permitted claims: semantic diffs, corpus equality oracle, "one canonical form" becomes executable. FORBIDDEN claim: model accuracy (frontier models vary <1.6% on stripped input; no study shows canonical form improves LLM edits). Overrides the v1 `aetherfmt` non-goal — recorded | `fmt(fmt(x)) == fmt(x)` corpus-wide; `parse(print(parse(x))) == parse(x)`; red-first comment-survival test | R8 |
| AIR-08 | Stable node ids in Gaia — the one decision with no cheap second chance: author-visible file-local node-id + global asset-id (Unity fileID/GUID precedent); baker emits id→text-range manifest; NO unaddressable nodes (batch-spawned anonymous nodes get base-id + index). Same identity closes the recorded streaming hazard — one work, two payoffs | red-first: reorder/reformat/rewrite fixtures with all cross-refs resolving; dangling id = coded diagnostic naming the target; round-trip preserves ids byte-exact | Gaia spec |
| AIR-09 | Edit formats: REJECT any bespoke patch grammar. Agents ride unified diff and exact search/replace; all leniency lives in the APPLICATOR; an ambiguous anchor is a refusal, never a guess (Diff-XYZ: loosened hunk headers make apply WORSE; aider: disabling lenient apply = 9× edit failures). For Gaia, `{node_id, key, value}` edits applied BY A TOOL with canonical re-print — a tool, never language syntax | id-addressed edit lands after an unrelated reformat+reorder; ambiguous text anchor refuses | R8 / Gaia tooling |
| AIR-10 | Familiarity + false-friend audit for every NEW keyword: a familiar word with unfamiliar semantics is presumed worse than a novel word until measured; measurement protocol = N generations per candidate against the trybuild corpus (the harness exists). Risk list before R3 hardens: `set`, `flag` vs `tag`, `each`, `attributes`, `relation`. Ratified words are not relitigated without measurement | a DECISIONS line per new keyword with audit or measurement; the script committed beside the corpus | before R3 hardens |
| AIR-11 | ONE compact generated gated surface file per language (~150 lines: EBNF spliced from the dispatch tables between sentinels + construct table + one exemplar block verbatim from a COMPILING gate test + refusal-code table). Kills the current duplication (two drifting cheat-sheet copies, neither gated). The 2725-line reference stays for humans; this is the documented thing an agent loads (~100 lines sufficed for a 98-production language; overload is actively harmful via lost-in-the-middle) | byte regenerate test; file in GATED_DOCS; exemplar from a compiling test | R8 |
| AIR-12 | Check-loop speed as a RECORDED budget with conditions/date/machine: warm edit→diagnostic ≤1 s (measured 561 ms), `aether check` ≤100 ms/file, `gaia check` separable from full bake. House rule binds every added checker: lands red-first on a known-bad fixture and reports WHAT it checked (counts), not just exit code. Not a wall-clock CI gate (timing gates flake). Today's only recorded number is the pessimistic 31.7 s cold — actively misleading | per checker: a committed red fixture; counters in output; both numbers in the campaign doc | R8 |
| AIR-13 | v1→v2 migration diagnostics — the sharpest AI-specific pivot risk: every agent with prior exposure will emit v1 forms, which today die as unknown-key noise; the version header is the ONE unrecoverable position. Ruling: the header becomes load-bearing for v2 blocks; a v1 form in a v2 block gets a MIGRATION diagnostic with the exact v2 spelling; header failures recover like any construct | red-first: `tag X(bitset);` in a v2 block → exactly ONE coded migration diagnostic naming `flag X;`; a broken header leaves neighbours expanding | R3 |
| AIR-14 | The reasoning lane: first-class comments in both grammars, surviving the formatter; no construct requires a computed value before a position where the derivation can be written | shared with AIR-07's comment-survival test; grammar-review item | R8 / Gaia spec |
| AIR-15 | Tolerant input (BAML-style near-miss normalization): **REJECTED** — the round-trip it would save costs ≤561 ms and ~0 under `aether check`, while the price is a relapse of the measured S2 class (two grammars per construct — the `at` lesson). The same repair arrives without forking the grammar: machine-applicable did-you-mean (AIR-01) + applicator leniency (AIR-09). **Owner-visible**: if overridden, the only admissible form is asymmetric (parser-accepts-wider, formatter-emits-one, every normalization a coded note) | negative goldens stay goldens; no test is re-blessed from refusal to acceptance without an owner line | — |
| AIR-16 | Gaia grammar rulings, on the spec from day one: delimited whitespace-insensitive text · author text optimized for WRITE reliability, never token compactness (the binary owns size) · ONE syntax for in-file and in-code forms · schema version in the file · cross-reference resolution as the MAIN gate with coded diagnostics naming targets (grammar conformance is the cheap gate) · many small files, whole-file rewrite stays viable · first-class patch/override semantics | AIR-17 + red-first per item once the baker exists | Gaia spec |
| AIR-17 | The repo carrier for Gaia's inherited decisions (canonical printer, byte round-trip gate, stable ids live ONLY in session memory today — the measured "nobody knows this rung exists" class): Gaia's DECISIONS.md carries them as ratified lines with the original rationale AND the AIR cross-note, before the first grammar commit | the lines exist before any grammar lands; cross-refs resolve | Gaia G0 |
| AIR-18 | The directive gets a rung: **R8 — AI-orientation tooling** (AIR-01/02/03/06/07/11/12/14), depends on R3; AIR-04/05/13 fold into R3; Gaia items bind through AIR-16/17 | every oracle demonstrably red before its fix; probes C and E are already-committed repros | CAMPAIGN |

## What AI-orientation is NOT (equally binding)

- **N1** — no runtime price: all of it is tooling or compile/bake-time; zero bytes in the shipped
  game; symbol-census oracle (the `boyko_reflect` precedent).
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

## Unverified (kept explicit so later docs cannot launder it into fact)

Per-principle contributions of the one designed-for-LLM DSL study (unablated; it lost its own
"hard" category); ANY measured agent delta for SARIF, canonical formatting, schema introspection,
error codes, LSP/MCP, llms.txt (adoption + mechanism only, everywhere); the exact Idris table
numbers (direction firm, one surveyor could not re-extract them); keyword-level false-friend
effects (no literature — hence the AIR-10 protocol); "a stable proc-macro cannot fill rustc's
`code` field" (undisputed but untested directly; every sketch routes around it); all in-session
timings are one machine, one block shape, a mid-edit workspace — re-measure on a clean tree before
citing as constants.
