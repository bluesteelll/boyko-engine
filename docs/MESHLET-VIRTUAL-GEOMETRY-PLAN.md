# VG-R0 — "The Ruler": the measurement rung of the virtual-geometry campaign

**Status:** DESIGN, **Rev 12** — **NOT APPROVED, and no code exists.** This document specifies
**only rung R0** of the ladder in [`docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`](MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md)
§4. R1–R8 stay as that document leaves them and are out of scope here. The owner's decision to
build a meshlet / virtual-geometry system is **settled** and is not re-litigated below.

**Rev 9 discharges the six items that blocked Rev 8, and three of them were Rev 8's own.** Its
review scored Rev 8 at **0 hold, 3 partial, 1 does not hold** — the sixth consecutive revision told
it had overclaimed — and the useful part is *which* repair regressed. Of Rev 8's six, the single one
that **extended** mechanism rather than bounding it (R0a's conditioned `(b′)`) is the single one
that went backwards, trading Rev 7's cannot-go-green for a cannot-go-red. Every repair that merely
deleted an apparatus stayed clean. That is the strongest evidence the bounding strategy has, and it
arrived as a counterexample rather than as an argument.

What Rev 9 fixes: R0d's gate list and mutation list used **(c)** for two different predicates,
because demoting the histogram check renumbered one and not the other — a renumbering defect
produced by the bounding act itself; R0a's `(b′)` now asserts something for **every** legal value of
`reason` (enumerated in `[k2_probe]`) instead of nothing for most of them, and has red mutations in
both directions; `[k1].k1_decision_rule` gained the non-degeneracy conjunct, without which one input
had three different dispositions across three codeable texts; R0d's `(a)` mutation was replaced —
a spawn-order permutation cannot falsify a cross-process **agreement** predicate, since the edit is
present identically in all three processes; R0b gained gate part `(a0)` so the `[gating]` row that
blocks it is actually read, and §13 stops claiming in the present indicative that rows nothing reads
block rungs; and the reachability gate's class 1 now recognises the unbracketed `table.field`
spelling. That last one found a live defect on its first run: `[k1_outcome]`'s **table header was
missing**, so Rev 8's new R1 sentinel parsed under `[corpus]` and the citation did not resolve.

**Rev 8 was the BOUNDING revision, and it is the first one that did not claim to have fixed the ONE
gate.** Rev 7's adversarial review — six disjoint lenses, each required to write every rule as an
inequality with units, substitute degenerate cases, and re-derive every named red mutation
arithmetically, then an independent refutation pass over every finding — returned **35 findings, 34
surviving, 16 P0, 8 blocking**, and scored Rev 7's own four claims at **1 hold, 2 partial, 1 does
not hold**. Five consecutive revisions had each been told the same thing.

The finding that ended the repair loop is not a gate defect at all: **the ONE gate's left-hand side
has no measurand at this rung.** The decidability floor is a resolvable *delta*, the frozen file
named our side's denominator as the **armed paired delta**, and R0 lands no meshlet, no cluster and
no LOD — so there is nothing to arm and nothing to measure. Five of the eight blocking P0s were
downstream of that single over-reach: R0 was pre-registering and adjudicating a comparative
performance claim before the thing being claimed existed.

So Rev 8 removes the decidability apparatus from R0 rather than repairing it for an eighth time.
R0e, R0f and R0f′ leave §8; `[decidability]`, `[absolute_mode]`, `[scope]` and `[ordering]` leave
the frozen file; `[claim]` and `[quality]` leave the claim file. All of it — specification,
denominators, the two-sided gate form, and the eight P0s each with the arithmetic that refuted it —
is **§14**, to be frozen at the first rung that lands an arm, where the arm, the denominator and the
floor are defined together. What remains is an instrument-and-census rung that decides K1 in the
refute direction, records whether a reference is producible, and **states what it does not decide**
(§9.1).

One mechanism landed with it, because the defect family finally had a machine-checkable form.
Rev 7's thesis was that fixes must land in §8 and not only in the frozen file; the review showed
that to be a proper subset — Rev 7's own new field was orphaned *inside* the frozen file, and the
file's own staleness fields were stale for the third revision running. The accurate rule is one
level up: **a rule is landed only when some consumer reads the symbol it defines.**
[`tests/vg_symbol_reachability.rs`](../tests/vg_symbol_reachability.rs) enforces it over both frozen
files and this document — dangling citations, definitions no rule reads, and frozen fields the plan
never names. It reported **32 violations** when written. It reports **0** as of this revision, and
it is asserted for exact equality, so the count cannot fall silently either.

**Revision history, kept because the errors are the useful part.** Rev 1 carried one open P0 and
three defects of one family — *a gate that cannot go red for the failure it exists to catch*.
Rev 2 attacked all four and closed **one**. Rev 3 attacked them again: of ten things it claimed to
fix, **four held, two were partial, four did not**, and it introduced three fresh defects of the
same family — including a gate-widening hole in the clause it labelled non-negotiable. Rev 4 is the
result.

**The pattern is now the most reliable finding in this document, so it is stated rather than
buried:** three consecutive revisions each claimed to close the P0 and each failed differently —
Rev 1 left the right-hand side undefined, Rev 2 wrapped a sha256 around the string `PENDING`, Rev 3
ordered the artifacts in a way that constrains *commits* but not *knowledge*. **An author's account
of their own fix has been wrong more often than right here**, which is the entire argument for the
adversarial pass that produced every correction below.

| Rev 3 claim | Verdict at Rev 4 |
|---|---|
| P0 fixed by ordering (claim blocks R0e) | **DOES NOT HOLD** — see §0.1's retraction |
| D1 fixed by the ladder-convergence estimator | **DOES NOT HOLD** — `D_est` is capped at exactly 4.0 and is a *lower* bound firing a kill. §5.5 |
| D2 + `assert_achieved_extent` | **HOLDS** — the strongest thing in the document |
| R0a branch-specific field lists | **HOLDS** |
| Non-authored install search | **PARTIAL** — bounded authorities, not a volume walk |
| R0f′ gets its own absolute instrument | **PARTIAL** — the admission is right, gate (b) still compares a resolution to a level |
| Two-file split | **PARTIAL** — nothing forbids editing the claim *after* the fill |
| Claim scope named (`bracketed_vb_pass_chain`) | **HOLDS** — best reasoning in the plan |
| Denominators written down | **HOLDS** — verified against the sibling exactly |
| §12 anchors re-derived | **HELD FOR §12 ONLY** — the body kept the stale ones, including the `:334~`→`:344~` anchor §12 itself called the most expensive of the set. Fixed at Rev 4 |

**Rev 7 — the governing defect was not any single gate, it was WHERE the fixes landed.** An
adversarial review of Rev 6 scored its ten claims **2 hold, 7 partial, 1 does not**, and named the
cause: **six of the ten landed only in `VG-CAMPAIGN-THRESHOLDS.toml`, while §8 — the section an
implementer codes from — kept the superseded rule.** Rev 6 diagnoses exactly this at R0d
(*"rewriting the explanation is not rewriting the gate"*) and then commits it at R0f′, whose gate
text still carried the one-sided inequality **and the sentence the frozen file explicitly refutes**.

**The generalisation, and it is Rev 7's whole lesson: a fix that lands only in the frozen file has
not landed.** The frozen file is the authority for *values*; the rungs are the authority for *what
is asserted*. Editing one and not the other leaves two documents disagreeing about the decision
rule — Rev 2's inverted `all_three_below` defect reached by a different route.

Rev 7 also retires a promise this document could not keep. §12's *"every line below was opened or
grepped"* is **withdrawn**, having been false four revisions running. ⚠️ **The rest of this
paragraph was Rev 7 history left in the present indicative, and Rev 11's repair of the identical
claim in §12 did not reach it — the same fix landing in N−1 of N texts, one section out.** As
history: gating the plan's anchors mechanically was attempted and reverted, because the plan cited
bare basenames in prose while the gate binds to resolvable path links, giving 83 "stale" of 146
dominated by misbindings; converting the citations was then the named follow-up. **All of that is
done.** The conversion landed, the plan is in `GATED_DOCS`, and the live limit is stated once, in
§12, where it belongs: it is not membership but the waiver. This document still states its numbers
as unchecked rather than verified — that clause is *not* superseded, because a bounds-checked
anchor is not a verified claim.

**Rev 6 — the fifth consecutive revision told it overclaimed, and the score is the point.** Rev 5
claimed eight fixes: **two held, two partial, four did not**, plus five fresh defects. The three
that blocked approval were all this campaign own signature family.

* **R0f-prime gate (b) PASSED FOR ITS OWN NAMED RED MUTATION.** With m = measured median, c = claim,
  s = spread, Rev 5 gate s*m < |m - c| is **symmetric**: it passes for a target already beaten
  *and* for one absurdly far away, reding only in a narrow band around the status quo. Its stated
  mutation — *set the claim below the measured floor* — works out to s*m < m(1-s), i.e.
  s < 0.5, **true for every s <= 0.25 the ceiling permits.** Rev 6 gate is two-sided: the claim
  must be an **improvement** *and* a **resolvable** one, and both mutations were re-derived until
  they fire.
* **The P0 ordering rule named ONE rung for TWO instruments.** In absolute mode the floor is
  measured at R0f-prime, two rungs after R0e, with the claim already filled and visible — so
  blocking R0e constrained nothing on the branch section 11 calls expected. Now attached per mode.
* **K1 firing instrument could not be built where the plan put it, and would have been inert
  anyway.** A fragment shader cannot count frustum/backface survivors — they never reach it — and a
  survivor count includes every occluded triangle: ~2.5 M against ~2.07 M covered pixels, so it
  could never fire, exactly like submitted/covered before it. **R0 is now REFUTATION-ONLY for
  K1.** Naming a rung that cannot be built is worse than naming none.

Two more worth recording for *where* they sat: r0e_min_quads = 200 was presented as the sibling
SV0_BENCH_MIN_QUADS, which is **30** — 200 is SV0_S1_5_SESSION_QUADS, a **measured**
transcription. A measurement laundered as a pre-registered floor, frozen, **inside the section added
to fix authored-constants-called-measured.** And section 3.2 stale-comment warning was itself stale:
zero sites assert VB_IMPLEMENTED == false today, because 792d992 fixed all **19** this session.
A warning about stale documentation went stale the ordinary way — the world was fixed and the
warning was not re-derived.

**Rev 5 closed the earlier remainder** — the items Rev 4 acknowledged but did not land:

* **P0-5, gate (b) of R0f′ — a dimension error shipped twice.** `absolute_floor_source` is a
  *relative fraction*; Rev 4 compared it to a *millisecond target* and called it *"a genuine
  inequality between two quantities of the same kind"*. Read as a fraction it is the same error
  Rev 3 made; read as milliseconds it says `0.28 ms < 5 ms`, **true for any non-degenerate
  target** — a resolution being smaller than a level says nothing about seeing the gap. Rev 5's
  gate compares the floor to the **distance to be closed**, both in ms. This guarded the branch
  §11 measures as the *expected* one.
* **P1-3, K1's redundant conjuncts** — modal bucket > 16 px *implies* `D_est ≲ 0.06`, so conjunct 1
  never spoke. Retired; the rule is now split by direction (§5.6).
* **P1-4, two "pre-registered" thresholds with no file to be registered in** — R0c(c)'s oracle
  tolerance and R0e's CI bound. Both decision-bearing, both on *neither* side of the two-file
  split. R0e's named mutation could not fire against anything. Now `[pre_registered]`.
* **P1-5, the ordering rule lived only on the UNHASHED side** — the single most decision-bearing
  rule in the campaign, disarmable by deleting one line, while `[k1]` and `[scope]` were frozen.
  Now duplicated into `[ordering]` on the hashed side.
* **P1-6, R0a's "enumerate fixed volumes"** — a recursive walk of two ~240 GB volumes inside a
  `cargo test`, with false positives from any stray binary. Replaced by bounded authorities, with
  the residual blindness recorded rather than claimed away.
* **P1-7, R0c(e) vs R0d(a)** — as a gate, (e) made a *legitimate finding* red the rung and block the
  ladder. It now measures and records; R0d is where it becomes a gate, in whichever shape (e)
  established. And it is measured at the top rung on the corpus, where it can actually fail.
* **P1-9, three authored constants called "measured"** — laundering pre-registered protocol
  thresholds as evidence, in the sentence claiming rigour.
* **The hash's mirror-image failure.** Rev 4 avoided Rev 2's *guaranteed to break* and produced
  **guaranteed not to fire**: every rung meant to re-assert the thresholds hash is a skipped or
  `#[ignore]`d GPU/corpus test on a box whose CI never exercises the GPU path. `[hash_assertion]`
  now requires one plain `cargo test --workspace` assertion.

| # | Rev 1 defect | Rev 2 attempt | Rev 3 |
|---|---|---|---|
| **P0** | The ONE gate is `floor < intended_delta`; the **right-hand side was never defined and would be set by the author who measures the left.** | **FAILED.** The RHS shipped as the literal string `PENDING`, and Rev 2's own `[gating]` table scheduled it to be filled **after** R0e measures the floor. A sha256 wrapped around a placeholder — the identical defect, one indirection down. | **Fixed by ordering, not hashing.** The claim fields now block **R0e** (the rung that measures the floor), not R0f (the rung that compares). The number that could be tuned to fit the floor must exist *before* the floor does. §0.1. |
| **D1** | K1's statistic is **capped at 1 by construction** — `vb_id` is one `R32G32_UINT` texel per pixel, so `distinct pairs / covered pixels ≤ 1`. | **Diagnosis right, fix broken.** Two ways: the frozen rule string `all_three_below` **inverted** the third conjunct (a `_max` among two `_min`s), so K1 did not fire on the canonical no-mechanism scene; and the decisive conjunct `submitted/covered < 1.0` is precluded by R0b's own high-poly corpus gate — self-satisfied out of existence. | **Replaced with a ladder-convergence estimator** that is uncapped, tight, and self-validating, plus two conjuncts pointing the same way. §5.5. |
| **D2** | The census resolution was **anchored to nothing**. | **WORKED** — the one fix that survived review. | Kept, plus the missing extent assertion (an OS-clamped window fabricates the curve). §5.4, §8 R0c. |
| **D3** | R0 had **no gate on its most likely branch** — §11 measures no UE5 on this box. | **Half.** The re-derived negative is a real improvement, but R0a's field-list gate was unsatisfiable on that branch, the re-derivation searched an author-written path list, and **R0f′ assumed an absolute-time instrument that §7's paired-delta harness structurally cannot be** — its algebra exists to *cancel* the absolute terms. | Branch-specific field lists; a non-authored install search; and absolute mode gets its **own, honestly worse, measured** floor. §8 R0a, §8 R0f′. |

**Two structural changes Rev 3 makes as a consequence.**

* **The frozen file is split in two.** Rev 2 put author-frozen thresholds and owner-fillable VALUES
  calls in one hashed unit, so the recorded hash was *guaranteed* to break at the first legitimate
  `PENDING` fill — and once "re-record the hash" is routine, the tripwire carries no signal and can
  launder a simultaneous threshold edit. Now: [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml)
  is hashed and **never changes**; [`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) is
  **not hashed** and is gated by the `PENDING`-sentinel rule `goldens/PINS.toml:15~` already defines.
* **The claim's scope is named.** Rev 2 compared a **per-pass** floor to a **frame-total** claim
  with no composition rule stated anywhere, which made the ONE gate not evaluable. The claim is now
  explicitly about the **bracketed VB pass chain**, and the chain floor is measured directly on the
  chain rather than composed from per-pass floors (the passes share occupancy and a queue; they are
  not independent).

**Why R0 exists.** The research's headline result is a refutation: no measured Nanite cost map
exists in any source five survey lenses could reach, so *"faster than Nanite"* is not currently
falsifiable. R0 builds the instrument that makes it falsifiable — or proves it cannot be built,
which is equally valuable and vastly cheaper than discovering it in month six.

**This document states no measured number in prose — with one fenced exception, §11.** Every fact
that could drift is a named test, and the test name is the citation. Numbers that appear are
either *structural counts* (how many files include a header; how many `.spv` a change perturbs) or
explicit `MEASURE` placeholders a rung fills in **code**, under the standing discipline:
**"MEASURED — do not edit these literals to make a failing run pass."** §11 is a dated environment
record; **no gate reads it**, and any rung that depends on one of its facts re-derives it in its
own test. That rule exists because in the sibling VB-SV0 plan hand-copied numbers in prose caused
every revision to introduce defects at the lines it edited.

**Three corrections to the R0 paragraph in the research synthesis, all verified against the tree.**

| Research says | Verified |
|---|---|
| R0 has *"no render change whatsoever … byte-identical goldens"* | **Half true.** The density census cannot read the visibility buffer without widening the `vb_id` ring's image usage — [`targets.rs`](../crates/boyko_rhi_vulkan/src/present/targets.rs):868~ declares `COLOR_ATTACHMENT \| SAMPLED`, no `TRANSFER_SRC`. R0c therefore makes a **device-object** change. Frame content is unaffected and the byte-identity of all VB pins is that rung's gate, but "no render change whatsoever" is withdrawn. |
| *(orchestrator prescription)* `vb_geom_fetch.hlsli` *"is included by EIGHT shaders"* | **REFUTED.** `grep -rn 'include "vb_geom_fetch'` over `crates/boyko_rhi_vulkan/shaders/` returns exactly **four**: [`vb_geo.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_geo.comp.hlsl):118, [`vb_resolve.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl):85, [`vb_shade.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl):90, [`vb_shade_split.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_shade_split.comp.hlsl):137. The research doc's own corrected count (four includers, **eight** sources touching the *encoding*) is the right one — §2. |
| Research §4 item 1 includes *"plus the beginnings of a bake artifact format"* | **SCOPED OUT, on the record.** Rev 1 and Rev 2 dropped it silently while stating the other two corrections explicitly. A bake format is an output of the offline builder (research ladder R4/R5) and has no consumer at R0: nothing in R0 produces clusters, a DAG or simplified LODs, so a format authored now would be authored against no data. It returns with its first producer. The research doc's stronger point — *"There is no bake stage. This is the actual first blocker and no survey named it"* — stands and is why §3 exists. |

---

## 0. What R0 is, and what it decides

**R0 = a high-poly ingest path + a licence-clean corpus + a screen-space triangle-density census +
a recorded answer to "is a Nanite reference producible on this box".** No meshlet, no cluster, no
DAG, no shader that did not exist before.

**R0's claim, stated so that it can fail:**

> The census, run over a real high-poly corpus at a frozen resolution ladder, either **refutes K1**
> — proving screen-space density genuinely reaches ~1 triangle/pixel, so cluster LOD has a
> mechanism of action on our content — or leaves K1 **undecided**, which is an owner call with a
> `PENDING` field that blocks R1. R0 also records whether a Nanite reference is producible here at
> all, and fires K2 if it is not.

That is the whole of it. **R0 evaluates no comparative performance claim**, and Rev 8 is the
revision that stopped pretending otherwise.

### 0.1 Why the ONE gate is not here any more

Rev 1 through Rev 7 put an inequality at the centre of this rung — `joint_floor < claim`, the
campaign's decidability condition — and five consecutive adversarial reviews found it broken in a
new way each time. The scores are the record: Rev 2 claimed four fixes and **one** held; Rev 3
claimed ten and **four** held; Rev 5 claimed eight and **two**; Rev 6 claimed ten and **two**;
Rev 7 claimed four and **one**.

**The reason it kept breaking is that its left-hand side has no measurand at this rung.** The floor
is a *resolvable delta*. A delta needs two configurations to sit between, and the frozen file named
our side's denominator explicitly: the **armed paired delta**. R0 lands no meshlet, no cluster and
no LOD, so there is no arm — the quantity the whole apparatus was built to bound cannot be measured
until the thing being claimed exists. Five of the eight P0s that blocked Rev 7 were downstream of
that one over-reach.

So the apparatus moves to **§14**, to be frozen at the first rung that lands an arm, where the arm,
the denominator and the floor are all defined at once. It moves *complete*: the specification, the
two denominators, the two-sided gate form, and the eight P0s each with the arithmetic that refuted
it. §7's harness contract stays where it is and binds that rung.

**What the P0 was, and what actually answers it.** *"The delta we intend to claim"* is not a
measurement, it is a choice, and left unpinned it is a choice made after seeing the floor. Four
revisions tried to close that with a mechanism — a sha256 around a file whose every field was the
literal `PENDING`; then ordering, which constrains **commits, not knowledge** (run the harness
dirty, read the floor, `git checkout .`, fill to fit); then a per-mode ordering rule; then a pin.
What carries the weight is **party separation** — the owner answers the VALUES call and does not run
the harness — and a mechanism honestly described as partial is worth more than one described as
complete. Rev 8 adds the cheapest possible improvement to that: **do not pre-register a number
before the rung that measures its counterpart exists.** Pre-registration is not weakened by being
late here; it is strengthened, because at §14's rung the claim is written against a named measurand
instead of a placeholder.

### 0.2 The freeze, which R0 does keep

[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) carries the census ladder, K1's
instrument and decision rule, and R0c's pre-registered tolerance — all authored before any
measurement is reachable. R0a records its sha256, and the tripwire that re-asserts it is named by the frozen file itself:
`[hash_assertion].hash_tripwire_test` is `crates/boyko_render/tests/vg_thresholds_freeze.rs` and
`[hash_assertion].hash_tripwire_landed_by_rung` is R0a. Its only job is to re-hash the file — no
GPU, no `dxc`, no corpus — which is what `[hash_assertion].must_run_in_plain_workspace_test`
demands: a bare `cargo test` must execute it.

⚠️ **That siting is the fix for a measured failure, not a preference.** Rev 4 wired the hash
assertion into four rungs and every one of them was a skipped or `#[ignore]`d GPU/corpus test on a
box whose CI never exercises the GPU path — a tripwire guaranteed **not to fire**, the mirror image
of Rev 2's guaranteed **to break**. Rev 6 then added a flag saying the assertion must run in a plain
workspace test and gave it no rung and no file. And a standing hazard when checking any of this:
`cargo check --all-targets` at this repo root is vacuum-green on a virtual manifest, so "the test is
in the workspace" is not evidence it runs — R0a's gate must show it **executing**.

[`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) is deliberately **not** hashed: its fields are
required to change exactly once, and hashing a file whose schedule requires it to change makes
re-recording the hash routine — at which point the tripwire carries no signal and can launder a
threshold edit alongside a legitimate fill. It is gated instead by the `PENDING` sentinel discipline
`goldens/PINS.toml` already defines.

**The two kills R0 can adjudicate, each a falsifiable test rather than a worry:**

| # | Kill | Test | Disposition if it fires |
|---|---|---|---|
| **K1** | **No content, no mechanism.** The corpus never approaches ~1 triangle/pixel, so cluster LOD has no mechanism of action on our content. | **Refute-only at R0.** `D_est ≥ [k1].d_est_min` at the decision resolution **refutes** K1 outright (a lower bound proves density). **Firing is UNREACHABLE at R0** (`[k1].k1_fire_at_r0`): the upper-bound instrument is mis-sited and probably inert, and is recorded UNSOLVED rather than scheduled. §5.6, §9 clause 1. | Refuted → the mechanism exists, proceed to R1. Undecided → owner VALUES call, §13 Q2, which blocks R1. |
| **K2** | **No baseline.** The Nanite reference cannot be produced on this box. | R0a's rig probe, before any engine code — and the negative is **re-derived by the test**, not declared. §8 R0a. | **Scope restatement**, an owner VALUES call: the eventual goal becomes an absolute ms/quality target. R0 records the branch; §14's rung is where a target is set. |

**Falsification-first ordering.** K2 is the cheapest to test — *zero* engine code, one operator
session — so R0a runs first. K1 needs the corpus and the instrument, so it lands third and fourth.

⚠️ **K3 — the undecidable harness — is not an R0 criterion any more.** It moved to §14 with the
rungs that tested it. R0 builds no harness and measures no delta, so there is nothing here for K3 to
be true or false about.

---

## 1. Naming — decided, not open

`cluster` in this codebase means **light froxel** (`cluster_cull.hlsl`, `ClusterGrid`,
`MAX_LIGHTS_PER_CLUSTER`, the whole VB-P1e campaign). Geometry uses **`meshlet`** for the leaf and
**`geo_group`** for the DAG group. `cluster` stays with lights. This is a one-way door and it is
decided; no rung re-opens it.

---

## 2. The blast radius R0 does not touch — but must state

R0 changes no shader. It nonetheless has to state the encode blast radius, because that number
shapes every rung after it and because the ladder's R2b exists purely to pay it down.

**Verified this session (grep over `crates/boyko_rhi_vulkan/shaders/`):**

* `vb_geom_fetch.hlsli` is `#include`d by **four** sources (listed in the status block).
* `vb_pack.hlsli` — which declares `VB_ID_SENTINEL` (`:19`) — is `#include`d by **six**:
  [`vb_classify_count.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_classify_count.comp.hlsl):29, [`vb_classify_scatter.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_classify_scatter.comp.hlsl):24, [`vb_geo.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_geo.comp.hlsl):117,
  [`vb_resolve.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl):84, [`vb_shade.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl):89, [`vb_shade_split.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_shade_split.comp.hlsl):136.
* The **encode** side is two more sources: [`vb_raster.vs.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_raster.vs.hlsl):82 exports the flat `instance_id`
  interpolant (`:63`), and [`vb_raster.fs.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_raster.fs.hlsl):25 is literally
  `return uint2(input.instance_id, raw_prim_id);` with `raw_prim_id : SV_PrimitiveID` (`:24`).
* **Eight sources total touch the encoding.** They compile to **sixteen** committed `.spv`
  (`vb_raster.{vs,fs}`, `vb_geo{,_mv}`, `vb_classify_{count,scatter}`, `vb_resolve{,_froxel}`,
  `vb_shade{,_tex,_froxel,_tex_froxel}`, `vb_shade_split{,_tex,_hwrt,_tex_hwrt}`).
* **All sixteen now have a re-DXC byte-identity gate**, across two files whose row tables are exact
  complements: `vb_lit_producer_spv_sync.rs`'s `VB_LIT_PRODUCER_ROWS` (ten — `vb_resolve{,_froxel}`,
  `vb_shade{,_tex,_froxel,_tex_froxel}`, `vb_shade_split{,_tex,_hwrt,_tex_hwrt}`) and
  `vb_raster_geo_classify_spv_sync.rs`'s `VB_RASTER_GEO_CLASSIFY_ROWS` (the other six).
  ⚠️ **Rev 7 said the six would "drift silently". That was true when written and is not now:** the
  six-row gate landed at `598f4ff`, which is the byte-neutral rung the research document
  ([`MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`](MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md) §"Blast radius")
  prescribed *"before touching the encoding"*. That prerequisite is discharged; no rung of this plan
  needs to carry it.
  **The coverage is conditional and the condition is not nothing:** both files SKIP, by design, on a
  host where no `dxc` resolves, because a different `dxc` failing them would mean "wrong toolchain",
  not "drifted shader". So the sixteen are gated *on a host carrying the pinned VulkanSDK 1.4.350.0*,
  and a green CI run that skipped proves nothing about them. Any rung that re-encodes `vb_id` must
  show the gate executing, not merely passing.

**The decode side is genuinely one line** — [`vb_geom_fetch.hlsli`](../crates/boyko_rhi_vulkan/shaders/vb_geom_fetch.hlsli):521 is exactly
`uint local_tri = raw_prim_id % tri_count;`. The **encode** side is not independently reachable: the
G lane is filled by a fixed-function system value, so authoring a meshlet id into it requires a mesh
shader, one draw per meshlet, or a software rasterizer. **The re-encode is downstream of the
raster-path decision, not independent of it.** R0 records this and touches none of it.

---

## 3. Ingest — what exists, and what a high-poly importer must produce

### 3.1 What imports geometry today

**Exactly one mesh loader exists.** `MeshGpu::LOADERS` is a single-entry compile-time table
([`mesh.rs`](../crates/boyko_render/src/mesh.rs):238) holding `ObjMeshLoader`, whose `EXTENSIONS` is `&["obj"]` (`loaders/obj.rs:60~`). It
decodes to `MeshData { vertices: Vec<Vertex>, indices: Vec<u32> }` and runs `generate_tangents` once
over the whole mesh (`:94~-96`). **There is no `.obj` file anywhere in the tree** — the loader has
never been pointed at a committed asset.

### 3.2 The contract an importer must satisfy

The importer's *only* obligation is to produce a `MeshData`. Everything downstream already works:

| Seam | Contract | Anchor |
|---|---|---|
| `Vertex` | `#[repr(C)]`, **64 B** (static-asserted), `position`@0 / `normal`@12 / `color`@24 / `uv`@40 / `tangent`@48 | [`mesh.rs`](../crates/boyko_render/src/mesh.rs):81~-104 |
| Index width | `Uint16` iff unique-vertex count ≤ `U16_INDEX_VERTEX_LIMIT`, else `Uint32`; the shader reads the width from `gMeshMeta[].index_width` | [`mesh.rs`](../crates/boyko_render/src/mesh.rs):124, [`mesh_assets.rs`](../crates/boyko_render/src/mesh_assets.rs):273~ |
| Device upload | `build_mesh_gpu(ctx, &vertices, &indices, geometry_table)` | [`mesh_assets.rs`](../crates/boyko_render/src/mesh_assets.rs):252 |
| VB geometry slot | claimed **iff** a live table is threaded; otherwise the record carries `VB_GEOMETRY_RESERVED_SLOT` (`0`) | [`mesh.rs`](../crates/boyko_render/src/mesh.rs):170, [`mesh_geometry_table.rs`](../crates/boyko_render/src/mesh_geometry_table.rs):66 |
| `gMeshMeta[]` row | `{index_width, vertex_count, index_count}` padded to 16 B; `tri_count = index_count / 3` | [`mesh_geometry_table.rs`](../crates/boyko_render/src/mesh_geometry_table.rs):82-93, `:116-118` |
| Table capacity | `MESH_GEOMETRY_TABLE_CAPACITY = 4096` slots | [`geometry_bindless.rs`](../crates/boyko_rhi_vulkan/src/geometry_bindless.rs):62 |

**The streamed path already threads the table.** `impl GpuUpload for MeshGpu` sets
`type Aux = MeshGeometryTableSlot` and calls `build_mesh_gpu(ctx, &cpu.vertices, &cpu.indices,
aux.0.as_mut())` ([`gpu_upload.rs`](../crates/boyko_render/src/gpu_upload.rs):51, `:59`). So a **loader-decoded** mesh claims a real slot and is
VB-visible. The **host-authored** primitives pass `None` at their own call site
([`mesh_assets.rs`](../crates/boyko_render/src/mesh_assets.rs):547~), and the explicit VB sibling is `MeshAssetsVbExt::register_mesh_vb`
([`mesh_assets.rs`](../crates/boyko_render/src/mesh_assets.rs):641, `:647`), which every VB fixture uses.

> ⚠️ **CORRECTED at Rev 4 — Rev 1 through Rev 3 all stopped one function too early, and the error
> propagated into R0b's headline red mutation (§8).** Passing `None` is **not** the end of the
> story: `backfill_vb_geometry_slots` runs at boot ([`runner.rs`](../crates/boyko_app/src/runner.rs):787~, after `upload_mesh_assets` and
> after `finish()`) and claims a slot for **every** still-reserved mesh under a VB boot — by design,
> so that *any* scene's meshes are re-fetchable by `vb_resolve` rather than only the ones an author
> remembered to route through `register_mesh_vb`. **A host-authored mesh registered during startup
> IS VB-visible.** The real hole is narrower and is the one R0b must target: the back-fill is a
> **boot one-shot**, so a mesh registered *after* boot keeps `VB_GEOMETRY_RESERVED_SLOT` with
> nothing to rescue it. No scene does that today, which is exactly why nothing catches it.
>
> This was found by an implementer refuting a premise I had written into its brief — the eleventh
> such refutation this campaign. It is also why the ⚠️ block below is dangerous in a *second* way:
> the stale comments do not merely under-describe the arming, they describe a `None` path whose
> consequences the code no longer has.

> ⚠️ **WITHDRAWN at Rev 6 — the trap this block described has been FIXED, and the block outlived
> the fact.** Rev 1–Rev 5 warned that *"at least six doc comments still assert it is `false`"* and
> enumerated nine anchors. **Zero of them do today:** `grep -rn VB_IMPLEMENTED crates/ --include=*.rs
> | grep -ci false` returns **0**, and `const VB_IMPLEMENTED: bool = true;` is at
> [`render_path_config.rs`](../crates/boyko_render/src/render_path_config.rs):130~. The rot was cleared at `792d992`, which found **19** stale sites
> across 12 files — not the six or nine this block claimed — and rewrote every one.
>
> **The block is kept, struck through, rather than deleted, because it is a specimen.** A warning
> about stale documentation went stale itself, and it did so in the ordinary way: the world was
> fixed and the warning was not re-derived. That is the same mechanism the warning was about, one
> level up, and it is why §12's blanket *"every line was verified"* claim keeps turning out false.
> **A document that describes a hazard must be re-checked when the hazard is addressed — the fix
> and the warning are not automatically committed together.**
>
> What survives, and what R0b's second mutation is now sourced from, is the *narrower* property
> §3.2 states above: `backfill_vb_geometry_slots` is a **boot one-shot**, so a mesh registered
> **after** boot keeps `VB_GEOMETRY_RESERVED_SLOT`. That is real, verified, and produces a mutation
> that actually reds.

### 3.3 The format decision — decided here, with reasons, not escalated

**Decision: glTF 2.0 binary (`.glb`), in-house decoder, deliberately narrow subset.**

* **Why not extend OBJ.** Licence-clean high-poly corpora ship as `.glb`/`.gltf`. OBJ carries no
  tangents, no index buffer (the loader sort-dedups every corner — `loaders/obj.rs:39`), and is a
  text parse over hundreds of megabytes.
* **Why in-house.** A `.glb` is a 12-byte header + a JSON chunk + a BIN chunk; only the JSON chunk
  needs a new reader. That is loader code, not hot-path code, and the same class of work
  `boyko_image`'s in-house PNG/zlib/DEFLATE already carries. §13 Q2 asks the owner only the
  **dependency-policy** half, which is a VALUES call; the format itself is decided.
* **The subset, stated as a scope cut rather than discovered as a bug.** Supported:
  `mode == TRIANGLES`, `POSITION`, `NORMAL`, `TEXCOORD_0`, `TANGENT`, `COLOR_0`, and indexed
  primitives with `u16`/`u32` indices. **Unsupported and a hard decode error, never a silent
  fallback:** sparse accessors, Draco/meshopt compression, animation, skins, morph targets,
  non-triangle modes, and non-indexed primitives. A missing `TANGENT` runs the existing
  `generate_tangents` post-pass; a missing `COLOR_0` takes `loaders/obj.rs:13`'s neutral default.
  Refusing loudly is the point: a partial mesh silently accepted is a census that measures a
  different scene than the reference capture does.

### 3.4 The residency hazard, named because nothing else names it

`build_mesh_gpu` creates **both** buffers as `MemoryLocation::HostVisibleCoherent`
([`mesh_assets.rs`](../crates/boyko_render/src/mesh_assets.rs):320~ for the vertex buffer; the index buffer follows). Every mesh in this
engine lives in host-visible memory, seeded once and read-only thereafter ([`mesh.rs`](../crates/boyko_render/src/mesh.rs):129~). At 64 B
per vertex a multi-million-triangle corpus mesh is a large host-visible allocation, and on a
discrete GPU without resizable BAR that heap is small. **R0b's gate includes "the corpus's largest
mesh registers without allocation failure"**; the abort route is a device-local + staging upload
path for meshes, which does not exist today and is a named follow-up, not R0 work.

---

## 4. Corpus — the decision, and the constraint that forces it

### 4.1 The convention that cannot be followed

`crates/boyko_app/assets/pbr_fixtures/README.md:1-6` documents the existing convention: *"Tracked,
in-repo ground-truth oracle texture sets — small … unlike `assets/materials/`, which is
gitignored."* `.gitignore` carries the counterpart rule (`/assets/materials/*` with a
`!/assets/materials/README.md` escape). There is **no `.gitattributes`**, so **Git LFS is not
configured**, and git history is immutable — a corpus committed once is carried forever by every
clone. §11 records the measured sizes that make this decisive.

### 4.2 Three candidates, and the decision

| Candidate | Verdict |
|---|---|
| **Tracked and small** | **Rejected.** A high-poly corpus is not small by any definition that keeps this repo cloneable, and there is no LFS seam to hide it behind. |
| **Generated procedurally at test time** | **Rejected as the corpus — adopted as the instrument's self-test.** A procedural generator has a density knob, so a density census run against it can always be cranked past ~1 triangle/pixel. That makes **K1 unfalsifiable by construction** — a gate that cannot go red for the failure it exists to catch, which is this campaign's single most-repeated defect. It is however the ideal *sensitivity control* for the census instrument (§8 R0c), where an analytically-known screen-space triangle size is exactly what is wanted. |
| **Fetched, gitignored, pinned by content hash** | **CHOSEN.** |

### 4.3 The chosen shape

* A committed, human-readable manifest `assets/vg_corpus/CORPUS.toml` — per asset: source URL,
  **licence identifier and licence URL**, sha256 of the archive, sha256 of each extracted `.glb`,
  triangle count as published, and the camera-path id it is censused under. The manifest is
  **tracked**; the payload is **gitignored** by a `/assets/vg_corpus/*` + `!CORPUS.toml` +
  `!README.md` rule mirroring the `assets/materials/` precedent exactly.
* **Licence-clean means recorded, not assumed.** The repo carries no `LICENSE` file of its own, so
  the corpus manifest is the only place a licence claim can live. An asset whose licence permits
  redistribution but not the *reference capture* (e.g. loading it into a third-party engine) is
  unusable for this campaign and must be rejected at manifest-authoring time, not at the rung that
  eventually compares two engines.
* **The same bytes feed both engines.** The Nanite reference (§6) imports the identical `.glb`
  files. If an asset cannot be imported by both, it is not corpus material.
* A `fetch_corpus` script verifies every pinned hash before extraction and refuses on mismatch. The
  **gate that reads it is a Rust test**, not the script — §8 R0b.

---

## 5. The density census — what exists, what must be added

### 5.1 Counters that exist today

* **Submitted triangles, host side.** `DrawBatch { mesh_id, index_count, index_type, base_instance,
  instance_count }` ([`mesh_draw.rs`](../crates/boyko_render/src/mesh_draw.rs):80-98) is gathered per frame; `index_count / 3 *
  instance_count` is the submitted-triangle count with no new plumbing.
* **Per-pass GPU time, partially.** `VbTimedPass` ([`gpu_timing.rs`](../crates/boyko_rhi_vulkan/src/present/gpu_timing.rs):203) brackets **three** passes:
  `CullReset` (`:211~`), `CullDispatch` (`:214~`), `VbShade` (`:229~`); `VB_PASS_COUNT = 3` (`:242`).
  **The VB raster pass, the `vb_geo` pass and the classify chain are NOT bracketed.** A per-pass
  table comparable to a Nanite capture therefore requires extending this enum — §14's rung, not R0.
* **A CPU coverage rasterizer.** `crates/boyko_app/tests/sv0_oracle/mod.rs` ships `rasterize`
  (`:279~`) producing a `Coverage` (`:211~`) of `CoveredPixel` (`:193~`) with `covered_count`
  (`:253`), plus `changed_covered_pixels` (`:798`). It is perspective-correct and supports
  translation-only instances.

### 5.2 Counters that do not exist

Nothing anywhere produces a **screen-space triangle-size histogram** or a **triangles-per-pixel**
statistic, and nothing reads the visibility buffer back to the host. `vb_id` is created with
`usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED` ([`targets.rs`](../crates/boyko_rhi_vulkan/src/present/targets.rs):868~) — **no
`TRANSFER_SRC`** — and [`frame_driver.rs`](../crates/boyko_rhi_vulkan/src/present/frame_driver.rs):750~ records that the engine deliberately has *"NO
`copy_image_to_buffer(depth)`"*; the only host readback path is the swapchain
(`host_dump.rs`, `BOYKO_HOST_DUMP`).

### 5.3 The instrument — decided with structure, not escalated

| Option | Cost | Verdict |
|---|---|---|
| (a) Widen `vb_id` usage with `TRANSFER_SRC`; `copy_image_to_buffer` on census frames; histogram on the host | +1 usage bit, +1 recorded copy on armed frames only, **zero** new `.spv`, **zero** manifest rows | **CHOSEN** |
| (b) A compute pass that histograms `vb_id` into an SSBO | a new `.spv`, a new `SHADER-VARIANT-MANIFEST.md` row, a new binding, a new barrier | Rejected — buys nothing (a) does not, and enlarges the very blast radius R0 exists to keep at zero |
| (c) Reuse the CPU rasterizer alone | zero engine change | Rejected **as the census** — it is a host mirror of the raster, not the shipped VB path, and the whole point of the census is to measure what the engine actually produces. Retained as R0c's cross-check |

`copy_image_to_buffer` already exists in the RHI (`boyko_rhi/src/encoder.rs:115`; impl at
`rhi_impl/encoder.rs:1031`). The readback is `[census].readback_retention` — streamed and hashed,
never retained: at 3840×2160 × 8 B that is 66.4 MB per censused frame, and §11 records this volume
at 16 GB free with `target/` at 58 GB, so retention would reproduce this project's standing hazard
of disk exhaustion surfacing as mingw linker errors.
The census is armed by an env knob and threaded as an `Option`, so an
unarmed frame records **zero** extra commands — the exact discipline
`Option<&VbTimestampCollector>` documents ([`gpu_timing.rs`](../crates/boyko_rhi_vulkan/src/present/gpu_timing.rs):247~) and the reason the golden
command stream stays byte-identical.

### 5.4 The statistic — a bracket, because the obvious one is capped at 1

**Rev 1's defect, stated plainly.** `vb_id` is an `R32G32_UINT` image ([`targets.rs`](../crates/boyko_rhi_vulkan/src/present/targets.rs):866~) — **one
`(instance_id, raw_prim_id)` pair per pixel**. So `distinct (instance_id, local_tri) pairs ÷
covered pixels` is **≤ 1 by construction**, saturating exactly when every covered pixel carries its
own triangle. It cannot distinguish *"we have just reached one triangle per pixel"* from *"we are
ten times past it"* — which is the entire regime the campaign exists to serve. A K1 phrased as
*"never approaches ~1"* against a statistic that **can never exceed 1** is not a threshold, it is a
ceiling being mistaken for a reading.

Per censused frame, from the readback pairs, `local_tri = raw_prim_id % tri_count` reproducing
[`vb_geom_fetch.hlsli`](../crates/boyko_rhi_vulkan/shaders/vb_geom_fetch.hlsli):521 on the host:

1. **`visible_tri_per_covered_pixel`** = distinct `(instance_id, local_tri)` ÷ covered pixels.
   In `(0, 1]`. **Saturating** — it *understates*, and by exactly the amount that matters most.
2. **`submitted_per_covered_pixel`** = §5.1's `index_count / 3 * instance_count` summed over
   `DrawBatch` ÷ covered pixels. **Unbounded** — it *overstates*, because submitted triangles
   include back-face-culled and off-screen ones.
3. **Screen-space triangle-size histogram** = covered pixels per distinct
   `(instance_id, local_tri)`, bucketed by powers of two, reported as a distribution — **not** a
   mean. Sub-pixel triangles never appear in it (they lose the coverage race), which is the same
   blindness as (1) and the reason (2) is carried alongside.

**Rev 2 made (2) K1's decisive conjunct, and that was wrong — the useful kind of wrong.** It is a
valid upper bound: submitted triangles are a superset of visible ones, so `submitted/covered ≥
visible/covered` always. It is also **so loose as to be inert.** It counts back-face-culled and
off-screen geometry, so it conflates *"the triangles are small"* with *"the level contains a lot of
geometry."* Firing K1 required `submitted/covered < 1.0` — the whole frame submitting fewer
triangles than the screen has covered pixels, at most ~2.07 M at 1080p — while **R0b's own gate
(b) requires each corpus mesh to match a published *high-poly* count.** A corpus that satisfies R0b
can never satisfy K1's decisive conjunct. The kill was self-satisfied out of existence, and the
demonstration is concrete: take a scene whose visible triangles are all 20+ px (a close-up of a few
large-triangle props), then place nine more copies of each asset *behind the camera*. Density is
unambiguously in the "no mechanism" regime; `submitted/covered` is ten times larger; K1 stays
silent.

### 5.5 The decisive statistic — the ladder, which costs nothing new

**The ladder frozen for D2 turns out to be the instrument D1 needed.** For a fixed camera and fixed
geometry, a triangle's screen-space area scales *exactly* with pixel count: a triangle covering 4 px
at 2160p covers 1 px at 1080p. Therefore

* `visible_tris(R)` — distinct `(instance_id, local_tri)` in the readback at resolution `R` — is
  **monotonically increasing** in `R`, because raising resolution lets smaller triangles win
  coverage races they previously lost;
* it **converges**, as `R` grows, to the true count of front-facing, unoccluded triangles in view;
* so measuring at the **top** rung reveals precisely the sub-pixel population the decision
  resolution hides — which is the population this campaign exists to serve.

The density estimate at the decision resolution is then

> **`D_est = visible_tris(top rung) ÷ covered_pixels(decision_resolution)`**

**Indexed by camera path, and aggregated exactly once.** The census runs this per committed camera
path (§5.7), so the quantity above is `D_est(p)` — one reading per path — while K1 is **one**
decision. **Throughout this document the unqualified symbol `D_est` means the aggregate
`min` over committed camera paths**, per `[k1].k1_path_aggregation`; §2's blast-radius row, §5.6's
split table, §9 clause 1 and §9's outcome table inherit that definition and deliberately do not
restate it. MIN because refutation is the campaign-**favourable** outcome and must therefore clear
the bar on the weakest committed path rather than the strongest.

⚠️ **MIN's monotonicity cuts both ways and Rev 11 recorded only the favourable half.** From
`min(S′) ≥ min(S)` for `S′ ⊆ S`: MIN closes the *add-a-flattering-path* lever — that is true and is
why MIN rather than MAX — but it opens the *omit-an-unflattering-path* lever, which is cheaper
still, because an uncommitted path leaves no diff and produces no census row for §9.1's
anti-cherry-pick argument to catch. Rev 11's frozen comment called MIN removal of "the cheapest
remaining tuning lever"; it is not, and the superlative is withdrawn. What makes MIN sound is not
the reduction but **the domain**: the committed set is pinned at R0b and asserted at R0d(d)
(`[k1].committed_paths_rule`), so paths cannot be dropped after their readings are known.

— **tight**, unlike (2). ⚠️ **Rev 3 called it "unbounded above" and that was false in two ways at
once. Both were caught by arithmetic on this page, and both matter to the kill's soundness.**

**Its ceiling is exactly 4.0.** `visible_tris(R)` counts distinct pairs in a readback holding one
pair per texel, so `visible_tris(2160p) ≤ 3840·2160`. At a common covered fraction φ,

```
D_est ≤ (3840·2160) / (1920·1080) = 4.0     exactly
```

This is Rev 1's construction defect with the ceiling raised from 1 to 4 by the ladder's own area
ratio — *"it cannot distinguish 'we have just reached one triangle per pixel' from 'we are ten times
past it'"*, which is §5.4's indictment of statistic (1), applying verbatim. **The estimator's
dynamic range is a consequence of a ladder frozen for an unrelated reason (D2), and nothing in
Rev 3 recorded that.** `[k1].d_est_min = 1.0` therefore sits at **one quarter of the instrument's
ceiling**, which is where a threshold should sit — but by accident, not by design.

**It is a LOWER bound, and Rev 3 fired a kill on it.** Sub-pixel triangles that win no sample are
absent from `visible_tris`, so `D_est ≤` true density. Rev 2's `submitted/covered` was an **upper**
bound: sound for firing `< 1.0`, but inert. **Rev 3 traded soundness for tightness and did not
notice the trade.** `K1 fires iff D_est < 1.0` refutes the campaign on evidence that cannot
support refutation — the true density may be arbitrarily higher.

### 5.6 The asymmetry this forces — and why it is good news, not a dead end

A lower bound cannot refute the premise. It can **prove** it:

> **`D_est ≥ 1.0` at the decision resolution proves density is genuinely ≥ 1 triangle/pixel.**
> K1 is **dead**, the mechanism exists, and no further instrument is needed.

So the census as designed — usage bit, `Option`-threaded copy, zero new `.spv`, zero manifest rows —
can **close the question in the favourable direction at its current cost**. That is the cheap
outcome and it is the one worth attempting first.

**Refuting the premise costs more, and the plan must say so rather than pretend otherwise.** Any
statistic derived from `vb_id` is capped by one-winner-per-texel, so a sound upper bound **must**
come from outside it. The tight one available is a **counter of triangles surviving frustum +
backface**, incremented in the raster path under the census arm. That is not free: it edits
`vb_raster.fs.hlsl`, which moves the very blast radius §5.3 chose option (a) to keep at zero
(16 `.spv`, all sixteen now byte-gated — §2). ⚠️ The gate does not make the edit cheaper; it makes
the cost **visible**. Editing `vb_raster.fs.hlsl` now reds
`vb_raster_geo_classify_six_rows_reproduce_under_frozen_recipe` until the `.spv` is re-emitted and
committed, which is the intended behaviour and is a re-bless step this branch must budget for
rather than discover.

**The ladder therefore splits, and the expensive half is conditional:**

| Outcome of the cheap census | Next |
|---|---|
| `D_est ≥ 1.0` **and non-degeneracy met** | **K1 dead.** No counter, no shader edit, no re-bless. Done. ⚠️ The conjunct is not decoration and Rev 8 omitted it here: on a 500-pixel frame with 600 visible triangles `D_est = 1.2`, so without it this row declares the campaign's premise proven from a frame covering 0.02% of the screen. `[k1].k1_decision_rule` carries the conjunct as of Rev 9; this table and §9's outcome table must not diverge from it again. |
| `D_est < 1.0`, **or** non-degeneracy unmet | Genuinely sparse, instrument-limited, or not adjudicable at all — indistinguishable from below. **K1 UNDECIDED**. Owner VALUES call, §13 Q2, held by `k1_outcome.undecided_disposition`, which blocks R1. The counter is NOT a scheduled rung: §8 contains none, and its design is recorded UNSOLVED. |
| Ladder not converged | `[k1_instrument].on_not_converged_fire_direction` — **K1 not adjudicated** for the FIRE direction, §9 clause 3. The REFUTE direction is unaffected: non-convergence means `D_est` understates, and an understatement already ≥ 1.0 still proves density ≥ 1 (`on_not_converged_refute_direction = "still_valid"`). |

**This front-loads the cheap decisive case and makes the expensive one explicitly optional**, which
is what Rev 3's single-path design hid.

### 5.7 The scaling law — the frozen ladder does not satisfy it

Rev 3's R0d gate (c) asserted the histogram's modal bucket moves by **exactly two buckets** between
adjacent rungs. Two buckets is 4× area, i.e. **2× linear**. The frozen ladder contains no such
step:

| pair | area ratio | buckets |
|---|---|---|
| 512² → 1080p | 7.910 | **2.98** |
| 1080p → 1440p | 1.778 | **0.83** |
| 1440p → 2160p | 2.250 | **1.17** |

**The gate was red by construction** — the mirror of the family this campaign hunts: a gate that
cannot go *green*. Modal-bucket indices are integers over powers of two, so 0.83 and 1.17 are not
even expressible as a shift. Rev 4 replaces the constant with the **per-pair `log2` of the actual
area ratio**. ⚠️ **Rev 9: that replacement is REPORTED, not checked** — §8 R0d demotes it, because
a tolerance of 0.35 around targets of 0.830075 and 1.169925 admits exactly one integer on each pair,
so an integer was still being asserted, and both splits a correct instrument can produce satisfy the
scaling law while only one passes. Rev 4 replaced the constant and reports the residual rather than
asserting an
integer.

**And rung 1 is excluded from the scaling check entirely.** 512² is **1:1** while the other three
are **16:9**, so the projection is a *different frustum*, not a rescaling — the visible triangle set
differs, and §5.5's premise (*"a triangle's screen-space area scales exactly with pixel count"*)
does not hold across 512²→1080p at all. Rung 1 keeps its one job: the CPU-oracle cross-check.

**Three further limits on the scaling law, stated because R0c must design around them:** sample
lattices between rungs are **not nested** (a sliver holding one coarse centre and no fine centre
*disappears* at higher resolution, so `visible_tris(R)` is not strictly monotone); depth-test
tie-breaking can flip a triangle from visible to invisible as resolution rises; and covered-pixel
count is area **+ O(perimeter)**, so the shift is asymptotic for large triangles and wrong exactly
in the micro-polygon regime the census is about.

**Non-degeneracy precondition, absent in Rev 3 and required.** On a sentinel-only readback
`visible_tris = 0` at both top rungs, the convergence check reads `0 ≤ 0` → *converged*, and
`D_est = 0 < 1.0` → conjunct 1 holds. K1 fires on an empty frame. R0c/R0d therefore assert a
minimum non-sentinel covered-pixel count and a non-zero `visible_tris` before any of this is
evaluated, and `covered_pixels == 0` is an explicit failure, not a division.

**Resolution — D2's fix, kept.** The census runs `[census].resolution_ladder` and reports a
**curve**. K1 is adjudicated at `[census].decision_resolution` = 1080p **alone**, frozen: 2160p
would flatter the campaign, 512² would refute it unfairly. **512²'s real justification is narrower
than Rev 2 claimed** — it is the extent every VB fixture and golden pin already uses
(`sv0_scene/mod.rs:162`), which is what makes R0c's *procedural-fixture* cross-check possible.
It does **not** make a corpus cross-check possible: `sv0_oracle::rasterize`
(`sv0_oracle/mod.rs:279-287`) takes **one** indexed mesh and `instances: &[[f32; 3]]` — pure
translations — so it cannot rasterize a multi-asset corpus or place a rotated instance at any
resolution. R0c gate (c) is therefore scoped to the fixture, explicitly.

**The extent must be asserted, not assumed.** This engine's render extent is a real OS window
client area ([`window.rs`](../crates/boyko_rhi_vulkan/src/window.rs):252, `AdjustWindowRectEx` at `:310~`), and OS clamping is *already* a
recorded hazard here at 512² — [`sv0_deferred_term_bench.rs`](../crates/boyko_app/tests/sv0_deferred_term_bench.rs):297~-299 checks it, because *"an
OS-clamped window would silently measure a different per-pixel workload."* A display that clamps
1440p and 2160p produces three plausible rows and a **fabricated curve**, and every conclusion
above rests on the scaling law those rows are supposed to demonstrate. `[census]
.assert_achieved_extent` makes the readback's own dimensions the check.

**No error target is needed, and this is why** (`[k1].measured_at` freezes it as
`"full_detail_no_lod"`). The census renders at **full detail** — this engine
has no LOD, so there is nothing to hold an error target against. That makes the censused density
the **ceiling** of the mechanism available to any LOD scheme: a cluster hierarchy can only reduce
triangles below it. If the ceiling does not reach the regime, no LOD scheme reaches it either.
K1 is therefore decidable today, without the error target Rev 1's phrasing implied it needed.

All statistics are reported per camera path, path definitions committed as test constants — the
shape `sv0_scene/mod.rs:149~-162` already uses for its camera.

---

## 6. The Nanite reference — stated plainly, including what it demands

### 6.1 What the reference must contain

UE5, **our** GPU, **our** resolution, **our** corpus, `r.Nanite.MaxPixelsPerEdge` **pinned** (its
default and its aggressive setting change rendered triangle count by roughly an order of magnitude,
so an unpinned comparison is not a comparison), per-pass milliseconds recorded with the pass names
documented. **The multi-view constraint is a fairness requirement, not a footnote:** a lit Nanite
frame runs cull+raster once per view, and any table that reports only the primary VisBuffer is
comparing one of our passes against a fraction of theirs.

### 6.2 Whether it is achievable on this box — measured, not assumed

**It is not achievable today, and the reason is concrete.** §11 records the probe: there is **no
UE5 installation on this machine** (the only Epic-shaped directory is empty), together with the
measured free space on both volumes. ⚠️ Rev 8: R0 no longer captures the reference, so the three
prerequisites below are no longer a prerequisite *to a rung of R0* — they are what R0a RECORDS the
availability of, and what §14's rung would need. The operator must supply, before any capture:

1. a UE5 install of a named version, with disk headroom for the editor **plus** a project **plus**
   its derived-data cache — and this project's standing hazard is that the Rust `target/` directory
   alone has filled this disk to zero and masked itself as linker errors;
2. a project that imports the §4 corpus with Nanite enabled;
3. a capture protocol — `stat GPU`, Unreal Insights, or RenderDoc — producing per-pass timings, with
   the same clock-pinning discipline §7 imposes on our own harness.

⚠️ **Four causes, not three, and the frozen file is the authority on the count.** Disk headroom is
listed above as a qualifier on prerequisite 1, but it fails independently of it — an engine can be
installed and registered while the volume cannot hold a project plus its derived-data cache, which
is exactly what §11 measures. `[k2_probe].reason_values` therefore enumerates it separately, and
R0a's gate binds `reason` to that four-value set. Where this section says "three" it is counting
prerequisites; where the frozen file says four it is counting causes, and the gate follows the
frozen file.

**If any of them cannot be supplied, K2 fires**, and the disposition is not "measure something
else": it is a **scope restatement** the owner makes consciously (§13 Q1). The whole falsifiability
argument for this campaign rests on this rung, which is why it runs first and why R0a's gate is
mechanical rather than a paragraph.

### 6.3 The reference's own floor — moved to §14 at Rev 8

A capture is an instrument too, and **a claim smaller than the reference's own reproducibility is
unfalsifiable no matter how good our side is** — Rev 1's joint floor named a pair of instruments
while defining only one. That term, its summation rule and the reason summation beats quadrature
(a systematic capture bias between two engines is not an independent random draw) are **§14.2's**,
because they are terms of an inequality R0 no longer evaluates.

⚠️ One correction travels with them and must not be lost, because Rev 7 shipped it: the reference
floor was derived from the peak-to-peak spread of **per-pass** medians and then used as a **chain**
floor — the exact composition the scope rule forbade one table over, inside the same inequality
whose other half obeyed it. §14.4 P0-3 carries the counterexample. Whoever re-authors this
discharges it there.

**What R0 keeps of §6 is §6.1 and §6.2 only:** what a reference must contain, and whether it can be
produced on this box. Recording that answer is R0a's job (§8 R0a) and firing K2 on it is §9
clause 2's. Capturing the reference is not an R0 rung.

---

## 7. The decidability statement — the harness contract

**This is not optional and it is not generic.** The sibling rung
`crates/boyko_app/tests/sv0_deferred_term_bench.rs` MEASURED, on this exact hardware, two failures
that R0's harness must be built to avoid:

* **A null control that read a third of the signal.** Strict `A,B,A,B` interleaving aliased the A/B
  phase with the frame-in-flight slot, because `FRAMES_IN_FLIGHT == 2`
  (`crates/boyko_render/src/ui/mod.rs:87`). Each phase therefore always landed on the same query
  pool, descriptor ring slot and staging region. The fix is a counterbalanced **ABBA quadruple**
  whose statistic is `(d1 + d2)/2` and whose *residual* `(d1 − d2)/2` is **printed, not hidden**
  ([`sv0_deferred_term_bench.rs`](../crates/boyko_app/tests/sv0_deferred_term_bench.rs):53~-77).
* **A spread gate measuring its own resolution.** The timestamp counter's *step* is not the
  `timestampPeriod` the device reports; the harness had to recover it as the **GCD of raw tick
  counts** over a whole session (`:83~-100`). A "cross-session spread" that is one lattice step
  carries no information.

**R0's harness MUST therefore, non-negotiably:**

1. counterbalance (ABBA), and **report** the order-bias residual with its own band;
2. carry a **null control** — two identical configurations — with a **pre-registered** maximum, as
   `SV0_NULL_CONTROL_MAX_FRACTION` (`:378`) does, fixed before the run and never widened;
3. **measure** the counter quantum by tick GCD and report it alongside `timestampPeriod`
   (`:94~-96` the RESOLUTION field list, `:448~`/`:463~` the transcribed bounds, `:751~-772` the consistency check);
4. state the **resolvable delta with confidence intervals**, and make the effective spread gate
   `max(stated gate, measured median lattice / |median|)` — **but only where the lattice term is
   licensed by evidence.** ⚠️ Rev 3 transferred the `max()` and dropped the guard, which turned a
   non-negotiable clause into a gate a homogeneous sample could widen to rescue a failing run —
   this campaign's own #1 named defect, introduced in the clause written against it. The sibling
   does **not** grant the widening by default: [`sv0_deferred_term_bench.rs`](../crates/boyko_app/tests/sv0_deferred_term_bench.rs):805~-807 reads
   `if may_widen { SV0_SESSION_SPREAD_MAX.max(lattice_floor) } else { SV0_SESSION_SPREAD_MAX }`,
   where `may_widen` requires at least `SV0_LATTICE_MIN_DISTINCT_TICKS = 7` (`:399`) distinct
   observed tick values (`:680-681`), *"licensed by EVIDENCE … rather than granted by default"*
   (`:798~`). A **separate, non-waivable** test asserts `lattice_floor <= SV0_SESSION_SPREAD_MAX`
   unconditionally, *"so it can never silently widen the gate"*. §14's rung lands **all three** — the
   `max()`, the distinct-tick evidence floor, and the non-waivable assertion — or none of them.
   This is R16 (*a literal transferred without its denominator*) one level up: **a gate transferred
   without its precondition**;
5. discard warmup, run ≥3 separate processes, and pin every session's transcribed number as a test
   literal under the MEASURED discipline.

**One trap §14's implementer will otherwise hit.** Every `read_query_pool_ns` reader requests all
of its collector's `(begin,end)` pairs with `VK_QUERY_RESULT_WAIT_BIT`, which **blocks forever** on a
pair its recorder never wrote that frame — [`gpu_timing.rs`](../crates/boyko_rhi_vulkan/src/present/gpu_timing.rs):344~ states this, and it is why three
separate collectors exist rather than one widened `PASS_COUNT`. Extending `VbTimedPass` to cover
raster/geo/classify means **every added pair must be written unconditionally on every armed frame**.
That rung therefore also lands a **written-pair bitmask asserted before the read**, so a conditional
bracket fails as a red assertion instead of hanging the test binary — a hang is not a gate.

---

## 8. Rungs

Ladder: **kill the baseline cheapest → land content → land the instrument → run the census → state
decidability**. ⚠️ That last clause used to read "→ state decidability → close the inequality"; both
of those steps left with R0e/R0f/R0f′ at Rev 8 (§14), so the ladder now ends at the census. Each rung is
independently committable, has **one** gate, and names the mutation that turns it red. *A mutation
that is only argued does not count; the commit message records the mutated run's output.*

### R0a — the reference-rig probe (zero engine code) — **kills K2 cheapest**

**Lands:** `docs/VG-R0-REFERENCE-RIG.toml` — a machine-readable record: UE version string, install
path, GPU name, driver version, capture tool + version, render resolution, `MaxPixelsPerEdge`, free
disk on the install volume, **the sha256 of
[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) — the freeze (§0.1); the claim file is
deliberately *not* hashed** — the **pass-correspondence map**, and a per-pass table for **one stock
UE5 scene** (no corpus needed).
Plus `crates/boyko_app/tests/vg_r0_reference_rig.rs` reading it. <!-- doc-anchor-ignore -->

**And — landed at Rev 8, ahead of the rung — the freeze tripwire itself:**
`crates/boyko_render/tests/vg_thresholds_freeze.rs`
(`[hash_assertion].hash_tripwire_test`, `[hash_assertion].hash_tripwire_landed_by_rung`). It does
nothing but re-hash the thresholds file against a recorded literal: no GPU, no `dxc`, no corpus,
so a bare `cargo test` executes it, which is what
`[hash_assertion].must_run_in_plain_workspace_test` demands. It carries its own SHA-256
known-answer test against the FIPS vectors — a wrong hash implementation is perfectly *stable*, so
without that the freeze would pass forever while hashing something reproducible by nobody — and its
own sensitivity control, which retunes K1's `d_est_min` in memory and requires the digest to move.
⚠️ It normalises `
` to `
` before hashing: this repository has `core.autocrlf` behaviour
active, so a hash over raw bytes would be a hash of the checkout configuration and would red on a
coworker's machine with nothing changed.

⚠️ **It is landed early ON PURPOSE, and it is a baseline rather than the freeze.**
`freeze_begins_at` says the campaign freeze starts when R0a records the hash into the rig file, and
R0a has not run — so an edit to the thresholds is still *authoring*, and updating the literal in the
same commit is the legitimate response. What exists from today is the property the file's own
`schema_version` / `frozen_at_revision` fields were supposed to provide and did not: **an edit
cannot be silent.** Those two went stale through Rev 4, Rev 5 and Rev 7 because nothing checked
them.

**The record has two shapes, and the gate says which fields each requires.** Rev 2 demanded *"every
field present and not `PENDING`"* over a list including the UE version string, the capture tool and
a stock-scene pass table — **none of which can exist on the `achievable = false` branch.** As
written, R0a could not pass on its own most likely outcome: the same structural hole D3 names,
relocated from the assertion into the field list.

**Gate (one) — `achievable = true` branch, four parts:** (a) every field in the *positive* set
present and not the `PENDING` sentinel — the same discipline `goldens/PINS.toml:15~` defines;
(b) the recorded **GPU name matches the one this engine reports at boot** on this box — a
mechanical cross-check, not a transcription; (c) the recorded resolution equals
`[census].decision_resolution` read from the **thresholds** file, not from a constant this rung
authors; (d) the recorded `VG-CAMPAIGN-THRESHOLDS.toml` sha256 matches the file re-hashed at test
time; and the record carries the **pass-correspondence map** — the reference's pass names for its
stock scene — recorded *here*, at rung one, rather than at the rung that eventually compares two
tables, where it would be written with both of them already in hand. That reasoning survives Rev 8's
re-scope unchanged, which is why R0a still records the map even though the rung that consumes it is
now §14's: whoever writes a correspondence after seeing both sides writes it with the answer
available, and no gate can tell that from an honest one.

**Gate (one) — `achievable = false` branch, three parts:** (a′) the *negative* field set is present
and not `PENDING` — `reason`, `search_method`, `editor_binary_name`, `probed_at`, with `reason` one
of `[k2_probe].reason_values`. **RED mutation:** record a `reason` outside the frozen set → (a′)
reds on set membership. This red belongs to (a′) and is filed here; Rev 11 sited the assertion in
(a′) and its mutation under (b′), which is one assertion under two part letters.

> ⚠️ **Rev 12 withdraws Rev 11's precedence clause from this part, and the withdrawal is the honest
> half of a dilemma rather than a retreat from it.** The clause read *"and, where more than one of
> them holds, the first one `[k2_probe].reason_precedence` lists"*. Enumerate the gate's inputs —
> the recorded `reason` (four legal values) crossed with what the authorities report (engine
> present / absent): over all eight rows the verdict with the clause is **identical to (b′)'s
> verdict without it**, and permuting `reason_precedence` moves no row. The reason is that its
> antecedent is not machine-establishable: only `no_engine_registered` has an oracle (the two
> authorities below), `no_importable_project` and `no_capture_protocol` have none at all, and the
> disk cause is retracted by name further down this rung — *"free disk is recorded as evidence and
> is deliberately NOT an assertion"*. So the clause **does not fire over the whole permitted range
> of the field it names**, which is this campaign's #1 defect family reached from the gate side.
>
> The dilemma is real and both horns were priced. Implementing the clause requires minting a
> `required_free_gb` threshold this document has retracted **twice**, on the same evidence both
> times. Leaving it in place ships a gate clause that cites a frozen field and cannot move a
> verdict, which is precisely the appearance of pre-registration that binds nothing. Rev 12 takes
> the third route the document already uses twice — R0c(e) and free disk itself: **`reason_precedence`
> is demoted to an authoring convention, recorded and deliberately not asserted**
> (`[k2_probe].reason_precedence_status`), and §9.1 enumerates the resulting limit instead of the
> document claiming a check it does not perform. What actually closes blocker 4's substantive
> hazard — the author selecting which assertion runs — is Rev 9's `(b′)`, which asserts something
> for **every** legal value and is untouched by this.

(b′) the re-derivation below passes **for whichever of those values
was recorded**: for `[k2_probe].machine_rederived_reason` the documented authorities must report NO
engine, and for every other value they must report that an engine IS present, per
`[k2_probe].non_rederived_reasons_require_engine_present`; (d) as above, unchanged — the thresholds
hash is asserted on both branches.

> ⚠️ **Two revisions were wrong here in opposite directions, and Rev 9 is the correction of the
> correction.** Rev 7's (b′) asserted flatly that no engine is registered — but §6.2 fires K2 if
> **any** of three prerequisites fails, so a legitimate `achievable = false` caused by disk headroom
> with UE5 installed **red the rung**: the gate could not go green for the cause §11 measures as
> most likely. Rev 8 conditioned the check on the recorded `reason` and enumerated the legal values
> **nowhere** — which traded a cannot-go-green for a **cannot-go-red**. An author writing any string
> other than `no_engine_registered` switched the only machine check off in the same act as recording
> the negative, leaving four non-`PENDING` fields (satisfied by any four non-empty strings) and a
> hash of a docs file (which carries no information about UE5 at all). Of the six repairs Rev 8
> named, this was the only one that **extended** mechanism rather than bounding it, and it is the
> only one that regressed.
>
> Rev 9's rule asserts something for **every** legal value instead of nothing for most of them. The
> three non-machine-checkable causes all *presuppose an install*, so a record claiming one of them
> while the authorities report no engine is self-contradictory and reds. A `reason` outside
> `[k2_probe].reason_values` also reds: an unrecognised string is a typo or a cause nobody has
> thought through, and both must stop the rung rather than disarm it.
>
> **RED mutation for (b′), which Rev 8's version did not have at all:** record
> `reason = "insufficient_disk"` on a box where the authorities report no engine → the
> `non_rederived_reasons_require_engine_present` clause reds. And the converse: record
> `reason = "no_engine_registered"` while an engine IS registered → the re-derivation reds. Both
> directions fire, which is what "the negative is the machine's, not the author's" has to mean.

**RED if / mutations (DEMONSTRATED):** edit the recorded GPU string by one character → (b) reds.
Blank one field of the branch's own set → (a)/(a′) reds. **Edit any threshold in
`VG-CAMPAIGN-THRESHOLDS.toml` → (d) reds** — the P0's mutation, and the one Rev 1 had no way to
express.

**The negative is re-derived, and the search space is not the author's to choose.** Rev 1 let the
record ship `achievable = false` with the test asserting "that shape", which any author satisfies by
typing `false`. Rev 2 re-walked a `probed_paths` list — better, but **still author-parameterised**:
record `["D:\\Epic Games"]` (§11 says it is empty) and the assertion is permanently true, while a
UE5 installed to `C:\Program Files\Epic Games\UE_5.4` fires nothing. So Rev 3:

* the test consults a **bounded, enumerable set of documented authorities**: the Epic launcher's own
  manifest (`C:\ProgramData\Epic\UnrealEngineLauncher\LauncherInstalled.dat`) **and** the registry
  hives that record launcher *and* source builds
  (`HKCU\Software\Epic Games\Unreal Engine\Builds`). A search space the record *describes*
  (`search_method`) but does not *define*;
* it asserts no engine is registered by any of those authorities. A UE5 the launcher or the
  registry knows about reds the stale `false`.

> ⚠️ **Rev 4 said "enumerates fixed volumes itself", and that was the wrong instrument.** It means a
> recursive walk of `C:` (239 GB) and `D:` (238 GB, holding a **58 GB `target/`** per §11) inside a
> `cargo test` process: unbounded runtime, permission denials on system directories,
> junction/reparse-point cycles, OneDrive placeholders, and millions of build-artifact entries.
> That is not a test. It also produced **false positives** — any stray `UnrealEditor.exe` in an
> extracted archive or a sample project would red R0a with no usable UE5 present.
>
> **The residual blindness is recorded in the rig file rather than claimed away.** The launcher
> manifest is not reliably pruned on uninstall, and a hand-placed engine outside both authorities
> is invisible to them. **A search that admits what it cannot see is stronger than one that claims
> to see everything** — and this rung's whole purpose is that a negative be the machine's, honestly
> bounded, rather than the author's.

**Free disk is recorded as evidence and is deliberately NOT an assertion.** Rev 2 asserted free
space below a recorded `required_free_gb`, which fails twice over: the number is author-set (set it
to 500 GB and it can never be met), and its truth value is **controlled by the build directory** —
§11 records `target/` at 58 GB on a volume with 16 GB free, so a routine `cargo clean` flips
"below" to "above" and reds R0a with nothing broken. An assertion a housekeeping command can
falsify is not a gate on UE5 availability. The figure stays in the record, as a §11-class fact.

### R0b — corpus + ingest

**Lands:** the `.glb` decoder (§3.3) registered as a second `LoaderEntry` on `MeshGpu::LOADERS`;
`assets/vg_corpus/CORPUS.toml` + the `.gitignore` rule + `fetch_corpus`;
`crates/boyko_app/tests/vg_corpus_ingest.rs`. <!-- doc-anchor-ignore -->

**Gate (one, five parts):** (a0) **`corpus.arrangement` is not the `PENDING` sentinel** — the
owner VALUES call this rung is blocked on (`[gating].r0b_blocked_by`), asserted here because a
`[gating]` row that no gate part reads blocks nothing. ⚠️ Rev 8 stated in the present indicative
that "the named rung refuses to run while the field is unanswered" while no rung asserted any row;
this is the part that makes the sentence true, and it is deliberately (a0) so the existing lettering
and its mutations are untouched. **RED mutation:** run the rung with the field still `PENDING` → (a0)
reds, and `golden.ps1`'s exit-2 discipline is the precedent for the sentinel's shape;
(a) every corpus payload's sha256 matches its manifest pin; (b) each
`.glb` decodes to a `MeshData` whose triangle count equals the manifest's published count;
(c) **every corpus mesh this rung registers — by whichever path it registers them** — lands a
geometry slot `!= VB_GEOMETRY_RESERVED_SLOT` and a `gMeshMeta` row whose `index_width` /
`vertex_count` / `index_count` match the decoded mesh;

> ⚠️ **Rev 8 widens (c)'s quantifier, and the reason is that the rung's own replacement mutation
> escaped the old one.** (c) read "each mesh, registered through the **streamed** path", while the
> mutation registers a mesh through `register_mesh` **after boot** — which moves it *out of the
> quantifier's domain* rather than falsifying the predicate, so the gate would have gone vacuously
> green on the mutation written to red it. That is the campaign's #1 defect family (an assertion
> quantified over a selection that excludes the failure) reached from the mutation side instead of
> the gate side. The hole the mutation targets is real and verified: `backfill_vb_geometry_slots`
> has no re-arm and exactly one call site, in `boyko_app::runner`'s boot path, so a mesh registered
> at runtime under VB keeps `VB_GEOMETRY_RESERVED_SLOT` forever. (d) the largest corpus mesh registers without allocation
failure (§3.4).

**RED if / mutations (DEMONSTRATED):**
* flip one byte of a pinned hash in `CORPUS.toml` → (a) reds;
* ⚠️ **RETRACTED at Rev 4 — this mutation was DEAD, and it was the one Rev 1–Rev 3 each called the
  rung's most important.** It read: *"register the same mesh through host-authored `register_mesh`
  instead of the streamed path → slot is `0` → (c) reds."* It does **not** red.
  `backfill_vb_geometry_slots` (`crates/boyko_render/src/gpu_upload.rs`, run at
  `crates/boyko_app/src/runner.rs:787~`, after `upload_mesh_assets` and after `finish()`) claims a
  slot for **every** still-reserved mesh under a VB boot — precisely so that any scene's meshes are
  re-fetchable by `vb_resolve`, not only those routed through `register_mesh_vb`. So the mutated
  path lands a real slot, gate (c) stays green, and a mutation written against a
  cannot-go-red defect was itself one. Found by an implementer refuting the premise I briefed.
* **Replacement mutation, and it targets the hole that actually exists:** the back-fill is a **boot
  one-shot** — its own doc states a mesh registered at *runtime* under VB would need it re-run, and
  no scene does that today. So: **register a mesh through `register_mesh` AFTER boot completes →
  its slot stays `VB_GEOMETRY_RESERVED_SLOT` → (c) reds.** Verify the one-shot property against
  `runner.rs` before relying on it; if a later rung makes the back-fill continuous, this mutation
  dies too and the gate needs re-deriving rather than re-wording.
* declare a `TANGENT`-less asset and delete the `generate_tangents` post-pass → (b)/(c) survive but
  the tangent lane is identity; asserted separately so the fallback cannot rot silently.

**Skip policy:** the payload is gitignored, so (a)–(d) skip when it is absent — the same shape as
the `dxc`-dependent gates ([`cluster_cull_spv_sync.rs`](../crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs):196~-204). **Procedural mitigation, and it is
binding: the rung is not commit-eligible until the gate has been run with the corpus present and
its output pasted into the commit message.** A gate proven only on a box that skipped it is not a
gate.

### R0c — the census instrument + its sensitivity control

**Lands:** `TRANSFER_SRC` on the `vb_id` ring ([`targets.rs`](../crates/boyko_rhi_vulkan/src/present/targets.rs):862~-872); an `Option`-threaded census
readback armed by env knob; the host-side histogram + triangles-per-pixel reducer;
`crates/boyko_app/tests/vg_density_census.rs`. <!-- doc-anchor-ignore -->

> ⚠️ **R0c lands the first in-frame image readback in the shipped recorder, and that is a bigger
> step than "reuse an existing seam" implies.** Every `copy_image_to_buffer` call site in this tree
> today is under `crates/boyko_rhi_vulkan/tests/`; there is **none** in `src/present/`, and
> [`frame_driver.rs`](../crates/boyko_rhi_vulkan/src/present/frame_driver.rs):750~ records that the engine deliberately has no depth readback. So R0c adds
> (i) a new layout transition of a **ring** image — `COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL
> →` its `SAMPLED` read — inside the RDG auto-barrier system, and (ii) a **host read of a per-FIF
> resource**, which is the exact shape of this project's recorded cross-frame bug class (host
> access racing the fence on per-FIF rings, with `FRAMES_IN_FLIGHT == 2` at `ui/mod.rs:87~`).
> Neither is visible to gate (a), because both exist only on **armed** frames — the frames the
> goldens never render. The readback must therefore wait on the frame's own fence before mapping,
> and that ordering is asserted in the rung's own test, not assumed.

**Gate (one, five parts):** (a) **every VB image golden byte-identical** to its `PINS.toml` pin
with the census unarmed — the usage widening and the unarmed `Option` must cost nothing. *Scoped to
the blessed legs:* §9 clause 4 records two `sha256_hwrt = "PENDING"` pins on which `golden.ps1`
exits 2 by design, and a gate quantified over an unblessed pin is the vacuous-selection defect
again;
(b) on a **procedurally generated** fixture whose screen-space triangle size is analytically known,
the census's modal bucket is the analytic bucket;
(c) the census's covered-pixel total agrees with `sv0_oracle::rasterize`'s `covered_count` **on that
same procedural fixture, at 512²**, within `[pre_registered].r0c_oracle_coverage_tolerance` — read
from the frozen file **by name**, not minted here, because this is the only gate anywhere that
validates covered pixels, which is `D_est`'s own denominator, and a tolerance supplied after seeing
the disagreement is chosen by whoever measures against it. Non-zero deliberately: the oracle is a
host mirror with its own sample-point rule, so exact agreement would be a coincidence rather than a
check. Scoped to the fixture because the oracle takes one mesh and translation-only instances (§5.7)
and cannot reach the corpus at any resolution;
(c′) the **non-degeneracy precondition** — `[k1_instrument].min_covered_pixels` and
`[k1_instrument].min_visible_tris` — holds on the censused frame, so a sentinel-only readback fails
here rather than flowing into R0d as a division by nothing;
(d) the ladder is driven from `[census].resolution_ladder` in the **thresholds** file, whose sha256
the test re-asserts, the census produces one row per rung, **and the readback's own dimensions equal
the requested rung** (`[census].assert_achieved_extent`) — a ladder silently truncated, or silently
clamped by the OS, reds;
(e) **cross-process `vb_id` identity is MEASURED and RECORDED here — and deliberately NOT
asserted.** ⚠️ Rev 4 wrote (e) as a gate, which made it incoherent with R0d: a negative result is
something the plan explicitly calls *"a real finding about the raster path"* and wants recorded,
yet asserting identity would **red R0c** and, via §9 clause 3, make the rung not commit-eligible —
**a legitimate finding blocking the ladder.** So (e) produces a number and writes it into
`docs/VG-R0-DENSITY-CENSUS.md`; **R0d** is where it becomes a gate, in whichever of the two shapes
(e) established (see R0d). And (e) is measured **in the regime where it can actually fail** — the
**top ladder rung on the corpus**, not R0c's 512² procedural fixture: §12's own warning is that
ties are common *"at 2160p on a multi-million-triangle corpus where near-coplanar sub-pixel
triangles"* collide, and measuring a hypothesis where it is least likely to fail is the
vacuous-selection defect wearing a lab coat.

**RED if / mutations (DEMONSTRATED):**
* (b): subdivide the procedural fixture 4× → the modal bucket must move by **two** buckets. A
  sensitivity control that only asserts "the number changed" is the defect this campaign keeps
  finding; the required *direction and magnitude* is what makes it a gate.
* (a): record the census copy unconditionally instead of under the `Option` → the command stream
  changes on golden frames → pins move → red.
* (c): feed the reducer the CPU oracle's own coverage instead of the readback → (c) passes
  vacuously while (b) fails; the pairing is what proves (c) is not self-referential.

### R0d — the census run — **K1's evidence**

**Lands:** the census executed over the corpus at the committed camera paths, **at every rung of
the frozen resolution ladder**; results written to `docs/VG-R0-DENSITY-CENSUS.md` as the density
curve, and the **decision-bearing** numbers pinned as literals in the test under the MEASURED
discipline.

**Gate (one, four parts):** (a) the census is **reproduced across `[census].cross_run_sessions`
separate processes** under `[census].cross_run_gate` — **the sha256 of the readback itself**;
(b) `D_est`, the convergence check, the histogram and both `[k1].report_only` statistics
(`visible_tri_per_covered_pixel` and `submitted_per_covered_pixel` — the saturating raw reading and
the cull-efficiency reading, neither of which adjudicates anything, and the modal bucket alongside
them under `[k1].modal_bucket_role`) are produced at **every** ladder rung, so the
resolution-dependence is on the page rather than in the choice of one row; (c) the **non-degeneracy precondition** holds at the decision resolution and at the top rung —
covered pixels at or above `[k1_instrument].min_covered_pixels` and distinct visible triangles at or
above `[k1_instrument].min_visible_tris` — because `D_est` and the convergence check are both
divisions and a sentinel-only readback proves nothing in either direction. ⚠️ This precondition was
stated in §5.7 and frozen in the companion file from Rev 4 onward and appeared in **no gate part of
either rung that produces the numbers**; Rev 8 lands it here and at R0c.
**(d) the census covered the whole aggregation domain** — exactly one census row per camera path
enumerated in `assets/vg_corpus/CORPUS.toml`, with the sha256 of that enumeration recorded beside
the readback hashes (`[k1].committed_paths_rule`). ⚠️ Rev 11 froze `min` over committed paths as
K1's aggregation and left the domain asserted by nothing, which is the same shape as a threshold
with no reader: under MIN the cheap lever is not adding a flattering path but **omitting an
unflattering one**, and an omitted path leaves no diff and no census row. This part is what makes
the omission visible. It bounds the *measurement*, not the *choice*: which paths are committed is
settled one rung earlier at R0b, by a different act, and §9.1 records that residual rather than
claiming it away.

**Measured and recorded, deliberately NOT a gate part:** the histogram's modal-bucket shift between
adjacent rungs, against the **per-pair `log2` of the actual area ratio**
(`[k1_instrument].histogram_shift_rule`), with the residual reported per pair and compared to
`[k1_instrument].histogram_shift_tolerance_buckets` **as a reported margin, not an assertion** —
over the **two** non-excluded pairs (1080p→1440p, 1440p→2160p),
`[k1_instrument].histogram_shift_excludes_rungs` naming rung 0 as a different frustum (§5.7).

> ⚠️ **Rev 8 demotes this from a gate, and the arithmetic is why.** The measured shift is a
> difference of **integer** bucket indices; the targets are 0.830075 and 1.169925; the tolerance is
> 0.35. So the rule accepts exactly one integer — 1 — on both pairs, which is an integer assertion
> wearing a tolerance, in a clause whose own frozen comment says *"the residual reported rather
> than an integer asserted"*. Worse, the two targets sum to **exactly 2.000** (3840/1920 =
> 2160/1080 = 2 exactly), so **both** splits a correct instrument can produce — (1,1) and (0,2) —
> satisfy the scaling law over the retained span, and which one occurs is set by the sub-bucket
> phase of the corpus's modal triangle size: a property of the assets, not of the instrument.
> And the histogram is left-censored at one covered pixel, so in the micro-polygon regime the
> census exists to serve — where §9 itself says `visible_tris` is still climbing steeply between
> exactly these rungs, every newly visible triangle entering at bucket 0 — the mode is pushed the
> wrong way. **The gate would red hardest exactly when the campaign's premise is most strongly
> confirmed.** That is the defect family this campaign hunts, pointing the other way.
>
> The disposition is the one §8 R0c(e) already chose and §9 clause 3 already rules on: *a
> legitimate finding must not red the rung and block the ladder*. The residual is produced,
> written into `docs/VG-R0-DENSITY-CENSUS.md` and interpreted by a reader; it does not adjudicate.
> Making it a gate again requires a statistic that is not re-binned onto the same integer lattice
> at both rungs — the `log2` ratio of a fixed quantile of covered-pixel counts is the obvious
> candidate — and that is a new instrument, not a retuned tolerance.
>
> **The rung-0 exclusion is SOUND and must not be "fixed" along with it:** 512² is 1:1 while the
> other three rungs are 16:9, so that pair is a different frustum rather than a rescaling, and the
> premise does not hold across it at all.

> ⚠️ **Rev 6 fixed this clause, and it is worth naming why it survived Rev 5.** §5.7 replaced the
> two-bucket constant and explained at length that it was *red by construction* — and R0d, **the
> rung that implements it**, still said *"the two-bucket shift … checked three times"* with a
> cross-reference to §5.5. So the frozen file and the rung disagreed about the decision rule: an
> implementer coding from §8 builds the gate that cannot go green; one coding from the TOML builds
> a different gate. That is Rev 2's inverted `all_three_below` string exactly, and it shows that
> **rewriting the explanation is not rewriting the gate.** "Three times" was also wrong once rung 0
> was excluded — two pairs, not three.

*The gate is that the instrument produced a reproducible number — **not** that the number is
favourable.* K1 is adjudicated in §9, deliberately, so that an unfavourable result cannot be
mistaken for a failing rung and quietly re-run until it passes.

> ⚠️ **`byte_identical` is a hypothesis this rung tests, not a property Rev 2 was entitled to
> assume.** Rev 2 justified it by *"a pipeline whose cross-process determinism the 24 existing
> golden pins already assert."* That justification is invalid: the pins hash an **8-bit shaded
> BMP** at 512² of a five-sphere fixture, and this project has **MEASURED** them blind below
> ~2⁻¹⁶ relative. Two adjacent triangles of a smooth mesh can shade identically to 8 bits and
> carry **different `vb_id`**. `vb_id` identity is a strictly finer function of the same state,
> and it is being asserted at 2160p on a multi-million-triangle corpus where near-coplanar
> sub-pixel triangles make coverage ties common — a regime the pins have never visited. **R0c
> measures it first** (gate (e)) and reports the result; R0d relies on it only if it held.

**If the readback proves non-deterministic** — e.g. a driver-side raster order that changes which
triangle wins a coverage tie — that is a **real finding about the raster path**, and it is recorded
as one. R0c(e) is what discovers it; **R0d then runs in its second shape**: the finding is entered
in §11.1 by name and date *first*, and only then does `[census].cross_run_spread_fallback` become
gate (a). The ladder is not blocked by a true discovery, and the fallback is still unreachable
without the dated entry — so it remains impossible to reach for it to make a run pass.

⚠️ **`cross_run_spread_fallback` names no statistic, and Rev 5 does not pretend it does.** A hash
has no spread. If the second shape is ever entered, the amendment must define *spread of what* —
per-pixel disagreement fraction, or the spread of the derived statistics — because a bound without
its denominator is R16 again, this time inside the frozen file itself. The value stays `0.05` as a
placeholder magnitude and is **not usable until that amendment names its quantity.**

**RED if / mutations, re-derived at Rev 9 until each fires against the gate it names.**

⚠️ **Rev 8 left this list describing the PREVIOUS gate lettering, and that is its own defect.**
Demoting the histogram check renumbered the gate list and not the mutation list, so "(c)" named two
different predicates 76 lines apart and the demoted assertion was re-armed by the mutation that was
supposed to probe it. Renumbering is exactly where this campaign's defects land, and the fix for a
renumbering is to re-derive, not to re-word.

* **(a) — the cross-process agreement gate.** ⚠️ Rev 2's *"point two of the three runs at different
  camera paths"* is not a gate test: it changes the input and would red any hash of anything. But
  Rev 5's replacement — *"permute the spawn order of two identical instances"* — **does not fire
  either, and it survived three revisions.** (a) asserts `H₁ = H₂ = H₃` over three processes of one
  build. A spawn-order permutation is an edit to committed scene construction, so it is present
  identically in all three processes: every `Hᵢ` moves to the same new value and the predicate stays
  **true**. The "shaded pin is byte-identical" justification gives the error away — that is an
  argument about comparison against a *pin*, which is R0c(a)'s shape, not an agreement predicate's.
  The mutation that does fire is one that breaks agreement **between** processes: seed the census
  readback with a per-process value (the PID, or the process start tick) at one texel → `H₁ ≠ H₂` →
  (a) reds. It is artificial on purpose: agreement gates are falsified by divergence, and nothing an
  author can write into committed source diverges across processes of one build. That is also the
  honest reading of what (a) tests — the driver-side nondeterminism `[census].cross_run_gate`'s own
  comment flags as an untested hypothesis at 2160p.
* **(b)** — drop the ladder to its decision row only → (b) reds.
* **(c) — the non-degeneracy precondition.** Render the corpus scene with the camera pulled back far
  enough that covered pixels at the decision resolution fall below
  `[k1_instrument].min_covered_pixels`, or point it at empty space so `visible_tris` at the top rung
  falls below `[k1_instrument].min_visible_tris` → (c) reds. The point of the mutation is that the
  rung must refuse to adjudicate a frame it cannot adjudicate, rather than dividing by it: on a
  sentinel-only readback the convergence check reads `0 ≤ 0` (converged) and `D_est = 0`, which is
  how an empty frame came to satisfy K1's fire condition in an earlier revision.
* **(d) — the aggregation domain.** Delete one camera path from `CORPUS.toml`'s enumeration, or skip
  it in the run, and leave everything else alone → the census yields one row fewer than the
  enumeration → (d) reds. **It isolates**, which is the property the other three make easy to get
  wrong: the drop is present identically in all three processes, so (a)'s agreement predicate stays
  true; it removes no ladder rung, so (b) stays true; and the surviving paths are as non-degenerate
  as they were, so (c) stays true. (d) is the only part that moves — which is what a mutation
  targeting one part must show, and is why the mutation is stated in terms of the *enumeration*
  rather than in terms of the path's density: a mutation phrased as "drop the weakest path" would
  also move `D_est` and could not distinguish (d) from the rule it feeds.
* **The histogram residual has no mutation, because it is no longer a gate.** It is produced and
  written into `docs/VG-R0-DENSITY-CENSUS.md` (see the demotion above). A mutation list entry for it
  would re-arm exactly what the demotion removed.

### R0e / R0f / R0f′ — REMOVED AT REV 8, and the removal IS the revision

Three rungs stood here: the decidability statement (K3's test), the Nanite reference capture, and
the absolute-mode closure. Between them they measured the campaign's decidability floor and closed
the ONE gate.

They are gone from R0 because **the ONE gate's left-hand side has no measurand at this rung.** The
floor is a *resolvable delta*, and a delta needs two configurations to sit between — but R0 lands no
meshlet, no cluster and no LOD. The frozen `[decidability]` table named our side's denominator
explicitly, and it was the **armed paired delta**; R0 has no arm. Six revisions of pre-registration
machinery were guarding a number that cannot be measured until the thing being claimed exists, and
five of the eight blocking P0s Rev 7's review returned were downstream of that one over-reach.

Nothing is discarded. The specification, the denominators, the two-sided absolute gate form, and the
eight P0s — each with the arithmetic that refuted it — are in **§14**, as requirements on whoever
freezes them at the rung that lands an arm. §7's harness contract stays where it is and binds that
rung.

**What R0 keeps of K2:** R0a still records whether the reference is achievable, and §9 clause 2
still states the disposition. What R0 no longer does is *capture* the reference, or compare anything
against it.

---

## 9. ABORT criteria

The rung is **reverted or the campaign re-scoped** — not softened mid-flight — if any of:

1. **K1 — no content.** The live rule is [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml)
   `[k1].k1_decision_rule`.
   ⚠️ **Through Rev 7 this clause opened by citing a `k1_fire_rule` field that does not exist**, and
   Rev 7 "fixed" it by annotating the dangling name in place. That is not a repair: the operative
   sentence still spelled the name an implementer would grep for, and a gate pointing at nothing is
   the defect the frozen file exists to prevent. Rev 8 deletes it — including from this correction,
   which is why the dead name is described here rather than quoted in citation form.
   [`tests/vg_symbol_reachability.rs`](../tests/vg_symbol_reachability.rs) now catches the class
   mechanically, and it cannot tell a live citation from a historical one, so leaving the spelling
   in place would have cost a permanent baseline exception for a name nothing should resolve.

   **Aggregated over camera paths by `[k1].k1_path_aggregation` — the MINIMUM.** ⚠️ Until Rev 11
   nothing said how the per-path readings combine, and the census is explicitly reported *per
   camera path* while K1 is ONE decision: whoever picked the aggregation picked the answer. MIN
   because refutation is the campaign-favourable outcome, so it must clear the bar on the weakest
   committed path rather than the strongest — and because a MIN cannot be raised by authoring one
   more flattering path.

   **The rule, per direction**, and the frozen file states each half as DATA so no later rung
   re-derives it from prose: `[k1_instrument].d_est_bound_direction` is `"lower"`,
   `[k1_instrument].d_est_may_refute_k1` is true and `[k1_instrument].d_est_may_fire_k1` is false.
   ⚠️ Those three fields carried the campaign's single most important correction and, through
   Rev 7, **were named nowhere in this document** — frozen precisely so prose could not drift from
   them, and then not read by the prose.

   `D_est ≥ [k1].d_est_min` **refutes K1 regardless of convergence**: non-convergence means
   `visible_tris` is still rising, so `D_est` *understates*, and an understatement already at or
   above the threshold still proves density ≥ 1 triangle/pixel. Convergence — the top-two-rung gap
   coming in under `[k1_instrument].ladder_convergence_margin` — is a precondition for **firing**,
   never for **refuting** (`[k1_instrument].on_not_converged_refute_direction`,
   `[k1_instrument].on_not_converged_fire_direction`). Non-degeneracy
   (`[k1_instrument].min_covered_pixels`, `[k1_instrument].min_visible_tris`) is required in
   **both** directions: a sentinel-only readback proves nothing either way.

   ### K1 has exactly two reachable outcomes at R0, and UNDECIDED is the likely one

   | Outcome | Condition | Disposition |
   |---|---|---|
   | **K1 REFUTED** | `D_est ≥ [k1].d_est_min` at `[census].decision_resolution`, non-degeneracy met | The mechanism exists. The ladder proceeds to R1. This is the cheap decisive case §5.6 front-loads, and it is R0's whole claim. |
   | **K1 UNDECIDED** | `D_est` below the threshold, or non-degeneracy unmet | **Owner VALUES call — §13 Q2**, held by `k1_outcome.undecided_disposition` in [`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml), which **blocks R1**. R0 cannot distinguish "genuinely sparse" from "the instrument hit its ceiling seen from below", because firing needs an upper bound R0 has no buildable instrument for. |
   | **K1 FIRED** | — | **Unreachable at R0** (`[k1].k1_fire_at_r0`). Requires the unsolved upper-bound instrument. |

   **On UNDECIDED the ladder does NOT silently proceed.** The owner chooses: accept the premise
   unadjudicated and proceed to R1 knowing K1 was never tested; change the target content class and
   re-run R0b–R0d; or fund the upper-bound instrument as its own campaign. The one route foreclosed
   is the one this document forecloses everywhere else — re-running the census until a number comes
   out favourable.

   > ⚠️ **Two dead rules are recorded rather than deleted, because each looked decisive.** Rev 2's
   > `all_three_below` put a `_max` among two `_min`s, so on the canonical no-mechanism scene — a
   > few giant flat quads — two conjuncts held, the third did not, and **K1 failed to fire on the
   > exact scene it was written to catch**; an implementer coding from the TOML would have built a
   > kill that fires when triangles are *small*, i.e. when the premise is *confirmed*. Rev 4's
   > two-conjunct replacement was **redundant**: a modal bucket above 16 px implies
   > `visible_tris ≲ covered_px/16`, hence `D_est ≲ 0.06 ≪ 1.0`, so conjunct 1 held automatically
   > whenever conjunct 2 did. "Two conjuncts, both pointing the same way" was one conjunct and a
   > weaker consequence of it. Rev 8 deletes both fields from the frozen file rather than leaving
   > them annotated.

   > ⚠️ **R0 CANNOT FIRE K1, and that is a stated scope boundary rather than a rung nobody wrote.**
   > Rev 5 named the firing instrument as a frustum+backface survivor counter in `vb_raster.fs.hlsl`
   > *"scoped as its own rung"* — and §8 contained no such rung. It is wrong twice, the second
   > fatally: **(a) wrong stage** — a fragment shader runs only for fragments that survived
   > rasterisation and, with early-Z, the depth test, i.e. approximately the *visible* set that
   > `vb_id` already caps, and §2 records that the per-primitive lane is not independently reachable
   > without a mesh shader, one draw per meshlet, or a software rasteriser; **(b) probably inert
   > regardless** — survivors include every *occluded* in-frustum front-facing triangle, and depth
   > complexity on a multi-million-triangle corpus is where the count lives, so
   > `survivors/covered < 1.0` cannot hold whatever the visible triangle size is. That is
   > `submitted/covered`'s self-satisfaction with a 2–4× constant knocked off.
   > **Naming a rung that cannot be built is worse than naming none.**

2. **K2 — no baseline.** R0a records `achievable = false` **and the test re-derives it** (§8 R0a).
   Then *"faster than Nanite"* is not currently falsifiable and the goal is restated as an
   **absolute** ms-at-quality target — an owner VALUES call, taken consciously at rung one.
   **This is a re-scope, not an abort.**
   ⚠️ Rev 8 changes what follows it. Through Rev 7 this clause said "the ladder continues to R0f′,
   which closes the same inequality"; R0f′ is gone from R0 (§8), so what K2 firing selects is which
   *mode* §14's rung will eventually freeze, not a rung of R0. R0 records the branch and stops
   there.

3. **The instrument is untrustworthy rather than the result being bad — and this has its own
   disposition, because it is the case that gets misread.** If R0c's sensitivity control (b) fails
   while (a) and (c) pass, the correct reading is *the instrument is blind*, **not** *the effect is
   absent*. Outcome: the rung is **not** commit-eligible, no number from it enters any later gate,
   and the failure is recorded in §11 with its date. The sibling rung's ABAB null control is
   precisely this case: three armed sessions looked tidy and inside their gate while the control
   said a third of the "signal" was ordering bias.

4. **Golden-bless throughput.** Two of the pins in `goldens/PINS.toml` carry
   `sha256_hwrt = "PENDING"` — their software legs are blessed, their hwrt legs are not, and
   `golden.ps1` exits 2 on a PENDING leg by design, so any gate quantified over one is vacuous until
   it is blessed. R0 moves no pin, so it is unaffected; but the first byte-moving rung of this
   campaign starts on an incompletely-green corpus, and §13 Q4 puts the bless-bandwidth question to
   the owner before that rung is scheduled, not after.

**K3 — the undecidable harness — is no longer an R0 abort criterion.** It moved to §14 with the
rungs that tested it. R0 builds no harness and measures no delta, so there is nothing at this rung
for K3 to be true or false about. The criterion returns, unchanged in substance, at the rung that
lands an arm.

### 9.1 What R0 does not decide — enumerated, because a bounded claim is the whole point of Rev 8

Rev 7's §0 opened with *"the three ways it kills the campaign"* and its own §9 then admitted the
headline was false as written. Rather than a headline and a retraction, the limits are listed:

* **K1 cannot be FIRED.** Only refuted or left undecided. Firing needs an upper bound on visible
  density whose firing condition is demonstrably not precluded by R0b's own high-poly corpus gate —
  an unsolved design problem, recorded as unsolved
  (`[k1].k1_fire_instrument_status`), and out of R0's scope until someone solves it.
* **K2's four causes are all checked, but only one is re-derived, and one configuration cannot go
  green.** ⚠️ Rev 9 changed this and Rev 8's wording survived it, so the bullet said the opposite of
  the gate beneath it. `[k2_probe].reason_values` enumerates **four** causes — no engine, short
  disk, no importable project, no capture protocol; §6.2's "three prerequisites" counts disk as part
  of the install, and the two counts are reconciled there. Every value is asserted against
  something: the one named by `machine_rederived_reason` is confirmed against the documented
  authorities, and the other three must be contradicted by those authorities reporting an engine
  IS present, since all three presuppose an install. ⚠️ **Rev 10 concluded from that "a disk-caused
  negative is checked, just not by the same instrument", and Rev 12 withdraws it as an
  overstatement.** What (b′) asserts for a disk-caused negative is the cause's **presupposition** —
  that an engine is present — so what is checked is that the record is not self-contradictory, not
  that the disk was short. No instrument in R0 measures free disk against a threshold, because this
  document retracted that threshold twice. The correct statement is the narrow one: every legal
  `reason` now carries *some* machine assertion, and for three of the four that assertion is about
  the presupposition rather than the cause.
  Three limits, all named in `[k2_probe]` rather than left to be found: more than one cause can hold
  at once (§11 measures this box as no-engine **and** short-disk simultaneously), so
  `reason_precedence` fixes which one is recorded — ⚠️ **as an authoring convention only, and Rev 12
  demotes it from R0a's gate for the reason recorded at `[k2_probe].reason_precedence_status`: over
  all eight of that gate's inputs no permutation of the list changes a verdict, so which of two
  simultaneously-true causes was written down is NOT machine-checked**; and a hand-placed engine
  invisible to both
  authorities, combined with short disk, **cannot go green**
  (`[k2_probe].hand_placed_engine_plus_short_disk_reds`) — the honest value reds and the only
  green value is false. That is the price of a bounded search rather than the unbounded volume walk
  Rev 4 retracted, and the disposition is that R0a reds until the engine is registered.
* **The census's cross-rung histogram shift is measured, not gated** (§8 R0d). It is not
  interpretable near the one-pixel censoring floor — the micro-polygon regime the census exists for
  — so gating on it would red hardest exactly where the campaign's premise is most strongly
  confirmed.
* **R0 has no representativeness floor, and the non-degeneracy floors are not one.**
  `[k1_instrument].min_covered_pixels` was frozen as an EMPTY-FRAME guard — a sentinel-only readback
  makes `D_est` a division by nothing — and 1024 px is about a hundredth of a percent of the
  decision resolution. At the rule's own boundary, `covered_pixels = visible_tris = 1024` gives
  `D_est = 1.0`, so K1 is refuted from a frame covering **0.049%** of the screen. `D_est` is
  scale-free by construction (both terms shrink with the covered region), so no floor on that axis
  can carry representativeness; it needs a floor on covered **fraction**, and R0 does not have one —
  `[k1_instrument].representativeness_floor_status` records it UNSOLVED rather than giving it an
  authored number nobody can justify yet.
  ⚠️ **And what limits cherry-picking is narrower than Rev 10 wrote here.** That text read: the
  paths "are committed as test constants and R0d(b) requires every statistic at every rung, so an
  unrepresentative frame appears as a row in the census rather than being selectable afterwards."
  R0d(b) quantifies over **ladder rungs**, not over paths — its own red mutation is *"drop the
  ladder to its decision row only"* — so it catches a path measured at too few rungs and never a
  path that was never measured at all. Under Rev 11's `min`-over-paths aggregation that is exactly
  the live lever, because MIN is monotone under set inclusion. **R0d(d) is what closes it**: one
  census row per enumerated path, enumeration hashed beside the readback.
  **The residual, stated rather than claimed away — R0d bounds the measurement, not the choice.**
  Which paths are committed is settled at R0b, one rung earlier and by a different act, and no gate
  in R0 asserts that the committed set is representative of the content class. That is the same
  unsolved axis as the covered-fraction floor above, reached from the domain side instead of the
  frame side.
* **When a censused frame fails non-degeneracy, R0d reds** — the rung is not commit-eligible and
  nothing is adjudicated. ⚠️ `[k1].k1_decision_rule` also maps that input to "UNDECIDED, escalate",
  which is a different act; **R0d's gate takes precedence**, because a frame that cannot be
  adjudicated is an instrument failure, not a finding about content, and §9 clause 3 already rules
  that an instrument failure must not enter a later gate. The conjunct inside `k1_decision_rule` is
  therefore vacuous *within R0* — R0d(c) asserts the same two floors before the rule is ever
  evaluated — and it earns its place only at a rung that adjudicates without R0d's gate.
* **No comparative claim is evaluated, decided or pre-registered.** There is no ONE gate at R0
  (§14). The deferral orphans the downstream gate rows that cite R0's floor, and **§14.1 owns that
  count — this bullet cites it and deliberately does not restate it.** ⚠️ Rev 11 restated it here as
  *five*, in the bounding enumeration, in the same revision that re-derived it to **one** twelve
  hundred lines below; a fact stated in two places is a fact that will disagree with itself. R0 does
  not fix that row and cannot — it is in another document — so it is the R1 author's first
  inherited problem.
* **This document's `file.rs:N` anchors are machine-checked only in part** (§12), and its citations of the
  two frozen files' field names **are** (`tests/vg_symbol_reachability.rs`). Those are different
  guarantees and only the second is mechanical.

---

## 10. Risks

| # | Risk | Precedent | Mitigation |
|---|---|---|---|
| R1 | **Vacuously-green gate** — an assertion quantified over an empty or self-referential selection. | The campaign's #1 recurring defect; found five times in the sibling plan alone. | Every rung names a mutation and the commit records its output; R0c(b)/(c) are deliberately paired so neither can pass alone. |
| R2 | **A procedural corpus makes K1 untestable.** | New, and it is why §4.2 rejects the cheapest corpus option. | The corpus is fetched real content; procedural geometry is confined to R0c's sensitivity control. |
| R3 | **The harness measures its own resolution, or its A/B rides the ring.** | MEASURED in the sibling rung, both of them: a "spread" that was one median lattice step, and an ABAB phase perfectly aliased with `FRAMES_IN_FLIGHT == 2`. | §7 clauses 1, 3–4: ABBA with the residual reported; the quantum measured by tick GCD and the spread gate read against it. |
| R4 | **`WAIT_BIT` readback hangs instead of failing.** | [`gpu_timing.rs`](../crates/boyko_rhi_vulkan/src/present/gpu_timing.rs):344~ documents the deadlock; three separate collectors exist because of it. | §7's written-pair bitmask, asserted before the read — binding on §14's rung, which is the one that brackets passes. Not an R0 risk any more. |
| R5 | **Stale doc sends the importer down the `None` path.** | Verified: ≥6 comments still claim `VB_IMPLEMENTED == false` while [`render_path_config.rs`](../crates/boyko_render/src/render_path_config.rs):130~ says `true`. | R0b's second red mutation targets exactly this; fixing the comments is a separate one-line commit, deliberately not absorbed here. |
| R6 | **Host-visible residency ceiling.** | [`mesh_assets.rs`](../crates/boyko_render/src/mesh_assets.rs):320~: every mesh buffer is `HostVisibleCoherent`. | R0b gate (d); the device-local + staging path is a named follow-up, not R0 work. |
| R7 | **The `vb_id` usage widening perturbs a golden.** | New. | R0c gate (a) over every VB pin, with a demonstrated red (record the copy unconditionally). |
| R8 | **UE5 capture measures a different scene than our census.** | New — the two engines must load the same bytes. | §4.3: an asset that cannot be imported by both is not corpus material; R0a(c) pins the resolution across both. |
| R9 | **Disk exhaustion masquerading as a build failure.** | This project's record: `target/` has filled this disk and surfaced as linker errors. | §11 records the measured headroom; R0a's record carries free-disk as a required field, and R0a's negative branch **re-reads** it at test time. |
| R10 | **The claim is set to meet the floor.** The cheapest way to close the ONE gate is to write the number on the right after seeing the left. | The P0 of Rev 1; of Rev 2, answered with a hash around the string `PENDING`; and of Rev 3–Rev 5, answered with an ordering rule that **named one rung for two instruments**. | **MOVED TO §14 AT REV 8, unresolved.** There is no claim at R0 to set against a floor, so this risk has no R0 surface — but it is not solved: §14.4 P0-5 carries the worked example showing the post-fill edit window still open on one branch. ⚠️ Rev 3–Rev 5 claimed here that ordering *"does not depend on anyone noticing an edit"* — **retracted**; ordering constrains commits, not knowledge, and party separation is what carries the weight. |
| R11 | **A statistic that cannot exceed its own threshold.** | Rev 1: `visible_tri_per_covered_pixel ≤ 1` by construction. Rev 2: `submitted/covered < 1.0` precluded by R0b's corpus gate. **Rev 3–Rev 5: `D_est` capped at exactly 4.0 and a *lower* bound firing a kill.** Three instruments, one defect. | §5.6's directional split — `D_est` may only **refute** K1 — plus `[k1].k1_fire_at_r0 = false`, because the proposed firing instrument is both mis-sited (a fragment shader cannot see frustum/backface survivors) and **probably inert for the same reason Rev 2's was** (~2.5 M survivors vs ~2.07 M covered pixels). ⚠️ Rev 4's entry here called the estimator *"uncapped"*. |
| R12 | **The census resolution silently decides K1.** Density scales as 1/resolution². | New in Rev 2 and the one fix that survived review. | Frozen ladder + frozen decision resolution; the curve is reported at every rung; **and the achieved extent is asserted**, because OS clamping is already a recorded hazard here at 512². |
| R13 | **The most likely branch has no gate.** §11 measures no UE5 on this box. | New — and it kept re-appearing: through Rev 5 the absolute branch had the ordering rule attached to the wrong rung **and** a gate (b) that passed for its own named red mutation. | R0a's negative is re-derived over **bounded documented authorities** (launcher manifest + the registry hives recording launcher *and* source builds), with residual blindness recorded. ⚠️ Rev 4's *"enumerate fixed volumes"* is **retracted** — a recursive walk of two ~240 GB volumes inside a `cargo test`, with false positives from any stray binary. ⚠️ **Still only partly mitigated at Rev 8:** §6.2 fires K2 on *any* of three prerequisites and R0a re-derives only the first, so a legitimate negative caused by disk — the cause §11 calls most likely — is recorded rather than machine-checked (§9.1). |
| R14 | **A frozen file whose schedule requires it to change.** A tripwire that fires routinely carries no signal, and a routine re-record can launder a threshold edit. | **Measured in Rev 2 by inspection:** its recorded hash was *guaranteed* to break at the `corpus.arrangement` fill, before the first rung that asserted it. | The split: thresholds hashed and never edited; claim unhashed and gated by the `PENDING` sentinel. |
| R15 | **A harness asked for a quantity its algebra removes.** | **Measured:** ABBA recovers `τ` by cancelling `μ`, `γ` and `β` — exactly what an absolute reading needs, and Rev 2 assumed otherwise. | **MOVED TO §14 AT REV 8.** R0 measures no delta, so nothing here asks a harness for a quantity its algebra removes. §14.2 carries the requirement that absolute mode gets its own instrument, its own pre-registered ceiling, and the honest statement that it is the weaker one. |
| R16 | **A literal transferred without its denominator.** | **Measured in Rev 2:** the sibling's 0.10 null-control gate moved from *armed delta* to *absolute pass median*, a ~20× weakening under which the precedent's own red event would have passed. | Denominators written down next to every fraction in `[decidability]`. |

---

## 11. Environment record — dated, and NOTHING READS THESE NUMBERS

Fenced exception to this document's no-measured-numbers-in-prose rule. These are facts about the
machine and the tree as of authoring; they are **evidence for design decisions, not gate
thresholds**. No test reads them, and any rung that depends on one re-derives it in its own code.

**Probed 2026-07-26, this box, working tree on branch `feat/multi-paradigm-render` at `a139799`.**

* **UE5:** no installation present. The only Epic-shaped directory on either volume,
  `D:\Epic Games`, exists and is **empty** (0 entries). No `UnrealEditor.exe` anywhere probed.
* **Free space:** `C:` 71.9 GB free of 238.3 GB; `D:` (the repo volume) 18.5 GB free of 237.7 GB.

**Re-probed 2026-07-27 at Rev 2, same box, working tree at `13f1c9a`** — recorded because the
figure **moved in the direction that matters**, and because R0a's negative branch now re-derives it
rather than trusting this record:

* **Free space:** `C:` **63 GB** free of 239 GB; `D:` (the repo volume) **16 GB** free of 238 GB.
  Both fell over one day of ordinary work. `target/` alone is **58 GB** — larger than the free space
  on the volume that holds it, and this project's standing hazard is that exhausting it surfaces as
  mingw linker errors rather than as a disk error.
* **What this does to K2.** A UE5 editor install plus a project plus its derived-data cache does not
  fit on `D:` today and is uncomfortable on `C:`. K2 firing is not a hypothetical branch of this
  plan — on the measured state of this machine it is the **expected** one, which is exactly why
  Rev 2 refuses to leave it ungated (§8 R0a, §9 clause 2).
* **Repo size:** `.git` 24.6 MB; all tracked assets under `crates/boyko_app/assets/` total 1.07 MB.
  No `.gitattributes` — **Git LFS is not configured**. No `LICENSE` file at the repo root.
* **Content today:** the VB fixtures render five instances of one `uv_sphere(radius, 28 stacks,
  40 slices)` at 512×512 (`sv0_scene/mod.rs:56-69`, `:162`). Twenty-four golden pins exist; two
  carry `sha256_hwrt = "PENDING"`.
* **Shaders:** 16 committed VB `.spv` are perturbed by a `vb_id` re-encode; 10 had a re-DXC gate
  when this record was probed. **Superseded 2026-07-27 by `598f4ff`: all 16 are gated** (§2). Left
  visible rather than overwritten, since §11's whole contract is that it is a dated record no gate
  reads — a fact of the box at a date, not a live number.

### 11.1 Amendment record

Frozen values in [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) change **only** by a
dated entry here, in a new plan revision, never by an in-place edit. The recorded sha256 in
`VG-R0-REFERENCE-RIG.toml` is updated in the same commit, deliberately and visibly.

| Date | Revision | Value | From → To | Why |
|---|---|---|---|---|
| — | — | — | — | *No amendment yet. The thresholds file is at its authoring state.* |

**Findings that pre-authorize a fallback** (per `[census].cross_run_spread_fallback` and
`[k1_instrument].on_not_converged_fire_direction`) are also entered here, by name and date, **before** the fallback
is used. A fallback adopted without an entry is the "widen the gate to make the run pass" move that
§7 clause 2 forbids.

---

## 12. Appendix — verified file:line anchors

⚠️ **Rev 7 WITHDRAWS the blanket claim that stood here.** It read *"Every line below was opened
or grepped while writing this revision"* and was **false in four consecutive revisions** — Rev 2
carried a ~10-line drift through the whole Timing block; Rev 4 re-derived the appendix and left the
body stale; Rev 6 re-derived the ingest block in the body and left the appendix stale, in the
opposite direction; and `targets.rs`'s anchors were ~56 lines off in *both* at once. Four rounds of
asserting verification, four rounds of being wrong, every time caught by an adversarial pass rather
than by anything mechanical.

**These anchors are machine-checked in part as of Rev 9, and the part matters more than the fact.**
⚠️ The printed denominator is also slightly generous: the `~` waiver is appended by textual match,
so a `:N` that is not a citation at all can absorb one, which inflates "201 anchors" rather than the
stale count. Read the 201 as an upper bound on what is bound, never as a count of verified claims.
The document is in `internal_docs_anchors.rs`'s `GATED_DOCS` and the gate is green: 123 path
mentions, 0 dead; 201 anchors, 0 stale. ⚠️ But it prints its own decomposition and the decomposition
is the honest reading — **102 of the 201 anchors carry the `~` waiver**, which asserts only that the
line number exists inside the cited file. `check_anchor` returns at the waiver branch *before* the
shape test, so a waived anchor that names the wrong line still passes. Half this appendix is
therefore bounds-checked, not verified.

That is not the gate under-performing: it models an anchor as pointing at a **definition**, and this
document cites **evidence lines** — a usage flag, an enum variant, a comment asserting the very fact
being cited. 93 of the 99 anchors it first called stale were that mismatch rather than rot, and
re-pointing them at definitions would move the citations away from the evidence they cite. What
membership does buy is the class that actually rots — a cited file that disappears or shrinks — and
it caught three dead paths on its first run.
⚠️ **This paragraph said, in the present indicative and thirteen lines below the sentence above, that the gate covers "the three navigation documents", that adding this plan "was attempted and reverted", and that converting the citations is still a pending follow-up.** All three were true history and false as current state: the round trip is real (added, removed, added again) and the conversion landed. The stale wording survived a repair that fixed four other texts and missed the one inside the section the repair was about — and it told the reader to trust nothing below it, so it weakened no gate but contradicted the section's own opening. **The live limit is not membership, it is the waiver:** 102 of 201 anchors carry `~` and assert only that the line number exists in the cited file.

**Ingest / mesh:** `crates/boyko_render/src/loaders/obj.rs:13` (default vertex colour), `:55`
(`ObjMeshLoader`), `:60~` (`EXTENSIONS = &["obj"]`), `:94~-96` (dedup + `generate_tangents`) ·
`crates/boyko_render/src/mesh.rs:81~-100` (`Vertex`), `:103-104` (`VERTEX_STRIDE == 64`, static
assert), `:124` (`U16_INDEX_VERTEX_LIMIT`), `:137-186` (`MeshGpu`), `:169~` (`geometry_slot`),
`:193~` (`type Cpu = MeshData`), `:237~` (single `LoaderEntry`) ·
`crates/boyko_render/src/mesh_assets.rs:238~-243` (`build_mesh_gpu` signature), `:259~-263` (index
width), `:290~` (**stale** `VB_IMPLEMENTED == false` comment), `:295~-305`
(`MemoryLocation::HostVisibleCoherent`), `:529~` (`register_mesh` passes `None`), `:619~-631`
(`MeshAssetsVbExt`), `:647~` (`register_mesh_vb` trait decl; impl at `:669`) ·
`crates/boyko_render/src/gpu_upload.rs:41~-61` (`GpuUpload for MeshGpu`; `type Aux =
MeshGeometryTableSlot` at `:50~`; **the threaded call at `:59`**).

**Geometry table:** `crates/boyko_render/src/mesh_geometry_table.rs:17~-27` (module doc),
`:66` (`VB_GEOMETRY_RESERVED_SLOT`), `:82-93` (`MeshGeometryMeta`), `:97` (16 B stride),
`:116-118` (`tri_count`), `:140-142` (`mesh_buffer_usage`), `:400~` (**stale** comment), `:413~`
(`MeshGeometryTableSlot`) · `crates/boyko_rhi_vulkan/src/geometry_bindless.rs:61~`
(`MESH_GEOMETRY_TABLE_CAPACITY = 4096`), `:43~` (**stale** comment).

**Path resolution:** `crates/boyko_render/src/render_path_config.rs:25~` (**stale** module-doc
sentence), **`:128~` (`const VB_IMPLEMENTED: bool = true;`)**, `:517~` (`vb_geometry_table` field),
`:890~-892` (the predicate).

**Encode / decode:** `crates/boyko_rhi_vulkan/shaders/vb_geom_fetch.hlsli:516`
(`vb_geom_fetch` signature), **`:521` (`uint local_tri = raw_prim_id % tri_count;`)** ·
[`vb_pack.hlsli`](../crates/boyko_rhi_vulkan/shaders/vb_pack.hlsli):19 (`VB_ID_SENTINEL`) · [`vb_raster.vs.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_raster.vs.hlsl):63 (flat `IID` interpolant), `:82`
(the export) · **[`vb_raster.fs.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_raster.fs.hlsl):24-25 (`uint2(input.instance_id, raw_prim_id)`)** ·
includers: [`vb_geo.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_geo.comp.hlsl):117/`:118`, [`vb_resolve.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl):84/`:85`,
[`vb_shade.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl):89/`:90`, [`vb_shade_split.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_shade_split.comp.hlsl):136/`:137`,
[`vb_classify_count.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_classify_count.comp.hlsl):29, [`vb_classify_scatter.comp.hlsl`](../crates/boyko_rhi_vulkan/shaders/vb_classify_scatter.comp.hlsl):24 ·
`crates/boyko_rhi_vulkan/tests/vb_lit_producer_spv_sync.rs`'s `VB_LIT_PRODUCER_ROWS` (ten gated
rows) · `crates/boyko_rhi_vulkan/tests/vb_raster_geo_classify_spv_sync.rs`'s
`VB_RASTER_GEO_CLASSIFY_ROWS` (the complementary six, landed `598f4ff`).

**Targets / readback:** `crates/boyko_rhi_vulkan/src/present/targets.rs:851~-856` (`VbTargets`),
**`:868~` (`COLOR_ATTACHMENT | SAMPLED` — no `TRANSFER_SRC`)** ·
`crates/boyko_rhi/src/encoder.rs:115` (`copy_image_to_buffer`) ·
`crates/boyko_rhi_vulkan/src/rhi_impl/encoder.rs:1031` (impl) ·
`crates/boyko_rhi_vulkan/src/present/frame_driver.rs:750~` (no depth readback) ·
`crates/boyko_app/src/host_dump.rs:1~-10`, `:67~` (`BOYKO_HOST_DUMP`).

**Timing — RE-VERIFIED at Rev 3; Rev 1 and Rev 2 both carried a consistent ~10-line drift here,
i.e. anchors read from a pre-VB-P1e-H0 tree.** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs`:
`:188~-194` (why collectors are separate — and the `PASS_COUNT` note), `:229~` (**`VbShade = 2`**,
not `:219~`), **`:242` (`VB_PASS_COUNT: u32 = 3`, not `:232`)**, `:281~`/`:293~-294` (the pool reset),
**`:344~` (`WAIT_BIT` BLOCKS FOREVER on a pair its recorder never wrote, not `:334~`)** — this one is
cited by §7's non-negotiable implementer trap and by risk R4, so the stale anchor was the most
expensive of the set — `:357` (`Sv0TimedPass`, not `:347~`), **`:381` (`SV0_PASS_COUNT = 1`, not
`:371`)**.

**Harness precedent:** `crates/boyko_app/tests/sv0_deferred_term_bench.rs:20~-51` (ABAB refuted by
its own null control), `:34~` and `:58~-62` (**the ABBA algebra — the model `m_k = μ + τ·armed + γ(fi)
+ β·k + ε` and the cancellation that makes absolute readings unavailable**, §14.2), `:83~-129`
(the quantisation finding), `:297~-299` (**the OS-clamped-extent check**, §5.4), **`:350`
(`SV0_BENCH_SESSIONS = 3`), `:366` (`SV0_SESSION_SPREAD_MAX = 0.10`), `:378`
(`SV0_NULL_CONTROL_MAX_FRACTION = 0.10`)** — Rev 2 cited `:284~` and `:312~` for two of these in one
block and `:350`/`:378` in another; **the `350`/`366`/`378` set is the correct one**, and the
contradiction is direct evidence that the older block was never re-verified ·
`crates/boyko_render/src/ui/mod.rs:87` (`FRAMES_IN_FLIGHT = 2`) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch`) ·
`crates/boyko_rhi_vulkan/src/window.rs:252` (`Window::open`), `:310~` (`AdjustWindowRectEx`),
`:342~-352` (`BOYKO_WIN_HIDDEN` — hidden, but still created at the requested size).

> **§12's opening sentence — *"Every line below was opened or grepped while writing this
> revision"* — was FALSE in Rev 2**, systematically, across the whole Timing block. It is the
> claim this project's own standing lesson exists against (*report line numbers are lower bounds;
> grep the pattern*). Every anchor in this section was re-derived at Rev 3 by grep; the ones that
> moved are called out inline above rather than silently corrected, because a silent correction
> would leave no evidence that the blanket claim had been wrong.

**Oracles / fixtures:** `crates/boyko_app/tests/sv0_oracle/mod.rs:182-208` (`OracleVertex`,
`CoveredPixel`), `:211-256` (`Coverage`, `covered_count` at `:253`), `:279-287` (`rasterize`),
`:765-798` (`ChangedPixels`, `changed_covered_pixels`) · `crates/boyko_app/tests/sv0_scene/mod.rs:56-69`
(mesh row constants), `:149~-162` (camera + `DUMP_EXTENT`), `:223` (`uv_sphere`) ·
`crates/boyko_app/tests/sv0_adequacy.rs:231~-232`, `:514~-515` (the shared-spawn inseparability test).

**Rev 2/Rev 3 additions, verified this session:**
`crates/boyko_rhi_vulkan/src/present/targets.rs:851~-856` (`VbTargets` doc — the ring is **one
`R32G32_UINT` texel per pixel**, which is what caps §5.4's statistic (1~) at 1), **`:866~`
(`format: Format::R32G32Uint` — Rev 2 cited `:865~`, which is `depth: 1`)**, `:868~` (the usage bits,
correct) · `crates/boyko_app/tests/sv0_scene/mod.rs:162` (`DUMP_EXTENT = 512`) ·
`crates/boyko_app/tests/sv0_oracle/mod.rs:279-287` (**`rasterize` takes ONE indexed mesh and
`instances: &[[f32; 3]]` — translation-only**, which is why R0c gate (c) is scoped to the procedural
fixture and cannot reach the corpus at any ladder rung) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch` — the source of the report-only
`submitted_per_covered_pixel`) · `crates/boyko_rhi_vulkan/shaders/vb_pack.hlsli:15-16`, `:19`
(`VB_ID_SENTINEL` marks a pixel the mesh raster leg never covered — the census's denominator is
mesh-covered pixels, not all pixels) ·
[`docs/VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) (hashed, never edited) ·
[`docs/VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) (unhashed, `PENDING`-gated, blocks R0b and R1).

**Corpus convention:** `crates/boyko_app/assets/pbr_fixtures/README.md:1-6` ·
`.gitignore` (`/assets/materials/*` + the `!README.md` escape) ·
[`PINS.toml`](../goldens/PINS.toml):15~ (the `PENDING` sentinel rule),
[`PINS.toml`](../goldens/PINS.toml):372~ and [`PINS.toml`](../goldens/PINS.toml):417~ (the two
unblessed hwrt legs — ⚠️ through Rev 8 these carried line numbers nine lines low AND in the bare
continuation form, which bound them to the preceding `README.md` link, so the gate resolved them
against a 44-line file. The stale numbers are described rather than quoted here, because a dead
anchor written in citation form is a live citation to any gate that reads the document. Verified by
grep: `sha256_hwrt = "PENDING"` sits at 372 and 417) · `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:196~-204` (the skip shape).

---

## 13. Open questions — VALUES / SCOPE only

Performance and architecture forks are decided with numbers in this project; the format choice
(§3.3), the census instrument (§5.3), the corpus shape (§4.2), the census resolution ladder and
K1's threshold (§5.4) are decided above and are not listed here.

⚠️ **Rev 7 opened this section with "every question below has a field waiting for it in
[`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml)", and that universal was false** — Q2, Q4 and Q6
had no field, and Q6 was the disposition of the outcome §9 itself calls the likely one. A preamble
asserting completeness it does not have is the highest-risk line in a document; the questions are
now split by whether they block anything.

**Blocking — each has a `PENDING` field and a `[gating]` row.** The rows are
`[gating].r0a_blocked_by`, `[gating].r0b_blocked_by`, `[gating].r0c_blocked_by`,
`[gating].r0d_blocked_by` and `[gating].r1_blocked_by` — five rows for five rungs, three of them
deliberately empty, so "nothing blocks this rung" is a recorded decision rather than a missing
entry. Two of the rows this table used to carry were English sentences rather than resolvable
`table.field` paths, which the `PENDING`-sentinel checker they exist to drive cannot resolve; every
row is now a list of paths.

⚠️ **What is and is not mechanical, because Rev 8 asserted the wrong one in the present indicative.**
Rev 8 wrote that "the named rung refuses to run while the field is unanswered" — and **no gate part
anywhere read a `[gating]` row**, so the sentence described a mechanism that did not exist. Of the
two non-empty rows: **R0b's is now asserted** by its own gate part (a0), so that half is true; **R1's
is not**, because R1 is outside this document and no rung here can assert it. The row is a recorded
requirement on whoever writes R1, and calling it anything stronger would repeat the defect.

1. **Corpus provenance and licence.** Who selects and licenses the high-poly assets, and is a
   fetched-and-gitignored payload with pinned hashes acceptable as the permanent arrangement?
   → `corpus.arrangement`, **blocks R0b**, which cannot author `CORPUS.toml` without it. This is the
   only early block in the ladder.
2. **If K1 comes back UNDECIDED, what happens?** R0 can refute K1 cheaply and soundly and **cannot
   fire it** — the upper-bound instrument is unsolved, not merely unscheduled (§5.6). So
   `D_est < [k1].d_est_min` leaves the campaign's premise *untested* rather than refuted, and with
   `[k1_instrument].d_est_ceiling` at 4.0 a plausible corpus puts the estimate in a band where that
   is the likely outcome. Proceed to R1 on an unadjudicated premise, change the target content
   class, or fund the instrument as its own campaign?
   → `k1_outcome.undecided_disposition`, **blocks R1**. Like every other pre-registration here it
   must be answered before the number exists; afterwards it is answered by someone who has seen it.
   ⚠️ This field is **new at Rev 8**. Through Rev 7 this question pointed at a claim file that had
   no field for it and a `[gating]` table with no row, so the one outcome R0 is most likely to
   produce had no sentinel and blocked nothing — the enforcement predicate was vacuously true for
   every input. That is the same structural omission D3 named, in the table built to prevent it.

**Advisory — no field, no gate, and that is deliberate: they shape work but block no rung of R0:**

3. **Third-party dependency policy for the importer.** §3.3 decides *glTF, in-house*. If the owner
   will accept a third-party glTF/JSON crate, the decoder shrinks substantially — but the
   workspace's demonstrated posture is fully in-house (raw-FFI Vulkan, in-house PNG/zlib/DEFLATE).
   The same question recurs, far more sharply, for the offline builder at R4/R5.
4. **Bless bandwidth.** How many byte-moving rungs per week can the owner actually bless? R0 moves
   no pin, but two hwrt legs are already `PENDING` (§9 clause 4), and that number caps the width of
   every rung after R2b.

**Moved to §14 at Rev 8:** the claim itself (*"if K2 fires, what replaces the goal?"*) and the
quality target. Both are the right-hand side of an inequality R0 no longer evaluates, and both are
answered at the rung that lands an arm — where the measurand they are compared against exists. A
`PENDING` sentinel that blocks no rung is not a gate, which is what those two fields had become.

---

## 14. Deferred — the decidability apparatus and the ONE gate

**Status: SPECIFICATION, not a rung of R0.** Nothing here is frozen, nothing here is gated, and no
value here is pre-registered. That is the point: Rev 2–Rev 7 froze this apparatus in
[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) and
[`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) while the rung that would read it did not exist,
and every revision was told by an adversarial pass that it had overclaimed. **A frozen value with no
reader is worse than prose** — it manufactures the appearance of pre-registration while binding
nothing.

### 14.1 When this becomes a rung

**At the first rung that lands an arm** — a meshlet/cluster path that can be switched on and off
against the shipped VB path. Not before, and the reason is arithmetic rather than scheduling: the
decidability floor is a *resolvable delta*, the frozen table named our side's denominator as the
**armed paired delta**, and a delta needs two configurations to sit between. R0 lands no meshlet, no
cluster and no LOD.

⚠️ **Rev 10 names the rung and withdraws "deferring costs the campaign nothing", which was false.**
Against the research ladder this document is bound to (§0), "the first rung that lands an arm"
resolves uniquely to **R6** — R5 is dark infra with the `Option` staying `None`, R6 is where the
meshlet cull is armed. So the floor arrives at R6, and **exactly one** downstream gate row cites
it: the research ladder gives **R2** a gate reading *"measured Δ on R0 corpus, decidable by R0's
floor"*, and the deferral orphans that row until R6. **This paragraph owns the count. §9.1 cites it
and does not restate it**, which is the only arrangement with a clean record here — a number stated
in two texts has disagreed with itself every time it has been stated in two texts.

⚠️ **Rev 10 wrote "R2, R2b, R3, R4 and R5 therefore each carry a gate citing a measurand that will
not exist until R6 — five orphaned gate rows", and the "therefore" does not follow.** Read the
ladder's gate column row by row: R2b is `*_spv_sync` tests, R3 a measured pass-1 hit rate produced
at R3, R4 a triangles-at-error curve produced at R4, R5 byte-identical goldens — a byte comparison
needs no delta at all. Only R2's gate cites R0's floor. Overstating a deferral cost fivefold is the
same overclaiming this revision series exists to stop, committed inside the repair for it.

⚠️ **And Rev 11's repair of that was itself both recorded shapes at once, which is why the rule
below is stated as a rule rather than as another correction.** It fixed this paragraph and left
§9.1 saying five (a fix landing in N−1 of N texts), and it fixed this paragraph by *appending* a
denial to the erroneous clause instead of rewriting it, so one sentence carried the count and its
negation across an em-dash. It further volunteered that "R2 **is** an arm — per-instance GPU cull,
on or off". **That claim is withdrawn**: this section's criterion is a *meshlet/cluster* path
switched against the shipped VB path, per-instance cull is neither, and had the claim held the
orphan count would be **zero**, not one — the volunteered fact would have undone the subtraction it
was appended to. The governing rule, and its evidence is that every repair which only *subtracted* has
survived review while the three that volunteered a positive claim — Rev 8's conditioned `(b′)`, Rev
11's "cheapest remaining tuning lever", Rev 11's "R2 is an arm" — were each refuted on the
volunteered half: **a repair is itself a claim and inherits the full burden of the claim it
replaces.** A repair of a stated fact is executed
as a grep over every text stating it, all N fixed in one act; and a repair may subtract freely, but
every positive claim it volunteers ships with its own substitution — and, if it is a gate clause,
its own isolating red mutation.

Repairing R2's row is out of this document's scope
(§0 binds R1–R8 to the research document), so it is named as the first thing the R1 author
inherits. And the thing that actually carries the
P0's weight — party separation, since §13's owner calls are not made by whoever runs the harness —
is unchanged by moving it later.

### 14.2 What must be frozen, and in what shape

* **The claim**, in one of two modes. `nanite_relative` — a fractional speedup on the bracketed pass
  chain, live only if the reference is achievable. `absolute` — a target in milliseconds for the
  same chain at the decision resolution, on a named corpus at a named quality target. Both close the
  same inequality; only the right-hand side's provenance differs, which is why K2 firing does not
  leave the campaign without a falsifiability condition.
* **Scope.** The claim is about the **bracketed pass chain**, never a frame: a frame also contains
  CSM, SDF, DDGI, post/AA, present and all CPU time, none of which this campaign touches and none of
  which the harness measures. Rev 2 compared a per-pass floor to a `frame_total` claim with no
  composition rule stated anywhere, which made the gate not evaluable.
* **Chain-floor composition.** The chain total's floor is measured on **one bracket spanning the
  chain**, never composed arithmetically from per-pass floors — the passes share occupancy, caches
  and a queue, so composition assumes an independence they do not have. This rule was sound in the
  frozen file and violated one table over; see P0-3.
* **The two denominators, which ARE the gate**, verified against the sibling harness's own
  arithmetic: the null control is gated against the **armed paired delta** — `sv0_deferred_term_bench.rs`
  fired at 33%, −2048 ns against a 6144 ns signal, and *that* failure is what produced the ABBA
  redesign — and the cross-session spread against the **paired delta**. Rev 2 transferred both
  literals while silently changing what they divide, a ~20× weakening under which the precedent's
  own red event would have passed. **A literal transferred without its denominator is not the same
  gate.**
* **Joint floor.** `our_floor + reference_floor`, summed rather than combined in quadrature, because
  quadrature assumes two independent draws from one noise process and a systematic capture bias
  between two engines is not that. Summing is conservative in the direction that makes our own claim
  harder to close.
* **The absolute-mode gate, two-sided.** `c < m` **and** `floor < |m − c|` — the claim must be an
  *improvement* and a *resolvable* one. Rev 5's one-sided form was symmetric and passed for its own
  named red mutation.
* **The ordering rule**, attached per mode to **whichever rung measures the floor for that mode**,
  with the claim pinned into that rung's own MEASURED-literal commit so the comparing rung compares
  against something frozen in the same act as the measurement.

### 14.2b K3 — the kill this apparatus carries, named because two texts say it moved here

⚠️ **§0.2 and §9 both state that K3 "moved to §14", and through Rev 9 the string `K3` appeared
nowhere in this section.** A kill said to have been relocated to a section that does not name it has
not been relocated; it has been dropped with a forwarding address.

**K3 — the undecidable harness.** The instrument cannot resolve the frozen claim. Two outcomes,
which must not be conflated: **(3a) the instrument misbehaves** — the null control is over budget, a
pass sits at the lattice, the cross-session spread is out of band. The ladder does not proceed until
the instrument is fixed, and nothing is learned about the campaign either way. **(3b) the instrument
works and the answer is no** — the inequality reds because the floor is real, measured, and larger
than the claim. The instrument is not broken, so fixing it is not the move: the owner either lowers
the claim to something this pair of instruments can resolve, which may make the campaign not worth
running, or invests in a better instrument. That is an owner VALUES call and it is the outcome the
whole decidability apparatus exists to surface early.

K3 is **not** an R0 criterion (§9): R0 builds no harness and measures no delta, so there is nothing
at that rung for it to be true or false about. It becomes live at §14.1's rung, together with
everything else here.

### 14.3 What §7 already settles, and it is binding here

§7's harness contract — ABBA counterbalancing with the order-bias residual reported, a null control
with a pre-registered maximum, the counter quantum measured by tick GCD, the `max()` spread gate
**with** its distinct-tick evidence licence and its non-waivable companion assertion, and the
written-pair bitmask that turns a `WAIT_BIT` deadlock into a red assertion — is **not deferred**. It
is the contract this rung is built against, every quotation in it was checked exact by the Rev 7
review, and it should be re-read before any of §14.2 is frozen.

### 14.4 The eight P0s the previous attempt shipped — requirements, not history

Rev 7's adversarial review (six lenses, each required to write the inequality with units, substitute
degenerate cases, and re-derive every named red mutation; then an independent refutation pass)
returned 35 findings, 34 surviving. These eight blocked approval. **Whoever authors this rung
discharges them explicitly.** They are the cheapest eight lessons available, and every one was found
by arithmetic rather than by reading.

1. **One floor, one symbol, and the gate names it.** Rev 7 added a lattice-floor rule with a comment
   stating it existed because *"without it the gate collapses at `s → 0` to `c < m`"*. The gate rule
   named a different symbol, so the operative floor stayed the superseded spread-only product in
   **both** documents. Define the floor once; make the gate name what is actually computed.
2. **§8 and the frozen file must not restate each other.** §8 quoted, in the present tense, a floor
   source the same revision had deleted. Where a rule exists in both places, one cites and the other
   defines.
3. **The reference floor obeys the chain-floor rule too.** §6.3 derived it from the spread of
   **per-pass** medians and then used it as a chain floor — the exact composition the scope rule
   forbids, inside the same inequality whose other half obeys it. Constructed counterexample:
   `A = (1.0, 1.4, 1.0)`, `B = (1.4, 1.0, 1.0)` ms gives per-pass peak-to-peak spreads of
   **0.40 / 0.40 / 0.00** and a chain-total spread of **0.00** — the totals are 3.4 both sessions.
   ⚠️ Rev 9 wrote `B = (1.4, 1.0, 1.4)`, whose totals are 3.4 and 3.8, so the chain spread is 0.40
   and the example demonstrated the opposite of its point. Re-derived here rather than re-worded. Also state the aggregation over passes (max? mean?) — "the spread of that table"
   is not a single number.
4. **Give the floor a unit and a denominator.** No text in three files assigned one. The reference
   floor is relative to each reference pass's own median, the claim is relative to our chain total,
   and a sum of three fractions with three denominators is not an inequality. Express both floors as
   a fraction of the **same** denominator — the bracketed chain total at the decision resolution —
   and state the conversion from a paired-delta-relative spread to a chain-relative resolution
   explicitly.
5. **The post-fill edit window closes on every branch, not one.** Only the `nanite_relative` rung
   asserted that the claim file still equals the pinned literal. Worked example on the other branch:
   pin `c = 4.0`, measure `m = 3.0`, `s = 0.10` → the improvement conjunct reds; edit the unhashed
   claim to `2.5` → distance `0.50` > floor `0.30` → **green**. The mode-consistency check does not
   read the claim's numeric value, and the file is deliberately unhashed.
6. **`max()` ships with its precondition or not at all.** §7 states it: the widening, the
   distinct-tick evidence floor, and the separate non-waivable assertion — *all three or none*. The
   rung shipped one. With this box's measured lattice and a 100 ns paired delta the bare form widens
   a 0.10 spread gate to 2.56; the guarded form reds. Bound the divisor away from zero too — this
   engine records a timing bracket that is "ALWAYS written (near-zero ns then)".
7. **Every pre-registered value is read by a gate part.** The `[pre_registered]` table was created so
   two decision-bearing thresholds had a file to be registered in, and no rung ever read any of them
   — so the mutation the plan listed as DEMONSTRATED, *"halve the sample count → the CI widens past
   the pre-registered bound → red"*, still named a right-hand side that did not exist.
   [`tests/vg_symbol_reachability.rs`](../tests/vg_symbol_reachability.rs) now catches this class
   mechanically; run it before freezing anything.
8. **Re-derive every red mutation against the arithmetic, every time.** A mutation that is only
   argued does not count. The specimen, kept because it shipped in **three** consecutive revisions:
   *"set the claim below the measured floor → the gate reds"*. Plug it in — `c < s·m` gives
   `distance = m − c > m(1−s)` and `floor = s·m`, so the gate asks `s·m < m(1−s)` ⟺ **`s < 0.5`**,
   true for every `s ≤ 0.25` the ceiling permitted, and it passes the improvement conjunct too.
   Green under both the old gate and the new one, for three revisions, because nobody did the
   substitution.

### 14.5 The two owner questions that move here

Both are VALUES calls and both must be answered **before** the number they concern exists —
otherwise they are answered by whoever has already seen it.

* **If K2 fires, what replaces the goal?** *"Faster than Nanite"* becomes *"N ms at quality Q on
  corpus C"*, and the owner sets N, Q and C. One tension this document accepts knowingly rather than
  hides: pre-registration asks the party with **no** measurement to judge whether a target is sane.
  That applies equally to a relative claim, which likewise carries no sanity band.
* **Quality target.** What pixel-error budget counts as "equal quality" — our equivalent of a pinned
  `MaxPixelsPerEdge` — and is the owner the arbiter by visual eval, or do we bind to a metric? The
  standing lesson that image statistics have already misled this project twice argues against a
  metric.

Neither has a field in [`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) any more, deliberately: a
`PENDING` sentinel that blocks no rung is not a gate, and until this becomes a rung there is no rung
to block.
