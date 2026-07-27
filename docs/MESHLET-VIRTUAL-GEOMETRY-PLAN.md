# VG-R0 — "The Ruler": the measurement rung of the virtual-geometry campaign

**Status:** DESIGN, **Rev 7** — **NOT APPROVED, and no code exists.** This document specifies
**only rung R0** of the ladder in [`docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`](MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md)
§4. R1–R8 stay as that document leaves them and are out of scope here. The owner's decision to
build a meshlet / virtual-geometry system is **settled** and is not re-litigated below.

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
| §12 anchors re-derived | **HELD FOR §12 ONLY** — the body kept the stale ones, including the `:334`→`:344` anchor §12 itself called the most expensive of the set. Fixed at Rev 4 |

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
grepped"* is **withdrawn**, having been false four revisions running. Gating the plan's anchors
mechanically was attempted and **reverted with the reason recorded**: the plan cites bare basenames
in prose while the gate binds to resolvable path links, giving 83 "stale" of 146 dominated by
misbindings. Converting the citations is the named follow-up; until then the document states its
numbers are unchecked rather than claiming they were verified.

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
  **not hashed** and is gated by the `PENDING`-sentinel rule `goldens/PINS.toml:15` already defines.
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
| R0 has *"no render change whatsoever … byte-identical goldens"* | **Half true.** The density census cannot read the visibility buffer without widening the `vb_id` ring's image usage — `targets.rs:868` declares `COLOR_ATTACHMENT \| SAMPLED`, no `TRANSFER_SRC`. R0c therefore makes a **device-object** change. Frame content is unaffected and the byte-identity of all VB pins is that rung's gate, but "no render change whatsoever" is withdrawn. |
| *(orchestrator prescription)* `vb_geom_fetch.hlsli` *"is included by EIGHT shaders"* | **REFUTED.** `grep -rn 'include "vb_geom_fetch'` over `crates/boyko_rhi_vulkan/shaders/` returns exactly **four**: `vb_geo.comp.hlsl:118`, `vb_resolve.comp.hlsl:85`, `vb_shade.comp.hlsl:90`, `vb_shade_split.comp.hlsl:137`. The research doc's own corrected count (four includers, **eight** sources touching the *encoding*) is the right one — §2. |
| Research §4 item 1 includes *"plus the beginnings of a bake artifact format"* | **SCOPED OUT, on the record.** Rev 1 and Rev 2 dropped it silently while stating the other two corrections explicitly. A bake format is an output of the offline builder (research ladder R4/R5) and has no consumer at R0: nothing in R0 produces clusters, a DAG or simplified LODs, so a format authored now would be authored against no data. It returns with its first producer. The research doc's stronger point — *"There is no bake stage. This is the actual first blocker and no survey named it"* — stands and is why §3 exists. |

---

## 0. What R0 is, and the three ways it kills the campaign

**R0 = a high-poly ingest path + a licence-clean corpus + a screen-space triangle-density census +
a Nanite reference capture + a decidability statement.** No meshlet, no cluster, no DAG, no shader
that did not exist before.

**The ONE gate — restated so that both of its sides can fail:**

> **`joint_floor < claim`**, both sides scoped to the **bracketed VB pass chain**
> ([`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) `[scope]`), where `joint_floor` is
> measured by R0e — combined, in `nanite_relative` mode, with the reference capture's own
> cross-session floor per `[decidability].joint_floor_rule` — and `claim` is the value the owner
> wrote into [`VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) **before R0e was allowed to run**,
> together with the reproducible per-pass cost table (**named GPU, named scene, named resolution,
> named error target**) the left-hand side is measured against.

### 0.1 The P0's mechanism is ordering, and Rev 2's hash was not it

The synthesis' original wording — *"a decidability floor smaller than the delta we intend to
claim"* — is a two-sided inequality with only one side specified. **"The delta we intend to claim"
is not a measurement; it is a choice, and left unpinned it is a choice made after seeing the
floor.** An author who measures a 12% floor and then declares a 15% intended delta has closed the
gate without moving anything, and no assertion anywhere fires.

**Rev 2's answer was a committed claim file with its sha256 recorded by R0a — and it did not
work.** Every field on the right-hand side was the literal `PENDING`, and Rev 2's own `[gating]`
table scheduled the fill for R0f, *after* R0e measures the floor. What was frozen was a schema.
The named red mutation — *"raise the claim after the freeze → the hash assertion reds"* — was
undemonstrable, because the first write of a real value is a legitimate, plan-sanctioned edit that
necessarily comes with re-recording the hash, and **no test can distinguish that edit from the
cheat.** Worse, `corpus.arrangement` had to be filled to run R0b, so the hash recorded at R0a was
*guaranteed* to break before the first rung that asserted it — and once "re-record the hash" is a
routine event, any re-record can carry a simultaneous edit to a K1 threshold with every rung green.

**Rev 3's mechanism is that the claim must exist before the floor does:**

* The claim fields block **R0e — the rung that measures the floor** — not R0f, the rung that
  compares them. `r0e_blocked_by = ["claim.mode", "claim.nanite_relative_chain_delta or
  claim.absolute_chain_ms"]`. R0e's test refuses to run while the value it will be compared against
  is still `PENDING`, using the sentinel discipline `goldens/PINS.toml:15` already defines (a
  `PENDING` pin makes the checker **exit 2** rather than pass).
* ⚠️ **RETRACTED at Rev 4.** Rev 3 wrote here: *"This does not depend on anyone noticing an edit."*
  **It does.** The ordering rule constrains **commits, not knowledge.** R0e is an `#[ignore]`d
  windowed GPU bench in the sibling's shape; the floor is *observed* by running the harness and
  reading its stdout, while the claim blocks the *assertion*, which runs later on transcribed
  literals. Nothing prevents: run the harness in a dirty tree → read the floor → `git checkout .` →
  fill the claim to fit → commit → re-run and transcribe. Commit order is preserved; knowledge order
  is not. Against a peek-then-fill author the mechanism is **weaker than nothing**, because it
  manufactures the appearance of a guarantee. Rev 4 states the limit instead of asserting it away.
* **What actually carries the weight is party separation, and the plan should have said so.** §13 Q1
  is an owner VALUES call, and the owner is **not** the party who runs the harness. The ordering
  rule is a *record* of pre-registration, not an enforcement of it. A mechanism honestly described
  as partial is worth more than one described as complete — that is the whole lesson of the three
  failed attempts above.
* **A stronger mechanism exists in this repo and Rev 4 adopts it: gate the instrument's OUTPUT, not
  the comparison.** `sv0_deferred_term_bench.rs:410-413` uses `f64::NAN` as the UNMEASURED sentinel
  *"so a forgotten transcription cannot produce a passing gate."* The analogue: **the R0e harness
  refuses to emit the floor at all while the claim is `PENDING`.** That constrains knowledge rather
  than commit order, which is the property the P0 actually needs.
* ⚠️ **Second Rev 3 hole, also open: nothing forbids editing the claim AFTER the fill.** R0e asserts
  only `!= "PENDING"`; R0f reads the file from disk at R0f time. So `fill 0.05` → R0e green → floor
  measures 0.12 → `edit to 0.30` → R0e *still* green (still not `PENDING`) → R0f closes. Rev 2's
  hash covered this window and Rev 3 removed it without replacement. **The correct axis is phase,
  not file:** the claim is mutable until filled and immutable forever after. Rev 4's fix is that
  R0e's own MEASURED-literal commit **pins the filled claim value alongside the floor**, so R0f
  compares against something frozen in the same act as the measurement rather than against a file.
* **The claim file is deliberately not hashed.** Its fields are *required* to change exactly once.
  Hashing a file whose schedule requires it to change is what destroyed Rev 2's tripwire.
* **The thresholds that must never change are hashed, in their own file.**
  [`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) carries K1's instrument and rule,
  the resolution ladder, the harness denominators and the scope rules — all authored before any
  measurement is reachable. R0a records its sha256.
  ⚠️ **Rev 7: Rev 6 enumerated R0c/R0d/R0e/R0f as the re-asserting rungs — and Rev 5 had already
  measured that set to be exactly the wrong one.** Every member is a skipped or `#[ignore]`d
  GPU/corpus test on a box whose CI never exercises the GPU path. Rev 6 added
  `[hash_assertion].must_run_in_plain_workspace_test = true` to fix it, then gave it **no rung and
  no test file**, so the field described an intention nobody implements. Rev 4 avoided Rev 2's
  *guaranteed to break*; Rev 6 shipped its mirror image, **guaranteed not to fire**.
  **The tripwire lives in `crates/boyko_render/tests/vg_thresholds_freeze.rs`** — a file whose only
  job is to re-hash `VG-CAMPAIGN-THRESHOLDS.toml` against the value R0a recorded. No GPU, no `dxc`,
  no corpus, so it runs under a bare `cargo test --workspace`. R0a lands it; the GPU rungs may
  re-assert too, but none of them is the mechanism.
  ⚠️ Standing hazard when checking this: `cargo check --all-targets` at the repo root is
  **vacuum-green** on this virtual manifest, so "it is in the workspace" is not evidence that it
  runs — R0a's gate must show the test executing.
  Because that file has **no** legitimate reason to change, a broken hash there is unambiguous.
* **Red mutation, now demonstrable on both sides:** raise the floor above the claim → the
  inequality assertion reds; edit any threshold in the thresholds file → the hash assertion reds in
  four rungs; run R0e with the claim still `PENDING` → R0e refuses.

**What ordering costs.** R0e cannot run until the owner answers §13 Q1. That is not a schedule
defect, it is the point — and it costs little, because R0e is rung five of six and R0a–R0d are
unblocked by it. `corpus.arrangement` (Q3) blocks R0b and is the only early one.

**The three kills, each a falsifiable test rather than a worry:**

| # | Kill | Test | Disposition if it fires |
|---|---|---|---|
| **K1** | **No content, no mechanism.** The corpus never approaches ~1 triangle/pixel, so cluster LOD has no mechanism of action on our content. | **Split by direction at Rev 5, because the cheap census can only settle it one way.** `D_est ≥ 1.0` at the decision resolution **refutes** K1 outright (a lower bound proves density). **Firing** K1 is UNREACHABLE at R0 (`[k1].k1_fire_at_r0 = false`): the upper-bound instrument is mis-sited and probably inert, and is recorded UNSOLVED rather than scheduled. §5.6, §9 clause 1. | **Campaign refuted** only when K1 *fires*. Not "descope" — the premise is gone. §9 clause 1. |
| **K2** | **No baseline.** The Nanite reference cannot be produced on this box. | R0a's rig probe (before any engine code) — and the negative is **re-derived by the test**, not declared. §8 R0a. | **Scope restatement**, an owner VALUES call: the goal becomes an **absolute** ms/quality target, and the ladder terminates in **R0f′**, which closes the *same* inequality. §13 Q1, §8 R0f′. |
| **K3** | **Undecidable harness.** The instrument cannot resolve the frozen claim. | R0e's decidability statement, with its null control. | Every future number is arguable. §9 clause 3 — and note this is the failure mode the sibling rung actually hit. |

**Falsification-first ordering.** K2 is the cheapest to test — it needs *zero* engine code and one
operator session — so R0a runs first. K1 needs the corpus and the instrument, so it lands third and
fourth. K3 needs the corpus (cost scales with density), so it lands fifth.

**K2 no longer terminates the ladder, and that is Rev 2's D3 fix.** In Rev 1, `achievable = false`
left R0f unrunnable and the ONE gate unclosed — on the branch §11 measured as *today's reality*. The
left-hand side of the inequality is **entirely ours**: R0e measures it with no reference at all.
Only the right-hand side's provenance changes between modes. So both branches close the same gate,
and the campaign never proceeds to R1 without a falsifiability condition.

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
  `vb_classify_count.comp.hlsl:29`, `vb_classify_scatter.comp.hlsl:24`, `vb_geo.comp.hlsl:117`,
  `vb_resolve.comp.hlsl:84`, `vb_shade.comp.hlsl:89`, `vb_shade_split.comp.hlsl:136`.
* The **encode** side is two more sources: `vb_raster.vs.hlsl:82` exports the flat `instance_id`
  interpolant (`:63`), and `vb_raster.fs.hlsl:25` is literally
  `return uint2(input.instance_id, raw_prim_id);` with `raw_prim_id : SV_PrimitiveID` (`:24`).
* **Eight sources total touch the encoding.** They compile to **sixteen** committed `.spv`
  (`vb_raster.{vs,fs}`, `vb_geo{,_mv}`, `vb_classify_{count,scatter}`, `vb_resolve{,_froxel}`,
  `vb_shade{,_tex,_froxel,_tex_froxel}`, `vb_shade_split{,_tex,_hwrt,_tex_hwrt}`).
* **Only ten of the sixteen have a re-DXC byte-identity gate** — `vb_lit_producer_spv_sync.rs`'s
  `VB_LIT_PRODUCER_ROWS` enumerates exactly those ten. `vb_raster.vs`, `vb_raster.fs`, `vb_geo`,
  `vb_geo_mv`, `vb_classify_count`, `vb_classify_scatter` would drift **silently**.

**The decode side is genuinely one line** — `vb_geom_fetch.hlsli:521` is exactly
`uint local_tri = raw_prim_id % tri_count;`. The **encode** side is not independently reachable: the
G lane is filled by a fixed-function system value, so authoring a meshlet id into it requires a mesh
shader, one draw per meshlet, or a software rasterizer. **The re-encode is downstream of the
raster-path decision, not independent of it.** R0 records this and touches none of it.

---

## 3. Ingest — what exists, and what a high-poly importer must produce

### 3.1 What imports geometry today

**Exactly one mesh loader exists.** `MeshGpu::LOADERS` is a single-entry compile-time table
(`mesh.rs:238`) holding `ObjMeshLoader`, whose `EXTENSIONS` is `&["obj"]` (`loaders/obj.rs:60`). It
decodes to `MeshData { vertices: Vec<Vertex>, indices: Vec<u32> }` and runs `generate_tangents` once
over the whole mesh (`:94-96`). **There is no `.obj` file anywhere in the tree** — the loader has
never been pointed at a committed asset.

### 3.2 The contract an importer must satisfy

The importer's *only* obligation is to produce a `MeshData`. Everything downstream already works:

| Seam | Contract | Anchor |
|---|---|---|
| `Vertex` | `#[repr(C)]`, **64 B** (static-asserted), `position`@0 / `normal`@12 / `color`@24 / `uv`@40 / `tangent`@48 | `mesh.rs:81-104` |
| Index width | `Uint16` iff unique-vertex count ≤ `U16_INDEX_VERTEX_LIMIT`, else `Uint32`; the shader reads the width from `gMeshMeta[].index_width` | `mesh.rs:124`, `mesh_assets.rs:273` |
| Device upload | `build_mesh_gpu(ctx, &vertices, &indices, geometry_table)` | `mesh_assets.rs:252` |
| VB geometry slot | claimed **iff** a live table is threaded; otherwise the record carries `VB_GEOMETRY_RESERVED_SLOT` (`0`) | `mesh.rs:170`, `mesh_geometry_table.rs:66` |
| `gMeshMeta[]` row | `{index_width, vertex_count, index_count}` padded to 16 B; `tri_count = index_count / 3` | `mesh_geometry_table.rs:82-93`, `:116-118` |
| Table capacity | `MESH_GEOMETRY_TABLE_CAPACITY = 4096` slots | `geometry_bindless.rs:62` |

**The streamed path already threads the table.** `impl GpuUpload for MeshGpu` sets
`type Aux = MeshGeometryTableSlot` and calls `build_mesh_gpu(ctx, &cpu.vertices, &cpu.indices,
aux.0.as_mut())` (`gpu_upload.rs:51`, `:59`). So a **loader-decoded** mesh claims a real slot and is
VB-visible. The **host-authored** primitives pass `None` at their own call site
(`mesh_assets.rs:547`), and the explicit VB sibling is `MeshAssetsVbExt::register_mesh_vb`
(`mesh_assets.rs:641`, `:647`), which every VB fixture uses.

> ⚠️ **CORRECTED at Rev 4 — Rev 1 through Rev 3 all stopped one function too early, and the error
> propagated into R0b's headline red mutation (§8).** Passing `None` is **not** the end of the
> story: `backfill_vb_geometry_slots` runs at boot (`runner.rs:787`, after `upload_mesh_assets` and
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
> `render_path_config.rs:130`. The rot was cleared at `792d992`, which found **19** stale sites
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
(`mesh_assets.rs:320` for the vertex buffer; the index buffer follows). Every mesh in this
engine lives in host-visible memory, seeded once and read-only thereafter (`mesh.rs:129`). At 64 B
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
  unusable for this campaign and must be rejected at manifest-authoring time, not at R0f.
* **The same bytes feed both engines.** The Nanite reference (§6) imports the identical `.glb`
  files. If an asset cannot be imported by both, it is not corpus material.
* A `fetch_corpus` script verifies every pinned hash before extraction and refuses on mismatch. The
  **gate that reads it is a Rust test**, not the script — §8 R0b.

---

## 5. The density census — what exists, what must be added

### 5.1 Counters that exist today

* **Submitted triangles, host side.** `DrawBatch { mesh_id, index_count, index_type, base_instance,
  instance_count }` (`mesh_draw.rs:80-98`) is gathered per frame; `index_count / 3 *
  instance_count` is the submitted-triangle count with no new plumbing.
* **Per-pass GPU time, partially.** `VbTimedPass` (`gpu_timing.rs:203`) brackets **three** passes:
  `CullReset` (`:211`), `CullDispatch` (`:214`), `VbShade` (`:229`); `VB_PASS_COUNT = 3` (`:242`).
  **The VB raster pass, the `vb_geo` pass and the classify chain are NOT bracketed.** A per-pass
  table comparable to a Nanite capture therefore requires extending this enum — R0e.
* **A CPU coverage rasterizer.** `crates/boyko_app/tests/sv0_oracle/mod.rs` ships `rasterize`
  (`:279`) producing a `Coverage` (`:211`) of `CoveredPixel` (`:193`) with `covered_count`
  (`:253`), plus `changed_covered_pixels` (`:798`). It is perspective-correct and supports
  translation-only instances.

### 5.2 Counters that do not exist

Nothing anywhere produces a **screen-space triangle-size histogram** or a **triangles-per-pixel**
statistic, and nothing reads the visibility buffer back to the host. `vb_id` is created with
`usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED` (`targets.rs:868`) — **no
`TRANSFER_SRC`** — and `frame_driver.rs:750` records that the engine deliberately has *"NO
`copy_image_to_buffer(depth)`"*; the only host readback path is the swapchain
(`host_dump.rs`, `BOYKO_HOST_DUMP`).

### 5.3 The instrument — decided with structure, not escalated

| Option | Cost | Verdict |
|---|---|---|
| (a) Widen `vb_id` usage with `TRANSFER_SRC`; `copy_image_to_buffer` on census frames; histogram on the host | +1 usage bit, +1 recorded copy on armed frames only, **zero** new `.spv`, **zero** manifest rows | **CHOSEN** |
| (b) A compute pass that histograms `vb_id` into an SSBO | a new `.spv`, a new `SHADER-VARIANT-MANIFEST.md` row, a new binding, a new barrier | Rejected — buys nothing (a) does not, and enlarges the very blast radius R0 exists to keep at zero |
| (c) Reuse the CPU rasterizer alone | zero engine change | Rejected **as the census** — it is a host mirror of the raster, not the shipped VB path, and the whole point of the census is to measure what the engine actually produces. Retained as R0c's cross-check |

`copy_image_to_buffer` already exists in the RHI (`boyko_rhi/src/encoder.rs:115`; impl at
`rhi_impl/encoder.rs:1031`). The census is armed by an env knob and threaded as an `Option`, so an
unarmed frame records **zero** extra commands — the exact discipline
`Option<&VbTimestampCollector>` documents (`gpu_timing.rs:247`) and the reason the golden
command stream stays byte-identical.

### 5.4 The statistic — a bracket, because the obvious one is capped at 1

**Rev 1's defect, stated plainly.** `vb_id` is an `R32G32_UINT` image (`targets.rs:866`) — **one
`(instance_id, raw_prim_id)` pair per pixel**. So `distinct (instance_id, local_tri) pairs ÷
covered pixels` is **≤ 1 by construction**, saturating exactly when every covered pixel carries its
own triangle. It cannot distinguish *"we have just reached one triangle per pixel"* from *"we are
ten times past it"* — which is the entire regime the campaign exists to serve. A K1 phrased as
*"never approaches ~1"* against a statistic that **can never exceed 1** is not a threshold, it is a
ceiling being mistaken for a reading.

Per censused frame, from the readback pairs, `local_tri = raw_prim_id % tri_count` reproducing
`vb_geom_fetch.hlsli:521` on the host:

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
(16 `.spv`, 6 of them ungated — §2).

**The ladder therefore splits, and the expensive half is conditional:**

| Outcome of the cheap census | Next |
|---|---|
| `D_est ≥ 1.0` | **K1 dead.** No counter, no shader edit, no re-bless. Done. |
| `D_est < 1.0` **and** the ladder converged | Genuinely sparse *or* instrument-limited — indistinguishable from below. **K1 UNDECIDED** — R0 cannot tell this from the instrument's own ceiling seen from below. Owner VALUES call, §13 Q6. The counter is NOT a scheduled rung: §8 contains none, and its design is recorded UNSOLVED. |
| Ladder not converged | `[k1_instrument].on_not_converged_fire_direction` — **K1 not adjudicated** for the FIRE direction, §9 clause 4. The REFUTE direction is unaffected: non-convergence means `D_est` understates, and an understatement already ≥ 1.0 still proves density ≥ 1 (`on_not_converged_refute_direction = "still_valid"`). |

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
area ratio**, checked within a stated tolerance, and reports the residual rather than asserting an
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
client area (`window.rs:252`, `AdjustWindowRectEx` at `:310`), and OS clamping is *already* a
recorded hazard here at 512² — `sv0_deferred_term_bench.rs:297-299` checks it, because *"an
OS-clamped window would silently measure a different per-pixel workload."* A display that clamps
1440p and 2160p produces three plausible rows and a **fabricated curve**, and every conclusion
above rests on the scaling law those rows are supposed to demonstrate. `[census]
.assert_achieved_extent` makes the readback's own dimensions the check.

**No error target is needed, and this is why.** The census renders at **full detail** — this engine
has no LOD, so there is nothing to hold an error target against. That makes the censused density
the **ceiling** of the mechanism available to any LOD scheme: a cluster hierarchy can only reduce
triangles below it. If the ceiling does not reach the regime, no LOD scheme reaches it either.
K1 is therefore decidable today, without the error target Rev 1's phrasing implied it needed.

All statistics are reported per camera path, path definitions committed as test constants — the
shape `sv0_scene/mod.rs:149-162` already uses for its camera.

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
measured free space on both volumes. The operator must therefore supply, as a prerequisite to R0f:

1. a UE5 install of a named version, with disk headroom for the editor **plus** a project **plus**
   its derived-data cache — and this project's standing hazard is that the Rust `target/` directory
   alone has filled this disk to zero and masked itself as linker errors;
2. a project that imports the §4 corpus with Nanite enabled;
3. a capture protocol — `stat GPU`, Unreal Insights, or RenderDoc — producing per-pass timings, with
   the same clock-pinning discipline §7 imposes on our own harness.

**If any of the three cannot be supplied, K2 fires**, and the disposition is not "measure something
else": it is a **scope restatement** the owner makes consciously (§13 Q1). The whole falsifiability
argument for this campaign rests on this rung, which is why it runs first and why R0a's gate is
mechanical rather than a paragraph.

### 6.3 The reference's own floor — a term Rev 1 never had

A capture is an instrument too. **A claim smaller than the reference's own reproducibility is
unfalsifiable no matter how good our side is**, and Rev 1's `joint_floor` named a pair of
instruments while defining only one. So the reference capture is repeated across
`[decidability].sessions` separate sessions on the identical scene, camera and settings, and the
relative peak-to-peak spread of its per-pass medians **is** the reference floor.

The two floors combine by `[decidability].joint_floor_rule = "sum"`, and the reason is stated
rather than conventional: **quadrature assumes two independent draws from one noise process, and
these are not that.** A systematic capture bias — a different clock discipline, different pass
boundaries, a driver-side difference between the two engines — is not an independent random error,
and adding it in quadrature would understate it. Summing is conservative in the direction that
makes the campaign's own claim harder to close, which is the correct direction for a gate whose
purpose is to keep us honest.

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
  (`sv0_deferred_term_bench.rs:53-77`).
* **A spread gate measuring its own resolution.** The timestamp counter's *step* is not the
  `timestampPeriod` the device reports; the harness had to recover it as the **GCD of raw tick
  counts** over a whole session (`:83-100`). A "cross-session spread" that is one lattice step
  carries no information.

**R0's harness MUST therefore, non-negotiably:**

1. counterbalance (ABBA), and **report** the order-bias residual with its own band;
2. carry a **null control** — two identical configurations — with a **pre-registered** maximum, as
   `SV0_NULL_CONTROL_MAX_FRACTION` (`:378`) does, fixed before the run and never widened;
3. **measure** the counter quantum by tick GCD and report it alongside `timestampPeriod`
   (`:94-96` the RESOLUTION field list, `:448`/`:463` the transcribed bounds, `:751-772` the consistency check);
4. state the **resolvable delta with confidence intervals**, and make the effective spread gate
   `max(stated gate, measured median lattice / |median|)` — **but only where the lattice term is
   licensed by evidence.** ⚠️ Rev 3 transferred the `max()` and dropped the guard, which turned a
   non-negotiable clause into a gate a homogeneous sample could widen to rescue a failing run —
   this campaign's own #1 named defect, introduced in the clause written against it. The sibling
   does **not** grant the widening by default: `sv0_deferred_term_bench.rs:805-807` reads
   `if may_widen { SV0_SESSION_SPREAD_MAX.max(lattice_floor) } else { SV0_SESSION_SPREAD_MAX }`,
   where `may_widen` requires at least `SV0_LATTICE_MIN_DISTINCT_TICKS = 7` (`:399`) distinct
   observed tick values (`:680-681`), *"licensed by EVIDENCE … rather than granted by default"*
   (`:798`). A **separate, non-waivable** test asserts `lattice_floor <= SV0_SESSION_SPREAD_MAX`
   unconditionally, *"so it can never silently widen the gate"*. R0e lands **all three** — the
   `max()`, the distinct-tick evidence floor, and the non-waivable assertion — or none of them.
   This is R16 (*a literal transferred without its denominator*) one level up: **a gate transferred
   without its precondition**;
5. discard warmup, run ≥3 separate processes, and pin every session's transcribed number as a test
   literal under the MEASURED discipline.

**One trap the R0e implementer will otherwise hit.** Every `read_query_pool_ns` reader requests all
of its collector's `(begin,end)` pairs with `VK_QUERY_RESULT_WAIT_BIT`, which **blocks forever** on a
pair its recorder never wrote that frame — `gpu_timing.rs:344` states this, and it is why three
separate collectors exist rather than one widened `PASS_COUNT`. Extending `VbTimedPass` to cover
raster/geo/classify means **every added pair must be written unconditionally on every armed frame**.
R0e therefore also lands a **written-pair bitmask asserted before the read**, so a conditional
bracket fails as a red assertion instead of hanging the test binary — a hang is not a gate.

---

## 8. Rungs

Ladder: **kill the baseline cheapest → land content → land the instrument → run the census → state
decidability → close the inequality** (R0f *or* R0f′, whichever branch R0a selected). Each rung is
independently committable, has **one** gate, and names the mutation that turns it red. *A mutation
that is only argued does not count; the commit message records the mutated run's output.*

### R0a — the reference-rig probe (zero engine code) — **kills K2 cheapest**

**Lands:** `docs/VG-R0-REFERENCE-RIG.toml` — a machine-readable record: UE version string, install
path, GPU name, driver version, capture tool + version, render resolution, `MaxPixelsPerEdge`, free
disk on the install volume, **the sha256 of
[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) — the freeze (§0.1); the claim file is
deliberately *not* hashed** — the **pass-correspondence map**, and a per-pass table for **one stock
UE5 scene** (no corpus needed).
Plus `crates/boyko_app/tests/vg_r0_reference_rig.rs` reading it.

**The record has two shapes, and the gate says which fields each requires.** Rev 2 demanded *"every
field present and not `PENDING`"* over a list including the UE version string, the capture tool and
a stock-scene pass table — **none of which can exist on the `achievable = false` branch.** As
written, R0a could not pass on its own most likely outcome: the same structural hole D3 names,
relocated from the assertion into the field list.

**Gate (one) — `achievable = true` branch, four parts:** (a) every field in the *positive* set
present and not the `PENDING` sentinel — the same discipline `goldens/PINS.toml:15` defines;
(b) the recorded **GPU name matches the one this engine reports at boot** on this box — a
mechanical cross-check, not a transcription; (c) the recorded resolution equals
`[census].decision_resolution` read from the **thresholds** file, not from a constant this rung
authors; (d) the recorded `VG-CAMPAIGN-THRESHOLDS.toml` sha256 matches the file re-hashed at test
time; and the record carries the **pass-correspondence map** — the reference's pass names for its
stock scene — which `[scope].require_pass_correspondence_map_at` puts *here*, at rung one, rather
than at R0f where it would be written with both tables already in hand (§8 R0f, P1-10's fix).

**Gate (one) — `achievable = false` branch, three parts:** (a′) the *negative* field set is present
and not `PENDING` — `reason`, `search_method`, `editor_binary_name`, `probed_at`; (b′) the
re-derivation below passes; (d) as above, unchanged — the thresholds hash is asserted on both
branches.

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
`crates/boyko_app/tests/vg_corpus_ingest.rs`.

**Gate (one, four parts):** (a) every corpus payload's sha256 matches its manifest pin; (b) each
`.glb` decodes to a `MeshData` whose triangle count equals the manifest's published count;
(c) each mesh, registered through the **streamed** path, lands a geometry slot
`!= VB_GEOMETRY_RESERVED_SLOT` and a `gMeshMeta` row whose `index_width` / `vertex_count` /
`index_count` match the decoded mesh; (d) the largest corpus mesh registers without allocation
failure (§3.4).

**RED if / mutations (DEMONSTRATED):**
* flip one byte of a pinned hash in `CORPUS.toml` → (a) reds;
* ⚠️ **RETRACTED at Rev 4 — this mutation was DEAD, and it was the one Rev 1–Rev 3 each called the
  rung's most important.** It read: *"register the same mesh through host-authored `register_mesh`
  instead of the streamed path → slot is `0` → (c) reds."* It does **not** red.
  `backfill_vb_geometry_slots` (`crates/boyko_render/src/gpu_upload.rs`, run at
  `crates/boyko_app/src/runner.rs:787`, after `upload_mesh_assets` and after `finish()`) claims a
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
the `dxc`-dependent gates (`cluster_cull_spv_sync.rs:196-204`). **Procedural mitigation, and it is
binding: the rung is not commit-eligible until the gate has been run with the corpus present and
its output pasted into the commit message.** A gate proven only on a box that skipped it is not a
gate.

### R0c — the census instrument + its sensitivity control

**Lands:** `TRANSFER_SRC` on the `vb_id` ring (`targets.rs:862-872`); an `Option`-threaded census
readback armed by env knob; the host-side histogram + triangles-per-pixel reducer;
`crates/boyko_app/tests/vg_density_census.rs`.

> ⚠️ **R0c lands the first in-frame image readback in the shipped recorder, and that is a bigger
> step than "reuse an existing seam" implies.** Every `copy_image_to_buffer` call site in this tree
> today is under `crates/boyko_rhi_vulkan/tests/`; there is **none** in `src/present/`, and
> `frame_driver.rs:750` records that the engine deliberately has no depth readback. So R0c adds
> (i) a new layout transition of a **ring** image — `COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL
> →` its `SAMPLED` read — inside the RDG auto-barrier system, and (ii) a **host read of a per-FIF
> resource**, which is the exact shape of this project's recorded cross-frame bug class (host
> access racing the fence on per-FIF rings, with `FRAMES_IN_FLIGHT == 2` at `ui/mod.rs:87`).
> Neither is visible to gate (a), because both exist only on **armed** frames — the frames the
> goldens never render. The readback must therefore wait on the frame's own fence before mapping,
> and that ordering is asserted in the rung's own test, not assumed.

**Gate (one, five parts):** (a) **every VB image golden byte-identical** to its `PINS.toml` pin
with the census unarmed — the usage widening and the unarmed `Option` must cost nothing. *Scoped to
the blessed legs:* §9 clause 5 records two `sha256_hwrt = "PENDING"` pins on which `golden.ps1`
exits 2 by design, and a gate quantified over an unblessed pin is the vacuous-selection defect
again;
(b) on a **procedurally generated** fixture whose screen-space triangle size is analytically known,
the census's modal bucket is the analytic bucket;
(c) the census's covered-pixel total agrees with `sv0_oracle::rasterize`'s `covered_count` **on that
same procedural fixture, at 512²**, within a **pre-registered** tolerance fixed before the run —
scoped to the fixture because the oracle takes one mesh and translation-only instances (§5.7) and
cannot reach the corpus at any resolution;
(d) the ladder is driven from `[census].resolution_ladder` in the **thresholds** file, whose sha256
the test re-asserts, the census produces one row per rung, **and the readback's own dimensions equal
the requested rung** (`[census].assert_achieved_extent`) — a ladder silently truncated, or silently
clamped by the OS, reds;
(e) **cross-process `vb_id` identity is MEASURED and RECORDED here — and deliberately NOT
asserted.** ⚠️ Rev 4 wrote (e) as a gate, which made it incoherent with R0d: a negative result is
something the plan explicitly calls *"a real finding about the raster path"* and wants recorded,
yet asserting identity would **red R0c** and, via §9 clause 4, make the rung not commit-eligible —
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

**Gate (one, three parts):** (a) the census is **reproduced across `[decidability].sessions` = 3
separate processes** under `[census].cross_run_gate` — **the sha256 of the readback itself**;
(b) `D_est`, the convergence check, the histogram and both report-only statistics are produced at
**every** ladder rung, so the resolution-dependence is on the page rather than in the choice of one
row; (c) the histogram's modal bucket moves between adjacent rungs by the **per-pair `log2` of the
actual area ratio**, within `[k1_instrument].histogram_shift_tolerance_buckets`, with the residual
reported — over the **two** non-excluded pairs (1080p→1440p, 1440p→2160p), rung 0 being excluded as
a different frustum (§5.7).

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

**RED if / mutations.** Rev 2's named mutation — *"point two of the three runs at different camera
paths"* — is **not a gate test**: it changes the test's *input*, and would red any hash of anything.
The mutations that actually probe (a) are ones that leave the shaded golden **identical** while
changing `vb_id`:
* **permute the spawn order of two identical instances** → every `instance_id` changes, the shaded
  pin is byte-identical, and (a) must red. This is the mutation that proves the gate reads `vb_id`
  and not the image;
* drop the ladder to its decision row only → (b) reds;
* (c)'s mutation must run **on the corpus**, since that is what R0d renders — R0c owns the
  procedural fixture, and Rev 5's version of this mutation named the fixture, which R0d never
  touches. The corpus mutation: **substitute one ladder rung's histogram with another rung's** →
  the per-pair `log2` residual for both affected pairs exceeds tolerance → (c) reds.

### R0e — the decidability statement — **K3's test**

**Lands:** `VbTimedPass` extended to bracket the VB raster, `vb_geo` and the classify chain, with
the written-pair bitmask of §7; a counterbalanced ABBA harness over the corpus scene with a null
control; `crates/boyko_app/tests/vg_r0_decidability.rs`, all session numbers transcribed as
literals.

**Blocked until the claim exists, and R0e PINS it.** R0e's test asserts `claim.mode` and its mode's
delta field are **not `PENDING`** before it measures anything, and fails if they are.

**R0e additionally transcribes the filled claim value as a MEASURED-discipline literal alongside the
floor**, in the same commit — `[ordering].claim_pinned_into_r0e_literals`. ⚠️ Rev 5 asserted that
boolean as `true` while §8 listed no such literal and **§8 R0f said the ONE gate reads the claim
"from a file"** — so the post-fill edit window §0.1 identified stayed wide open in the rung specs
while a data file claimed it closed. R0f now asserts the file still equals R0e's pinned literal;
a claim edited between the two rungs reds.

⚠️ **And the block attaches per mode, which Rev 5 got wrong.** `[ordering]` named R0e for both
instruments — but in `absolute` mode §8 R0f′ states plainly that R0e's paired-delta floor is
unusable and R0f′ needs its own. The floor the claim is compared against is then measured **two
rungs later**, with the claim already filled and visible. Blocking R0e constrained nothing there.
`claim_blocks_rung_absolute = "R0f_prime"` — **the rule attaches to whichever rung measures the
floor for that mode**, which is the campaign's single P0 and had been unguarded on the branch §11
calls expected.

**Gate (one, three parts), every fraction AND ITS DENOMINATOR read from
[`VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) rather than minted here:**
(a) the null control's `|median paired delta|` is at or below
`[decidability].null_control_max_fraction` of **the armed paired delta**;
(b) the reported `median_lattice_ns / |median|` is at or below that fraction for **every** bracketed
pass — a pass whose cost sits at the lattice is reported as **not resolvable**, by name, rather than
averaged into a total;
(c) the cross-session spread of the **paired deltas** over `[decidability].sessions` processes is
within `max([decidability].session_spread_max, measured lattice / |median|)`;
(d) the chain total's floor is measured on **one bracket spanning the chain**
(`[scope].chain_floor_rule`), not composed from the per-pass floors — the passes share occupancy,
caches and a queue, so arithmetic composition would assume an independence they do not have.

> ⚠️ **The denominators are the gate.** Rev 2 transferred the sibling's three literals while
> silently changing what they divide. The sibling gates the null control against the **armed
> delta** — it fired at 33% (−2048 ns against a 6144 ns signal) and *that failure is what produced
> the ABBA redesign*. Rev 2 gated it against *"the smallest per-pass median"*, an **absolute** cost:
> a pass costing 100 µs would carry a 10 µs null budget where the sibling's was 614 ns, roughly a
> 20× weakening, **and the precedent's own red event would have passed.** R0e's first named
> mutation — revert to strict ABAB — would then not have fired. Same for (c): this project has
> recorded ~21% run-to-run spread on **absolute** GPU pass costs at high N (the VB-P1d bench), so a
> 0.10 gate on absolute medians would red for a known instrument property rather than a finding.
> Both denominators are now written down (`null_control_denominator`, `session_spread_denominator`).

The three literals themselves are not invented for this campaign — they are the ones
`sv0_deferred_term_bench.rs:350`, `:366`, `:378` already carry, fixed by a rung that had never
heard of virtual geometry.

> ⚠️ **Rev 3/Rev 4 called them *"measured on this exact box"*. They are not measurements** — they
> are **pre-registered protocol thresholds**, and the sibling says so at the constants themselves:
> `SV0_NULL_CONTROL_MAX_FRACTION` is *"Registered at the same 10% … **Fixed here, before any run**,
> so it cannot be widened to rescue a failing control"*; `SV0_SESSION_SPREAD_MAX` carries a
> `const { assert!(… <= 0.10) }` and *"may be TIGHTENED on new evidence, never widened"*;
> `SV0_BENCH_SESSIONS = 3` is *"The plan's session count."* In a document whose central discipline
> is the measured/authored distinction — §11's fenced exception exists for exactly this — calling
> three authored numbers "measured" **launders them as evidence**. That is the category error this
> campaign exists to prevent, committed in the sentence claiming rigour.

**RED if / mutations (DEMONSTRATED):**
* revert the harness to strict ABAB → (a) must exceed its budget. This was **measured** on this
  hardware in the sibling rung, so it is a re-demonstration, not a hope;
* make one added bracket conditional on a branch the fixture never takes → the written-pair
  bitmask assertion reds (instead of the `WAIT_BIT` readback hanging);
* halve the sample count → the confidence interval widens past the pre-registered bound → red.

### R0f — the reference capture — **closes the ONE gate** (`nanite_relative` mode)

**Runs only if R0a recorded `achievable = true`.** Otherwise the ladder terminates in R0f′ below.

**Lands:** the corpus imported into the R0a project; per-pass Nanite timings at the pinned error
target and the pinned resolution, for the **same camera paths**, **repeated across
`[decidability].sessions` capture sessions** (§6.3), recorded into
`docs/VG-R0-REFERENCE-RIG.toml`'s table; and the campaign's **decidability statement**: the smallest
delta this pair of instruments can jointly resolve.

**Gate (one, four parts):** (a) a reproducible per-pass table with named GPU / scene / resolution /
error target; (b) the reference floor is derived from the cross-session spread of that table, and
`joint_floor = our_floor + reference_floor` per `[decidability].joint_floor_rule`; (c) the ONE gate
itself — `joint_floor < claim.nanite_relative_chain_delta`, **both sides scoped to the bracketed VB
pass chain** (`[scope].claim_scope`), with the claim read from a file that R0e already proved was
filled before the floor was measured; (d) the **pass-correspondence map** recorded at R0a is
**total** over our bracketed pass set — without it `nanite_relative_per_pass_regression_max` cannot
be evaluated at all, and whoever writes the map after both tables exist writes it with the answer in
hand.

**Scope, stated because Rev 2 got it wrong.** Rev 2 compared a per-pass floor against a field named
`frame_total_delta` with **no composition rule anywhere**, which made the ONE gate not evaluable.
R0e measures the VB pass chain: raster, `vb_geo`, classify, and the three passes `VbTimedPass`
already brackets. A *frame* additionally contains CSM, SDF, DDGI, post/AA, present and all CPU
time — none of which this campaign touches and none of which the harness measures. The claim is
about the chain, and the field is now named `nanite_relative_chain_delta`.

**RED if / mutations — one per side, which is the point:**
* raise the recorded floor above the claim → the inequality assertion reds (Rev 1 had this one);
* **fill the claim to fit the floor → impossible without reding R0e**, which refuses to run while
  the field is `PENDING` and whose MEASURED literals are committed after the fill. This is the
  cheaper cheat, and it is the one Rev 1 and Rev 2 both left open.

That pair is the whole campaign's falsifiability condition, and both halves must be able to fail.

### R0f′ — the absolute-mode closure — **closes the ONE gate when K2 has fired**

**Runs iff R0a recorded `achievable = false` and its negative was re-derived.** This rung exists
because Rev 1 left the *most likely* branch with no closure at all: no reference, therefore no R0f,
therefore no falsifiability condition, therefore a campaign that proceeds to R1 on an argument.

> ⚠️ **Rev 2 wrote *"nothing new to measure — the left-hand side is already ours"*, and that was
> its sharpest error.** The harness §7 mandates measures **paired differences**, and its entire
> algebra exists to *cancel* the absolute terms: in `m_k = μ + τ·armed(k) + γ(fi(k)) + β·k + ε_k`,
> the ABBA statistic `(d₁+d₂)/2` recovers `τ` **precisely by eliminating** `μ` (the per-frame
> baseline), `γ` (the frame-in-flight-slot offset) and `β` (position drift)
> — `sv0_deferred_term_bench.rs:34`, `:58-62`. Those are exactly the terms an absolute millisecond
> reading must **retain**, and this repo has measured them to be large (the null control read
> −2048 ns against a 6144 ns signal — a third of the "signal"). **A floor produced by a design that
> cancels the absolute terms says nothing about whether an absolute reading is trustworthy.**
> R0f′ therefore needs its own instrument, and it does not get to pretend otherwise.

**Lands:** an **absolute-time** measurement — per-pass and chain-total *medians*, not deltas —
across `[decidability].sessions` separate processes, with the cross-session spread of those
absolute medians reported; plus the inequality assertion in `vg_r0_reference_rig.rs` against
`claim.absolute_chain_ms`, and the corpus-and-quality context that makes an absolute target
meaningful (`[corpus].arrangement`, `[quality].arbiter`).

**Gate (one, three parts):** (a) the measured absolute cross-session spread is at or below
`[absolute_mode].absolute_session_spread_ceiling` — **derived by measurement here, not adopted**;
the ceiling is pre-registered at 0.25 from this project's recorded ~21% absolute-cost spread at
high N, with margin, and R0f′ reds if the measurement exceeds it; (b) the ONE gate, and **Rev 5
rewrote it because Rev 3 and Rev 4 both shipped it as a dimension error** — see below; (c) the
thresholds file's sha256 re-asserts, and `claim.mode == "absolute"` is consistent with R0a's
`achievable = false` — a mode set to `nanite_relative` while the rig says unachievable reds, so the
two documents cannot disagree silently.

> ⚠️ **Gate (b), twice wrong, and it guarded the *expected* branch.** Rev 3 compared a
> dimensionless fraction against *"the resolvable fraction of a millisecond target"*. Rev 4 claimed
> to fix that with *"`absolute_floor < claim.absolute_chain_ms`, both in milliseconds — a genuine
> inequality between two quantities of the same kind"*. **It is not.**
> `[absolute_mode].absolute_floor_source` is `cross_session_spread_of_absolute_per_pass_medians` —
> a **relative fraction**, the same quantity gate (a) compares against a fraction. One source
> cannot be both. Read as relative, gate (b) is the identical dimension error one file over. Read
> as milliseconds, it says `0.28 ms < 5 ms` — **true for any non-degenerate target**, because a
> *resolution* being smaller than a *level* tells you nothing about whether you can see the gap.
>
> **Rev 5's gate (b), from `[absolute_mode]`'s three new rules:**
> **`claim.absolute_chain_ms < measured_chain_median_ms`  AND  `absolute_floor_ms <
> absolute_distance_ms`**, where `absolute_floor_ms = spread × measured_chain_median_ms` and
> `absolute_distance_ms = |measured_chain_median_ms − claim.absolute_chain_ms|`. Under the first
> conjunct this is `c < m(1−s)`. Both sides milliseconds; the inequality is about **the distance we
> intend to close**, not the level we intend to reach.
>
> ⚠️ **Rev 7 wrote the second conjunct's partner into THIS SECTION, and that is the fix.** Rev 6
> landed the two-sided rule in `[absolute_mode].absolute_gate_rule` and left §8 — the section an
> implementer codes from — carrying the one-sided version *and the sentence the frozen file
> explicitly refutes* (*"a target already passed now reds"*: false outside a ±`s·m` band, since
> `c > m` passes whenever `c > m(1+s)`). Rev 6 diagnoses this exact failure at R0d — **"rewriting
> the explanation is not rewriting the gate"** — and then committed it one rung over. The governing
> lesson of Rev 7: **a fix that lands only in the frozen file has not landed.**

**Absolute mode is honestly weaker, and saying so is the deliverable.** Its floor is roughly
2.5× the paired-delta floor, because absolute readings keep every term ABBA was built to remove.
A weaker instrument reported plainly beats a strong-looking number the harness cannot support —
and if (a) reds, the correct reading is that **this box cannot support an absolute claim at all**,
which is a finding, not a failure.

**RED if / mutations — re-derived at Rev 7 until they fire, not re-worded:**
* set `mode = "nanite_relative"` while `achievable = false` → (c) reds;
* set `c > m` (a target already beaten) → **conjunct 1** reds: there is nothing to close;
* set `c` inside `(m − s·m, m)` → **conjunct 2** reds: the gap is inside our own resolution;
* reuse R0e's paired-delta floor as `absolute_floor` instead of measuring absolutes → (a) has no
  measurement to gate and the rung cannot report.

> ⚠️ **The mutation Rev 5 and Rev 6 both named here PROVABLY DOES NOT FIRE, and it shipped three
> times.** It read *"set `absolute_chain_ms` below the measured floor → (b) reds."* Plug it in:
> `c < s·m` ⟹ `distance = m − c > m(1−s)` and `floor = s·m`, so the second conjunct asks
> `s·m < m(1−s)` ⟺ **`s < 0.5`** — true for every `s ≤ 0.25` the ceiling permits. It also passes
> conjunct 1, since `c < s·m ≤ 0.25m < m`. **Green under both the old gate and the new one.** Kept
> struck through as a specimen: a red mutation is a claim about arithmetic, and this one was never
> checked against the arithmetic in three revisions of writing it down.

**Deliberately NOT red: an absurdly ambitious target** (`c → 0`). This gate adjudicates
**decidability**, not achievability — a 1000× claim is trivially resolvable. ⚠️ But the handoff is
real and Rev 7 names its cost rather than leaving it implied: the plan sends achievability to §13
Q1, **and the pre-registration rule requires the owner to set that number before any measurement
exists.** So the party asked to judge whether a target is sane is, by construction, the one party
with no measurement. That tension is inherent to pre-registration and is accepted knowingly; it
applies equally to `nanite_relative_chain_delta`, which likewise carries no sanity band.

---

## 9. ABORT criteria

The rung is **reverted or the campaign re-scoped** — not softened mid-flight — if any of:

1. **K1 — no content.** `[k1].k1_fire_rule` (**a field that does not exist — see the Rev 7 note below**): the **upper-bound survivor count** per covered pixel is
   below 1.0 at `[census].decision_resolution`. **Only the upper-bound instrument can fire this
   kill** — §5.6. The cheap census can *refute* K1 (`D_est ≥ 1.0`) and can never fire it, because
   `D_est` is a lower bound; anything below the threshold is indistinguishable from the
   instrument's own ceiling seen from underneath.
   ⚠️ **Rev 7 corrects two things in this clause, and the second was blocking the only outcome R0
   can produce.**
   * The field named above does **not exist**. `[k1]` carries `rule`, `k1_decision_rule`,
     `k1_fire_at_r0` and `k1_fire_instrument_status`. The campaign's abort criterion was defined by
     reference to a nonexistent field — the frozen file's whole purpose is that gates point at it,
     so a dangling name is a gate pointing at nothing. The live rule is `k1_decision_rule`.
   * Rev 6 wrote *"K1 is not adjudicated at all if the ladder-convergence check fails"*,
     **unconditionally**. That contradicts `[k1_instrument].on_not_converged_refute_direction` and
     is wrong in the direction that matters: non-convergence means `visible_tris` is still rising,
     so `D_est` **understates** — and an understatement already ≥ 1.0 still proves density ≥ 1.
     Convergence is a precondition for **firing**, never for **refuting**. Worse, the two are in
     structural tension: in the micro-polygon regime this campaign exists to serve, `visible_tris`
     is still climbing steeply between 1440p and 2160p, so `ladder_convergence_margin = 0.05`
     **cannot** be met — and the unconditional form would therefore rule *the favourable case* out
     of adjudication precisely when it is true. A frozen file vetoing its own plan's decisive case,
     which is the shape §9's own Rev 6 note names one rung over.

   **The rule, per direction:** `D_est ≥ 1.0` **refutes K1 regardless of convergence.** Only the
   fire direction requires convergence, and R0 cannot fire K1 at all (`k1_fire_at_r0 = false`), so
   in practice non-convergence leaves R0 with REFUTED-or-UNDECIDED — see the disposition below.
   Non-degeneracy (`min_covered_pixels`, `min_visible_tris`) is required in **both** directions:
   a sentinel-only readback proves nothing either way.

   ### K1 has exactly two reachable outcomes at R0, and UNDECIDED is the likely one

   ⚠️ **Rev 6 declared `k1_fire_at_r0 = false` and left `k1_decision_rule`'s `escalate` naming no
   addressee — no rung, no `[gating]` field, no §13 question.** That is D3's shape relocated: the
   most likely branch with no disposition. With `d_est_ceiling = 4.0`, a plausible corpus puts
   `D_est` in the 0.7–1.3 band, so UNDECIDED is not a corner case.

   | Outcome | Condition | Disposition |
   |---|---|---|
   | **K1 REFUTED** | `D_est ≥ 1.0` at the decision resolution, non-degeneracy met | The mechanism exists. The ladder proceeds to R1. This is the cheap decisive case §5.6 front-loads. |
   | **K1 UNDECIDED** | `D_est < 1.0`, or non-degeneracy unmet | **Owner VALUES call — §13 Q6.** R0 cannot distinguish "genuinely sparse" from "the instrument hit its ceiling seen from below", because firing needs an upper bound R0 has no buildable instrument for. |
   | **K1 FIRED** | — | **Unreachable at R0.** Requires the unsolved upper-bound instrument. |

   **On UNDECIDED the ladder does NOT silently proceed.** The owner chooses: accept the campaign's
   premise unadjudicated and proceed to R1 on that basis, knowing K1 was never tested; change the
   target content class and re-run R0b–R0d; or fund the upper-bound instrument as its own campaign.
   The one route foreclosed is the one this document forecloses everywhere else — re-running the
   census until a number comes out favourable.

   **Consequence for §0, stated rather than left standing:** the headline *"the three ways it kills
   the campaign"* is now **false as written** — only K2 and K3 can fire at R0. K1 can be refuted or
   left undecided, never fired.

   > ⚠️ **Two dead rules are recorded here rather than deleted, because each looked decisive.**
   > Rev 2's `rule = "all_three_below"` put a `_max` among two `_min`s, so on the canonical
   > no-mechanism scene — a few giant flat quads — two conjuncts held, the third did not, and **K1
   > failed to fire on the exact scene it was written to catch**; an implementer coding from the
   > TOML would have written `modal < 16` and built a kill that fires when triangles are *small*,
   > i.e. when the premise is *confirmed*. Rev 4's two-conjunct replacement was **redundant**:
   > modal bucket > 16 px implies `visible_tris ≲ covered_px/16`, hence `D_est ≲ 0.06 ≪ 1.0`, so
   > conjunct 1 held automatically whenever conjunct 2 did. "Two conjuncts, both pointing the same
   > way" was one conjunct and a weaker consequence of it — and the real decisive statistic was the
   > modal bucket, whose cross-rung derivation this ladder cannot support (§5.7).

   > ⚠️ **Rev 6: R0 CANNOT FIRE K1, and that is now a stated scope boundary rather than a rung
   > nobody wrote.** Rev 5 named the firing instrument as a *"frustum + backface survivor counter"*
   > in `vb_raster.fs.hlsl`, *"scoped as its own rung"* — and §8 contains no such rung. It is wrong
   > twice, the second fatally:
   > **(a) wrong stage** — a fragment shader runs only for fragments that survived rasterisation
   > and, with early-Z, the depth test, i.e. approximately the *visible* set that `vb_id` already
   > caps; frustum- and backface-culled triangles never reach it, and §2 of this very plan records
   > that the per-primitive lane *"is not independently reachable"* without a mesh shader, one draw
   > per meshlet, or a software rasteriser.
   > **(b) probably inert regardless** — apply §5.4's own killing argument to the replacement:
   > survivors include every *occluded* in-frustum front-facing triangle, and depth complexity on a
   > multi-million-triangle corpus is where the count lives. A 5 M-triangle asset in frame gives
   > ~2.5 M survivors against ~2.07 M covered pixels at 1080p, so `survivors/covered < 1.0` cannot
   > hold **whatever the visible triangle size is**. That is `submitted/covered`'s self-satisfaction
   > with a 2–4× constant knocked off — the same defect, one instrument later.
   >
   > So `[k1].k1_fire_at_r0 = false`. **R0's claim is that it can REFUTE K1 cheaply and soundly**,
   > which is a real and sufficient deliverable. Firing needs an upper bound on *visible* density
   > whose firing condition is demonstrably **not** precluded by R0b's own high-poly corpus gate —
   > an unsolved design problem, recorded as unsolved, and out of R0's scope until someone solves
   > it. Naming a rung that cannot be built is worse than naming none.

   When K1 fires — which R0 cannot do — cluster LOD has no mechanism of action on
   this content — at **full detail**, i.e. at the ceiling any LOD scheme could ever see — and **the
   campaign is refuted as stated**. The disposition is the owner's: change the target content class
   (and re-run R0b–R0d against it), or stop. It is explicitly *not* "generate a denser corpus" —
   §4.2 records why that makes the kill vacuous — and it is explicitly *not* "adjudicate at 2160p
   instead", which §5.4 forecloses by freezing the decision resolution.
2. **K2 — no baseline.** R0a records `achievable = false` **and the test re-derives it** (§8 R0a).
   Then *"faster than Nanite"* is unfalsifiable and the goal is restated as an **absolute**
   ms-at-quality target. **Owner VALUES call** (§13 Q1) — taken consciously, at rung one.
   **This is a re-scope, not an abort:** the ladder continues to **R0f′**, which closes the same
   inequality with `claim.absolute_chain_ms`. Rev 1 treated this branch as terminal, which left the
   campaign's *most likely* path with no falsifiability condition at all.
3. **K3 — undecidable harness.** Two distinct outcomes, which Rev 2 conflated under one clause:
   * **(3a) the instrument misbehaves** — R0e's gate reds (null control over budget, a pass sitting
     at the lattice, cross-session spread out of band). The ladder does not proceed to R1 until the
     instrument is fixed. Nothing is learned about the campaign either way.
   * **(3b) the instrument works and the answer is no** — R0f/R0f′'s inequality reds: the floor is
     real, measured, and **larger than the claim**. This is research §5's K3 as actually worded
     (*"if the resolvable delta exceeds the delta we intend to claim, no result from this campaign
     is defensible"*), and its disposition is different: the instrument is not broken, so fixing it
     is not the move. The owner either lowers the claim to something this pair of instruments can
     resolve — which may make the campaign not worth running — or invests in a better instrument.
     **Owner VALUES call**, and it is the outcome the whole R0 rung exists to surface early.
4. **The instrument is untrustworthy rather than the result being bad — and this has its own
   disposition, because it is the case that gets misread.** If R0c's sensitivity control (b) fails
   while (a) and (c) pass, or R0e's null control fails while the armed medians look tidy, the
   correct reading is *the instrument is blind*, **not** *the effect is absent*. Outcome: the rung
   is **not** commit-eligible, no number from it enters any later gate, and the failure is recorded
   in this document's §11 with its date. The sibling rung's ABAB null control is precisely this
   case: three armed sessions looked tidy and inside their gate while the control said a third of
   the "signal" was ordering bias.
5. **Golden-bless throughput.** Two of the twenty-four pins in `goldens/PINS.toml` still carry
   `sha256_hwrt = "PENDING"` (`:364`, `:409`) — their software legs are blessed, their hwrt legs are
   not. R0 moves no pin, so it is unaffected; but the first byte-moving rung of this campaign
   starts on an incompletely-green corpus, and §13 Q4 puts the bless-bandwidth question to the
   owner before that rung is scheduled, not after.

---

## 10. Risks

| # | Risk | Precedent | Mitigation |
|---|---|---|---|
| R1 | **Vacuously-green gate** — an assertion quantified over an empty or self-referential selection. | The campaign's #1 recurring defect; found five times in the sibling plan alone. | Every rung names a mutation and the commit records its output; R0c(b)/(c) are deliberately paired so neither can pass alone. |
| R2 | **A procedural corpus makes K1 untestable.** | New, and it is why §4.2 rejects the cheapest corpus option. | The corpus is fetched real content; procedural geometry is confined to R0c's sensitivity control. |
| R3 | **The harness measures its own resolution, or its A/B rides the ring.** | MEASURED in the sibling rung, both of them: a "spread" that was one median lattice step, and an ABAB phase perfectly aliased with `FRAMES_IN_FLIGHT == 2`. | §7 clauses 1, 3–4: ABBA with the residual reported; the quantum measured by tick GCD and the spread gate read against it. |
| R4 | **`WAIT_BIT` readback hangs instead of failing.** | `gpu_timing.rs:344` documents the deadlock; three separate collectors exist because of it. | R0e's written-pair bitmask, asserted before the read. |
| R5 | **Stale doc sends the importer down the `None` path.** | Verified: ≥6 comments still claim `VB_IMPLEMENTED == false` while `render_path_config.rs:130` says `true`. | R0b's second red mutation targets exactly this; fixing the comments is a separate one-line commit, deliberately not absorbed here. |
| R6 | **Host-visible residency ceiling.** | `mesh_assets.rs:320`: every mesh buffer is `HostVisibleCoherent`. | R0b gate (d); the device-local + staging path is a named follow-up, not R0 work. |
| R7 | **The `vb_id` usage widening perturbs a golden.** | New. | R0c gate (a) over every VB pin, with a demonstrated red (record the copy unconditionally). |
| R8 | **UE5 capture measures a different scene than our census.** | New — the two engines must load the same bytes. | §4.3: an asset that cannot be imported by both is not corpus material; R0a(c) pins the resolution across both. |
| R9 | **Disk exhaustion masquerading as a build failure.** | This project's record: `target/` has filled this disk and surfaced as linker errors. | §11 records the measured headroom; R0a's record carries free-disk as a required field, and R0a's negative branch **re-reads** it at test time. |
| R10 | **The claim is set to meet the floor.** The cheapest way to close the ONE gate is to write the number on the right after seeing the left. | The P0 of Rev 1; of Rev 2, answered with a hash around the string `PENDING`; and of Rev 3–Rev 5, answered with an ordering rule that **named one rung for two instruments**. | §0.1 **plus** `[ordering]`: the claim blocks whichever rung measures the floor **for that mode**, R0e pins the filled value into its MEASURED literals, and R0f asserts the file still equals the pin. ⚠️ Rev 3–Rev 5 claimed here that this *"does not depend on anyone noticing an edit"* — **retracted**; ordering constrains commits, not knowledge, and party separation is what carries the weight. |
| R11 | **A statistic that cannot exceed its own threshold.** | Rev 1: `visible_tri_per_covered_pixel ≤ 1` by construction. Rev 2: `submitted/covered < 1.0` precluded by R0b's corpus gate. **Rev 3–Rev 5: `D_est` capped at exactly 4.0 and a *lower* bound firing a kill.** Three instruments, one defect. | §5.6's directional split — `D_est` may only **refute** K1 — plus `[k1].k1_fire_at_r0 = false`, because the proposed firing instrument is both mis-sited (a fragment shader cannot see frustum/backface survivors) and **probably inert for the same reason Rev 2's was** (~2.5 M survivors vs ~2.07 M covered pixels). ⚠️ Rev 4's entry here called the estimator *"uncapped"*. |
| R12 | **The census resolution silently decides K1.** Density scales as 1/resolution². | New in Rev 2 and the one fix that survived review. | Frozen ladder + frozen decision resolution; the curve is reported at every rung; **and the achieved extent is asserted**, because OS clamping is already a recorded hazard here at 512². |
| R13 | **The most likely branch has no gate.** §11 measures no UE5 on this box. | New — and it kept re-appearing: through Rev 5 the absolute branch had the ordering rule attached to the wrong rung **and** a gate (b) that passed for its own named red mutation. | R0a's negative is re-derived over **bounded documented authorities** (launcher manifest + the registry hives recording launcher *and* source builds), with residual blindness recorded. ⚠️ Rev 4's *"enumerate fixed volumes"* is **retracted** — a recursive walk of two ~240 GB volumes inside a `cargo test`, with false positives from any stray binary. R0f′'s gate is now two-sided (§8 R0f′). |
| R14 | **A frozen file whose schedule requires it to change.** A tripwire that fires routinely carries no signal, and a routine re-record can launder a threshold edit. | **Measured in Rev 2 by inspection:** its recorded hash was *guaranteed* to break at the `corpus.arrangement` fill, before the first rung that asserted it. | The split: thresholds hashed and never edited; claim unhashed and gated by the `PENDING` sentinel. |
| R15 | **A harness asked for a quantity its algebra removes.** | **Measured:** ABBA recovers `τ` by cancelling `μ`, `γ` and `β` — exactly what an absolute reading needs. Rev 2's R0f′ assumed otherwise. | `[absolute_mode]`: its own instrument, its own pre-registered ceiling, and the honest statement that absolute mode is ~2.5× weaker. |
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
  Rev 2 refuses to leave it ungated (§8 R0a, §8 R0f′, §9 clause 2).
* **Repo size:** `.git` 24.6 MB; all tracked assets under `crates/boyko_app/assets/` total 1.07 MB.
  No `.gitattributes` — **Git LFS is not configured**. No `LICENSE` file at the repo root.
* **Content today:** the VB fixtures render five instances of one `uv_sphere(radius, 28 stacks,
  40 slices)` at 512×512 (`sv0_scene/mod.rs:56-69`, `:162`). Twenty-four golden pins exist; two
  carry `sha256_hwrt = "PENDING"`.
* **Shaders:** 16 committed VB `.spv` are perturbed by a `vb_id` re-encode; 10 have a re-DXC gate.

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

**These anchors are NOT machine-checked, and that is now stated instead of denied.**
`tests/internal_docs_anchors.rs` gates the three navigation documents; adding this plan was
attempted and reverted, because the plan cites bare basenames in prose (`` `mesh_assets.rs:252` ``)
while the gate binds an anchor to the nearest resolvable path link. Measured on the attempt: 83
"stale" of 146, dominated by misbindings rather than rot. **Converting this document's citations to
the link form is the named follow-up that would let the gate hold the promise this sentence used to
make.** Until then, treat every number below as evidence to re-derive, not as a verified fact.

**Ingest / mesh:** `crates/boyko_render/src/loaders/obj.rs:13` (default vertex colour), `:55`
(`ObjMeshLoader`), `:60` (`EXTENSIONS = &["obj"]`), `:94-96` (dedup + `generate_tangents`) ·
`crates/boyko_render/src/mesh.rs:81-100` (`Vertex`), `:103-104` (`VERTEX_STRIDE == 64`, static
assert), `:124` (`U16_INDEX_VERTEX_LIMIT`), `:137-186` (`MeshGpu`), `:169` (`geometry_slot`),
`:193` (`type Cpu = MeshData`), `:237` (single `LoaderEntry`) ·
`crates/boyko_render/src/mesh_assets.rs:238-243` (`build_mesh_gpu` signature), `:259-263` (index
width), `:290` (**stale** `VB_IMPLEMENTED == false` comment), `:295-305`
(`MemoryLocation::HostVisibleCoherent`), `:529` (`register_mesh` passes `None`), `:619-631`
(`MeshAssetsVbExt`), `:647` (`register_mesh_vb` trait decl; impl at `:669`) ·
`crates/boyko_render/src/gpu_upload.rs:41-61` (`GpuUpload for MeshGpu`; `type Aux =
MeshGeometryTableSlot` at `:50`; **the threaded call at `:59`**).

**Geometry table:** `crates/boyko_render/src/mesh_geometry_table.rs:17-27` (module doc),
`:66` (`VB_GEOMETRY_RESERVED_SLOT`), `:82-93` (`MeshGeometryMeta`), `:97` (16 B stride),
`:116-118` (`tri_count`), `:140-142` (`mesh_buffer_usage`), `:400` (**stale** comment), `:413`
(`MeshGeometryTableSlot`) · `crates/boyko_rhi_vulkan/src/geometry_bindless.rs:61`
(`MESH_GEOMETRY_TABLE_CAPACITY = 4096`), `:43` (**stale** comment).

**Path resolution:** `crates/boyko_render/src/render_path_config.rs:25` (**stale** module-doc
sentence), **`:128` (`const VB_IMPLEMENTED: bool = true;`)**, `:517` (`vb_geometry_table` field),
`:890-892` (the predicate).

**Encode / decode:** `crates/boyko_rhi_vulkan/shaders/vb_geom_fetch.hlsli:516`
(`vb_geom_fetch` signature), **`:521` (`uint local_tri = raw_prim_id % tri_count;`)** ·
`vb_pack.hlsli:19` (`VB_ID_SENTINEL`) · `vb_raster.vs.hlsl:63` (flat `IID` interpolant), `:82`
(the export) · **`vb_raster.fs.hlsl:24-25` (`uint2(input.instance_id, raw_prim_id)`)** ·
includers: `vb_geo.comp.hlsl:117`/`:118`, `vb_resolve.comp.hlsl:84`/`:85`,
`vb_shade.comp.hlsl:89`/`:90`, `vb_shade_split.comp.hlsl:136`/`:137`,
`vb_classify_count.comp.hlsl:29`, `vb_classify_scatter.comp.hlsl:24` ·
`crates/boyko_rhi_vulkan/tests/vb_lit_producer_spv_sync.rs`'s `VB_LIT_PRODUCER_ROWS` (the ten
gated rows).

**Targets / readback:** `crates/boyko_rhi_vulkan/src/present/targets.rs:851-856` (`VbTargets`),
**`:868` (`COLOR_ATTACHMENT | SAMPLED` — no `TRANSFER_SRC`)** ·
`crates/boyko_rhi/src/encoder.rs:115` (`copy_image_to_buffer`) ·
`crates/boyko_rhi_vulkan/src/rhi_impl/encoder.rs:1031` (impl) ·
`crates/boyko_rhi_vulkan/src/present/frame_driver.rs:750` (no depth readback) ·
`crates/boyko_app/src/host_dump.rs:1-10`, `:67` (`BOYKO_HOST_DUMP`).

**Timing — RE-VERIFIED at Rev 3; Rev 1 and Rev 2 both carried a consistent ~10-line drift here,
i.e. anchors read from a pre-VB-P1e-H0 tree.** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs`:
`:188-194` (why collectors are separate — and the `PASS_COUNT` note), `:229` (**`VbShade = 2`**,
not `:219`), **`:242` (`VB_PASS_COUNT: u32 = 3`, not `:232`)**, `:281`/`:293-294` (the pool reset),
**`:344` (`WAIT_BIT` BLOCKS FOREVER on a pair its recorder never wrote, not `:334`)** — this one is
cited by §7's non-negotiable implementer trap and by risk R4, so the stale anchor was the most
expensive of the set — `:357` (`Sv0TimedPass`, not `:347`), **`:381` (`SV0_PASS_COUNT = 1`, not
`:371`)**.

**Harness precedent:** `crates/boyko_app/tests/sv0_deferred_term_bench.rs:20-51` (ABAB refuted by
its own null control), `:34` and `:58-62` (**the ABBA algebra — the model `m_k = μ + τ·armed + γ(fi)
+ β·k + ε` and the cancellation that makes absolute readings unavailable**, §8 R0f′), `:83-129`
(the quantisation finding), `:297-299` (**the OS-clamped-extent check**, §5.4), **`:350`
(`SV0_BENCH_SESSIONS = 3`), `:366` (`SV0_SESSION_SPREAD_MAX = 0.10`), `:378`
(`SV0_NULL_CONTROL_MAX_FRACTION = 0.10`)** — Rev 2 cited `:284` and `:312` for two of these in one
block and `:350`/`:378` in another; **the `350`/`366`/`378` set is the correct one**, and the
contradiction is direct evidence that the older block was never re-verified ·
`crates/boyko_render/src/ui/mod.rs:87` (`FRAMES_IN_FLIGHT = 2`) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch`) ·
`crates/boyko_rhi_vulkan/src/window.rs:252` (`Window::open`), `:310` (`AdjustWindowRectEx`),
`:342-352` (`BOYKO_WIN_HIDDEN` — hidden, but still created at the requested size).

> **§12's opening sentence — *"Every line below was opened or grepped while writing this
> revision"* — was FALSE in Rev 2**, systematically, across the whole Timing block. It is the
> claim this project's own standing lesson exists against (*report line numbers are lower bounds;
> grep the pattern*). Every anchor in this section was re-derived at Rev 3 by grep; the ones that
> moved are called out inline above rather than silently corrected, because a silent correction
> would leave no evidence that the blanket claim had been wrong.

**Oracles / fixtures:** `crates/boyko_app/tests/sv0_oracle/mod.rs:182-208` (`OracleVertex`,
`CoveredPixel`), `:211-256` (`Coverage`, `covered_count` at `:253`), `:279-287` (`rasterize`),
`:765-798` (`ChangedPixels`, `changed_covered_pixels`) · `crates/boyko_app/tests/sv0_scene/mod.rs:56-69`
(mesh row constants), `:149-162` (camera + `DUMP_EXTENT`), `:223` (`uv_sphere`) ·
`crates/boyko_app/tests/sv0_adequacy.rs:231-232`, `:514-515` (the shared-spawn inseparability test).

**Rev 2/Rev 3 additions, verified this session:**
`crates/boyko_rhi_vulkan/src/present/targets.rs:851-856` (`VbTargets` doc — the ring is **one
`R32G32_UINT` texel per pixel**, which is what caps §5.4's statistic (1) at 1), **`:866`
(`format: Format::R32G32Uint` — Rev 2 cited `:865`, which is `depth: 1`)**, `:868` (the usage bits,
correct) · `crates/boyko_app/tests/sv0_scene/mod.rs:162` (`DUMP_EXTENT = 512`) ·
`crates/boyko_app/tests/sv0_oracle/mod.rs:279-287` (**`rasterize` takes ONE indexed mesh and
`instances: &[[f32; 3]]` — translation-only**, which is why R0c gate (c) is scoped to the procedural
fixture and cannot reach the corpus at any ladder rung) ·
`crates/boyko_render/src/mesh_draw.rs:80-98` (`DrawBatch` — the source of the report-only
`submitted_per_covered_pixel`) · `crates/boyko_rhi_vulkan/shaders/vb_pack.hlsli:15-16`, `:19`
(`VB_ID_SENTINEL` marks a pixel the mesh raster leg never covered — the census's denominator is
mesh-covered pixels, not all pixels) ·
[`docs/VG-CAMPAIGN-THRESHOLDS.toml`](VG-CAMPAIGN-THRESHOLDS.toml) (hashed, never edited) ·
[`docs/VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml) (unhashed, `PENDING`-gated, blocks R0e).

**Corpus convention:** `crates/boyko_app/assets/pbr_fixtures/README.md:1-6` ·
`.gitignore` (`/assets/materials/*` + the `!README.md` escape) ·
`goldens/PINS.toml:15` (the `PENDING` sentinel rule), `:363-364`, `:408-409` (the two unblessed
hwrt legs) · `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:196-204` (the skip shape).

---

## 13. Open questions — VALUES / SCOPE only

Performance and architecture forks are decided with numbers in this project; the format choice
(§3.3), the census instrument (§5.3), the corpus shape (§4.2), the census resolution ladder and
K1's thresholds (§5.4) are decided above and are not listed here.

**Every question below has a field waiting for it in
[`docs/VG-CAMPAIGN-CLAIM.toml`](VG-CAMPAIGN-CLAIM.toml)**, and that file's `[gating]` table states
which rung each one blocks. The short version: **Q3 blocks R0b. Q1 blocks R0e — deliberately, and
that ordering is the P0's whole fix (§0.1).** Q5 blocks only the final rung; nothing blocks R0a,
R0c or R0d.

**Q1 moved earlier between Rev 2 and Rev 3, and it is the one schedule cost this revision
knowingly accepts.** Rev 2 let the claim be written at R0f — after the floor was measured — which
is exactly the defect the claim file was created to fix. Answering Q1 before R0e runs is what makes
the inequality falsifiable, so the question is now on the critical path at rung five rather than
rung six. Everything up to and including the census is unblocked by it.

1. **If K2 fires, what replaces the goal?** *"Faster than Nanite"* becomes *"N ms at quality Q on
   corpus C"* — the owner sets N, Q and C. This is the single most consequential question in the
   document and it must be answered at rung one, not month six.
2. **Third-party dependency policy for the importer.** §3.3 decides *glTF, in-house*. If the owner
   will accept a third-party glTF/JSON crate, the decoder shrinks substantially — but the
   workspace's demonstrated posture is fully in-house (raw-FFI Vulkan, in-house PNG/zlib/DEFLATE).
   The same question recurs, far more sharply, for the offline builder at R4/R5.
3. **Corpus provenance and licence.** Who selects and licenses the high-poly assets, and is a
   fetched-and-gitignored payload with pinned hashes acceptable as the permanent arrangement?
   Without an answer, R0b cannot author `CORPUS.toml`.
4. **Bless bandwidth.** How many byte-moving rungs per week can the owner actually bless? R0 moves
   no pin, but two hwrt legs are already `PENDING` (§9 clause 5), and that number caps the width of
   every rung after R2b.
6. **If K1 comes back UNDECIDED, what happens?** R0 can refute K1 cheaply and soundly and cannot
   fire it — the upper-bound instrument is unsolved (§5.6). So `D_est < 1.0` leaves the campaign's
   premise untested rather than refuted. Proceed to R1 on an unadjudicated premise, change the
   target content class, or fund the instrument? **This is the second-most consequential question
   in the document after Q1**, and like Q1 it must be answered before the number exists, not after.

5. **Quality target.** What pixel-error budget counts as "equal quality" — our equivalent of a
   pinned `MaxPixelsPerEdge` — and is the owner the arbiter by visual eval, or do we bind to a
   metric? Note the standing lesson that image statistics have already misled this project twice,
   which argues against a metric.
