# The workflow cost plan — limits

Index: [WORKFLOW-COST-PLAN.md](WORKFLOW-COST-PLAN.md) ·
evidence: [WORKFLOW-COST-EVIDENCE.md](WORKFLOW-COST-EVIDENCE.md) ·
procedure: [WORKFLOW-COST-PROCEDURE.md](WORKFLOW-COST-PROCEDURE.md) ·
briefs: [WORKFLOW-COST-BRIEFS.md](WORKFLOW-COST-BRIEFS.md)

This file exists because the corpus's central lesson is that **every mechanism shipped at least one
false sentence about itself**. This is the residue, written before the reach.

---

## What this procedure will not catch

### L1 — defects no mutation can express

[Q1](WORKFLOW-COST-BRIEFS.md#q1--the-mutation-question) is the procedure's main instrument, and it is
silent wherever the defect cannot be written as a source edit. This is not hypothetical: EG2's own
re-derivation found **three of eight sites where the wrong-id mutation is inexpressible** — either no
second operand-derived id exists (`spawn_at_command.rs:257`, `spawn_batch_command.rs:593`) or the two
values are provably equal at the call site (`insert_command.rs:260`) — `eg2cnt_land.md` Act 1.

**Mitigation:** the pass-0 registration inventory writes `NONE` rather than silence, so an
inexpressible site is a disclosed row instead of an unexamined one.

### L2 — defects the fixture class cannot host

The corpus's sharpest class-1 finding is invisible to Q1 as written. The double-drop through a safe
`pub fn` (`eg2_refute.md` §2) was undetectable because **every gate fixture was `Copy`**, so no
planted mutation in any fixture could produce a second drop. It was found by *reading the ownership
transfer and writing a drop counter*.

**Mitigation, and it is the reason pass 2 keeps two agents rather than one:** the adversary's mandate
is to plant *and* to read. Do not degrade pass 2 to a single mutation-runner.

### L3 — the surfaces the corpus never closed

Four of the five class-2 findings outside passes 1-4 become disclosed rows earlier: **pass 0** takes
the two `ui_clock_tick` rows (its plan names the system eleven times) and **pass 1** takes
`ui_visual_sink_on_add`, which no plan text names; EG2's `migration_helpers.rs:623` re-discovery is
its own pass-2 finding restated. **The fifth is not covered by anything in this procedure.** EG2's
pass-13 re-derivation found both serialize load walks ungated, and it fired only because a repair was
underway. Under [R2](WORKFLOW-COST-PROCEDURE.md#r2--a-repair-is-re-derived-from-the-criterion-never-substituted-from-a-list),
if no repair happens, no re-derivation happens.

**Accepted, with the reason:** the finding is a coverage gap in a path outside the rung's set, it was
class 2 rather than class 1, and it shipped as a disclosure under the old workflow too — after
thirteen passes. The procedure loses nothing that was actually gained.

### L4 — doc rot, deliberately

The procedure deletes the two instruments that policed coordinates and counts. After it, nothing
guarantees that a citation resolves or that a figure in prose is current.

**Why that is acceptable at this price.** Every documented catch by the 4218-line anchor census is an
anchor **the same pass had just moved** — `eg2r3_landCode.md` (*"created by me"*),
`eg2r5_landText.md`, `eg2r6_landCode.md`. **83 % of its cap rows are caps over an empty population.**
The instrument's measured yield against pre-existing rot is not merely low; the corpus records none.
Deleting it removes a cost, not a defence.

**What replaces it, at near-zero cost:** figures are emitted by a gate at run time
([R4](WORKFLOW-COST-PROCEDURE.md#r4--never-write-a-figure-about-the-edit-set-you-are-inside)), and
claims that cannot be emitted are deleted
([R3](WORKFLOW-COST-PROCEDURE.md#r3--a-claim-that-cannot-be-re-derived-mechanically-is-deleted-not-repaired)).
A sentence that no longer exists cannot rot.

### L5 — prose a future reader needed

R3 deletes claims that cannot be mechanically re-derived. Some of those would have been useful to a
reader a year out. The procedure protects only two categories: an owner decision (which cannot become
false, only superseded) and a disclosure emitted by a gate. **Everything else is a real loss, and it
is chosen.**

The corpus supports the trade for the *instrument's* prose and says nothing either way about design
rationale. Keep rationale; keep it short; never repair it — supersede it.

### L6 — the pass-0 audit can itself be wrong

Pass 0 is the procedure's highest-leverage step and its evidence base is **one instance**
(`d26_block.md`). It was right there, and it was right in an unusually favourable setting: the gates
already existed as a written plan. A rung whose plan is thin gives pass 0 less to audit, and its
verdict is then weaker than the arithmetic in
[Plan § The cost](WORKFLOW-COST-PLAN.md#the-cost-before-and-after) assumes.

### L7 — the escalation clause moves cost to the owner

A class-1 defect surviving pass 3 becomes an owner decision rather than a pass 4. That is what
happened in the corpus (`a1r3_ruling.md` FORK B) and it worked — but it converts agent time into
owner time, which is the scarcer resource. If a rung escalates twice, the procedure is not the
problem; the rung's specification is.

### L8 — the gate over these documents breaks their own class rule, and it is admitted anyway

`tests/workflow_cost_docs.rs` gates this set: heading-slug collisions, citation resolution, and the
[provenance table](WORKFLOW-COST-EVIDENCE.md#the-provenance-table-re-run-by-a-gate). Its citation
check **fails the [class rule](WORKFLOW-COST-PROCEDURE.md#the-ratios-and-the-rule-that-holds-them)**
— insert a line above `migration_helpers.rs:623` and the gate reds over a change that altered no
behaviour, which is exactly the property that condemns `ui_a1_source_census.rs`.

**Admitted, with the reason and the exit condition.** The provenance half has a measured yield
against **pre-existing** wrong figures — six, found while it was being written — where the anchor
census's measured yield against pre-existing rot is zero (L4). The citation half is the part that
will rot. **If the citation check ever costs a pass to repair, delete the citation check and keep
the provenance table.** That is the trade this set argues for everywhere else, applied to itself.

**And its exemption list is the thing to watch.** `CITATION_EXEMPTIONS` is where a coordinate goes
when it cannot resolve — today two entries, each with a reason, and asserted *exact* so an unused
one reds. A growing exemption list is this corpus's residue class arriving in a new carrier; it is
not a defect in itself, and it is the number to read before trusting the gate.

**It produced a class-3 defect on its first landing, and the fix is in the gate, not in the census
it broke.** Its first draft wrote coordinates in the `<name>.rs:<line>` spelling inside its own doc
comments. `tests/internal_docs_anchors.rs` binds that spelling wherever it occurs in a Rust source
and re-derives its own totals from the count, so the new file moved that census's bound-citation
figure from **242 to 244** and pushed its dead-path cap from **5 to 6** — `cargo test -p
boyko-engine --test internal_docs_anchors` was **green with the file absent and red with it
present**, run both ways. Two options existed: re-bless the other census's caps, or stop writing the
spelling. The second was taken, because the first is
[R4](WORKFLOW-COST-PROCEDURE.md#r4--never-write-a-figure-about-the-edit-set-you-are-inside)'s exact
failure mode — a figure moved by the edit that reports it. **The gate written against this campaign's
signature defect produced one on the way in.** That is the fixed point this document set is about,
and it is recorded rather than smoothed.

---

## What in this set is an estimate rather than a measurement

Listed because this document set is about false claims, and an unlabelled estimate is one.

| claim | status |
|---|---|
| line counts, cumulative diff growth, test counts, comment/code splits | **measured** for this document (`git show --numstat`, `git diff --numstat`, `wc -l`, `grep -c`, a comment classifier over added lines) |
| agent counts | **measured, and the word names six quantities.** Launched / returned / failed / superseded come from the harness journal, slots from the run records and the journal agreeing; the *intended* count the pass tables print comes from the scripts. Which figure uses which is stated at each site: [Evidence § What "agent run" means](WORKFLOW-COST-EVIDENCE.md#what-agent-run-means--six-quantities-one-name). **The harness writes two carriers independent of the workflow source** — the per-event journal and the per-run record — and the slots column is the one figure computed from both and asserted equal (rows P38-P41). Both cover only workflow-launched agents: an agent the orchestrator launched directly through its own Agent tool appears in neither record and in no figure here, so the two agreeing says they read the same population correctly and says nothing about whether that population is complete |
| per-pass wall clock | **measured, ±5 min, and a floor.** Script mtime → last-report mtime; includes queueing. A1 pass 1 excluded as unreliable (mtime arithmetic returns 14 h 56 m). It is a floor because a run whose agents all failed writes no report and therefore has no clock: EG2 pass 11's [403 run](WORKFLOW-COST-EVIDENCE.md#the-eleven-agents-that-produced-nothing--the-cost-this-set-never-counted) is in no duration in this set, including the `~59 h` |
| the class 1/2/3/4/5 classification | **judgement.** Every class-1 and class-2 assignment is individually cited so it can be checked; the aggregate counts for classes 3, 4 and 5 are not re-derived here and are inherited from the pass audit at roughly ±10 % |
| **"≈4.4-5.9× fewer agent runs"** | **arithmetic on agent counts** (57 and 61 *launched*, measured from the journal, against 10 and 13 *intended* at one repair round and the measured landing sizes), **not a measured before/after.** The procedure has never been run, so its side of the ratio has no journal and is intent; the corpus's launch-to-intent overhead of 1.076 is applied to the after-arms to make the two sides comparable, and without it the range reads 4.7-6.3×. It replaces a "≈4-6×" on the script-text count, an earlier "≈4-5×" computed against a count blind to thunk-launched agents, and an original "at least 6×" that assumed one landing pass per rung — a ceiling [the corpus contradicts](WORKFLOW-COST-PLAN.md#the-cost-before-and-after) |
| the landing ceiling **1351 insertions** | **measured** — EG2 pass 8's inherited-insertion delta, `4429 → 5780`, read from the pass-9 brief's PREAMBLE. That it generalises to *"a landing agent cannot reliably exceed it"* is an **inference from one maximum**, not a capacity measurement, and it is what the whole agent-run arithmetic now rests on |
| **"≈1.3-1.8× less wall clock"** | **estimate.** Sized from the corpus's own measured pass durations (2-agent passes 22 m-**1 h 57 m**; 4-agent passes 1 h 27 m-**5 h 40 m**). It assumes the procedure's passes cost what the corpus's passes cost, which is unverified — and the corpus does not support the assumption strongly: its 5-agent passes ran under its 4-agent ceiling, so agent count does not order duration. It replaces an earlier "≈2.2-3.0×", whose 4-agent ceiling of 2 h 58 m omitted `a1r9.js` (4 agents, 5 h 40 m) from its own bucket and whose 2-agent ceiling of 1 h 12 m was taken over the rung passes only, leaving out `split-final.js` (2 agents, 1 h 57 m); and before that an "≈4×" computed at one landing pass |
| **the three-round cap on pass 3** | **judgement over a measurement.** Two repair rounds exhausted both lanes (A1's class-1 findings end at pass 4, EG2's at pass 3, and no pass in 22 further tries found another). Three is one round past the measured need. Nothing in the corpus tests what a third round would have found, because no lane needed one |
| **"−61 % instrument lines with zero production defects lost"** | the **−61 %** is measured (13,823 → 5,448). The **"zero lost"** is an inference from the corpus record that the removed 8375 lines caught no production defect. It is a claim about the past, not a guarantee about the future |
| the 3-in-5 bad-fix rate | **not used as evidence anywhere in this set.** It appears only as [Error 1](WORKFLOW-COST-BRIEFS.md#error-1--supplying-a-number-the-orchestrator-did-not-measure) — an example of an unmeasured number entering four briefs |

**Two figures from the input briefs did not survive re-derivation and were corrected:**

* A1's production-code delta. Gross `493` is right; the **non-comment** figure is **40**, of which 22
  are a `const _: () = assert!` macro, leaving a release-behaviour delta of **five lines**. Two
  earlier readings gave 37 and 19+24; the classifier run for this document gives 18+22.
* The cap-row census figure. It is **66/54/12 at pass 10 and 84/70/14 at pass 11** — it moved because
  the passes that wrote it added ledgers. Any single reading quoted without its pass is a pre-state.

---

## The honest counter-case

The strongest argument against this procedure is that the corpus's later passes were not *worthless*
— they were **cheap insurance whose premium happened to be visible**. Two facts support that reading
and are recorded here rather than argued away:

1. **Pass 13 of EG2 found two genuinely ungated production sites** (`eg2cnt_land.md`). A procedure
   that stops at pass 3 does not find them unless a repair triggers R2.
2. **Roughly 38 % of the owner-facing prose is genuine escalation** — 709 of 1881 lines — and none of
   it came from the record loop. The tripwire in
   [Procedure § What to stop writing](WORKFLOW-COST-PROCEDURE.md#what-to-stop-writing) must not be
   applied to it.

The counter-argument to the counter-case is the arithmetic: **67 of 128 agent launches, 60 of 117
agent returns, and ~34 h 40 m of ~59 h produced nothing of either kind**, and — since an agent that
never returned cannot have found anything — **11 of those launches produced no report at all**. The
insurance was bought at a price that also
manufactured the campaign's most dangerous artifact — a live probe in tracked production source over
two certifications of its absence (`a1r9_refute.md` F1). **The procedure trades a small, real loss of
late-pass coverage for a large, measured reduction in self-inflicted defect.** That is the trade, and
it is a trade, not a free win.
