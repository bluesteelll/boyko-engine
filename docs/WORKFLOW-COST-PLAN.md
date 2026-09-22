# The workflow cost plan — index

**Status:** proposed procedure, 2026-08-29. Written against two measured rungs: UI **A1**
(`D:/wt/ui`, `e7a16fd9` → `2a10f3a4`) and reflection **EG2** (`D:/wt/reflect`, `f7c46c76` →
`d272e1fd`; it was an uncommitted working tree while this set was drafted — see
[Evidence § How these numbers were taken](WORKFLOW-COST-EVIDENCE.md#how-these-numbers-were-taken)).

**The question this answers, in the owner's words:** *"How do we fix the workflow so that it is
CHEAP, there are no endless pointless rewrites, and everything still works correctly."*

**The parts.**

| file | contains |
|---|---|
| this file | the problem, the procedure as a checklist, the cost arithmetic |
| [WORKFLOW-COST-EVIDENCE.md](WORKFLOW-COST-EVIDENCE.md) | the pass audit and the line ledger, with how each number was taken |
| [WORKFLOW-COST-PROCEDURE.md](WORKFLOW-COST-PROCEDURE.md) | the pass structure, the stopping rule, the repair discipline, what to stop writing |
| [WORKFLOW-COST-BRIEFS.md](WORKFLOW-COST-BRIEFS.md) | the brief templates and the orchestrator's own rules |
| [WORKFLOW-COST-LIMITS.md](WORKFLOW-COST-LIMITS.md) | what this procedure will not catch, and the estimates in this set that are not measurements |
| `tests/workflow_cost_docs.rs` | the gate over these five files: heading-slug collisions, citation resolution, and the [provenance table](WORKFLOW-COST-EVIDENCE.md#the-provenance-table-re-run-by-a-gate) re-run against the tree. It breaks this set's own class rule; [L8](WORKFLOW-COST-LIMITS.md#l8--the-gate-over-these-documents-breaks-their-own-class-rule-and-it-is-admitted-anyway) says why it is admitted and when to delete half of it |

---

## The problem in one page

Two rungs landed across two days. Each ran as a chain of 4-agent workflows — land, land-record,
independent re-run, adversarial refutation.

**What it cost** (measured; method in
[Evidence § How these numbers were taken](WORKFLOW-COST-EVIDENCE.md#how-these-numbers-were-taken)):

| | passes | runs | agents **launched** | agents **returned** | wall clock |
|---|---|---|---|---|---|
| A1 | 13 | 13 | 57 | 51 | ~27 h 10 m (passes 2-13; pass 1's clock is unreliable) |
| EG2 | 14 | 15 | 61 | 57 | ~28 h 11 m |
| plan-split refactor | 3 | 3 | 10 | 9 | ~3 h 40 m |
| **total** | **30** | **31** | **128** | **117** | **~59 h** |

**Launched** and **returned** are counted from the harness journal, not from the workflow scripts,
and they are different quantities that this set spent three rounds calling one name: a launched agent
costs tokens, a returned agent delivers work, and 11 agents did the first without the second.
Definitions, the per-lane counts and what could not be attributed:
[Evidence § What "agent run" means](WORKFLOW-COST-EVIDENCE.md#what-agent-run-means--six-quantities-one-name).
**This table is the cost side, so it is priced on launched.**

**What it produced.** A1's remediation commit `2a10f3a4` is 9543 insertions. Under `src/`, after
subtracting comment and blank lines, it adds **40 lines** — and 22 of those are a compile-time
`const _: () = assert!` macro in `gather.rs`, which is an instrument that happens to live in `src/`.
The entire release-behaviour change of twelve remediation passes is this, in `animation.rs`:

```rust
if !(duration_ms.is_finite() && duration_ms.is_sign_positive()) {
    invalid_tween_duration(duration_ms);
    return;
}
```

plus one line moving the completion boundary. Everything else added under `src/` is comments
(279 of animation.rs's 298 added lines) or a `#[cold]` helper whose body is a `debug_assert!`.

**Where the passes actually went.** Two files account for **8375 lines** — A1's
`crates/boyko_ui/tests/ui_a1_source_census.rs` (4157) and EG2's `tests/internal_docs_anchors.rs`
(+4218). They were the subject of **16 of the 30 workflows**. Between them they caught **zero**
production defects. Each shipped at least one false sentence *about itself* on every pass, each
false sentence was a finding, each finding produced a repair, and each repair moved the coordinates
the other census then caught.

**The loop, named.** A defect is found → a repair is written → the repair asserts something false →
the next pass finds *that*. The corpus's own briefs assert a "measured rate of 3-in-5 repairs
introducing a NEW false statement"; one of the orchestrator's own later briefs calls that figure
"folklore from one campaign"
([Briefs § Error 1](WORKFLOW-COST-BRIEFS.md#error-1--supplying-a-number-the-orchestrator-did-not-measure)).
Whatever the exact rate, the direction is not in doubt: **code repairs face a compiler and prose
repairs face nothing**, so the loop can only terminate on a rule, never on exhaustion.

**When it was actually safe to stop.** The last finding of a wrong *production behaviour* is
`a1r2_refute.md` §1 (A1 pass 4) and `eg2r_refute.md` §§1-3 (EG2 pass 3). Every pass after that found
gate blind spots, then instrument blind spots, then prose about instrument blind spots. **17 of the
30 passes — 67 of 128 agent launches (52 %), 60 of 117 agent returns (51 %), ~34 h 40 m of ~59 h,
59 % of the wall clock — produced neither a class-1 nor a class-2 finding: nothing about the engine
at all.** Both fractions are given because the claim is work delivered against cost incurred, and
those have different denominators. Enumerated pass by pass at
[Evidence § The yield curve](WORKFLOW-COST-EVIDENCE.md#the-yield-curve). *(Three earlier drafts read
"19 of the 30 passes — 67 of 116 agent runs", then "18 … 63 of 106", then "17 … 60 of 119". The first
two denominators came from a counting command blind to a third of the runs, the third from a count of
the script text, and none of them was a count of agents; the pass set additionally scored A1 pass 3
as a zero because its adversary's report had been filed under pass 4. The set is the 17 passes the
evidence enumerates.)*

---

## The procedure, as a checklist

Full text with justification and cost-saved per step:
[Procedure § The pass structure](WORKFLOW-COST-PROCEDURE.md#the-pass-structure).

- [ ] **Pass 0 — GATE AUDIT. One agent. No code exists yet.**
      For every gate the plan names: name the production mutation it must catch, and say whether the
      gate *as specified* observes it. Enumerate every system, public fn and hook the rung will
      register, and name the gate covering each — or write `NONE`. Output is a refusal or an
      approval with a corrected gate list.
      *Precedent: `d26_block.md` did exactly this and found six of EG2's ten gates observing
      something other than what they claim. EG2 was refused rather than written and then unwritten.*

- [ ] **Pass 1 — LANDING. One implementer per ≤1351 insertions of retained landing (+ one doc agent
      only if the rung carries an owner decision).** For every gate it lands, the implementer applies
      the red mutation with `Edit`, runs the gate, and pastes the failure output. Then it plants the
      same defect class in **every** branch of every system in the rung's set — including branches
      the fixture does not reach — and records the GREENs as well as the REDs.
      *1351 is the corpus's largest single-pass landing (EG2 pass 8, `4429 → 5780`). Both measured
      rungs exceed it and would take 2 and 4 landing passes.*
      *This pass, not pass 0, is where `ui_visual_sink_on_add` is caught: it is a new `on_add` hook
      the rung's own code registers, named in no plan text. `ui_clock_tick` is not like it — pass 0
      reaches that one; see
      [Evidence](WORKFLOW-COST-EVIDENCE.md#a1s-class-1-and-class-2-findings-individually).*

- [ ] **Pass 2 — VERIFY PAIR. Two agents, in parallel, that do not see each other's output.**
      (a) *Re-run*: reproduce every number in the landing from scratch. (b) *Adversary*: plant
      mutations and try to make gates green. Their overlap decides whether pass 3 fires.

- [ ] **Pass 3 — REPAIR + VERIFY. Conditional; repeats while each round strictly reduces the class-1
      count, hard cap 3 rounds; 3 agent runs per round.**
      Fires only if pass 2 returned a wrong production behaviour or a gate that does not observe the
      defect it claims. Its subject is the repair diff alone. A round that does not strictly reduce
      the class-1 count does not get another.
      *"At most once" — what an earlier draft said — leaves no terminating state for the bugs in the
      repair itself, and A1 pass 3's repair shipped a class-1 regression that pass 4 found.*

- [ ] **STOP.** Ship, escalate, or withdraw.
      **Ship** when the last landing changed no non-comment line under `src/` **and** the verify pair
      reported no wrong claim about production behaviour. Check it with
      `git status --untracked-files=all` **first**, then a **content** diff — not `--numstat` — and
      then the **non-comment classifier over that diff**, which is the only one of the three that
      evaluates the predicate.
      **Escalate** to the owner a **VALUES or SCOPE** question that survives pass 3, and only that.
      **Withdraw** the rung if a plain class-1 defect is still open **when the repair loop
      terminates — by the cap of three rounds, or by a round that did not strictly reduce**; do not
      ship it and do not hand it to the owner.
      *Precedent: that is what actually ended A1's chain. `a1r3_ruling.md` FORK B ruled the immortal
      long-duration row a VALUES call, filed it, and did not repair it — the corpus's only escalation,
      and a VALUES call, which is why the clause claims no more than that.*

Everything still open at STOP is written once as a **disclosure**, never repaired into a pass 4:
[Procedure § The stopping rule](WORKFLOW-COST-PROCEDURE.md#the-stopping-rule).

---

## The cost, before and after

**Before — measured.** A1: 13 passes / 13 runs / **57 agents launched**. EG2: 14 passes / **15 runs**
/ **61 agents launched**. Counted from the harness journal
(`<session>/subagents/workflows/wf_<id>/journal.jsonl`, one line per agent event), pinned by
provenance rows P31-P37, and defined at
[Evidence § What "agent run" means](WORKFLOW-COST-EVIDENCE.md#what-agent-run-means--six-quantities-one-name).
**The factor prices what was spent, so it divides launched agents.**

*(This is the third instrument in three rounds and the first that counts agents rather than source
text. The set began with `grep -c "await agent("` → 49 and 48, which is blind to thunk-launched
fan-outs; corrected to call sites plus `.map()` arity → 53 and 57, which is right about what the
scripts intend and silent about what ran; the journal says 57 and 61. Rows P05, P06, P07 and P26
still pin the script-text figures, and are now labelled as measuring intent.)*

**After — the procedure's arms.** Pass 0 is 1 run; pass 1 is **L** landing runs plus a doc agent when
the rung carries an owner decision; pass 2 is 2; pass 3 is **3 per round**, up to 3 rounds.

| | passes | agent runs |
|---|---|---|
| small rung (L = 1), clean | 3 | 1 + 2 + 2 = **5** |
| small rung, one repair round | 4 | 5 + 3 = **8** |
| small rung, the cap (3 rounds) | 6 | 5 + 9 = **14** |

**L is measured, not chosen.** The corpus's largest single-pass landing is **1351 insertions**
(EG2 pass 8, `eg2r6.js`, `4429 → 5780`, read from the pass-9 brief's PREAMBLE). Under the class rule
the two measured rungs retain **2425** insertions (A1: 493 production + 1932 instrument) and **5136**
(EG2: 1620 + 3516) — so **L = 2 for A1 and 4 for EG2**, and 3 and 6 once the tripwire's prose
allowance (≤ production + 200) is added. Sizing pass 1 at one implementer per rung was a ceiling the
corpus contradicts, and it was what the "at least 6×" headline rested on.

**The factor, on the two rungs actually measured, with one repair round** — `1 + L + 1 doc + 2 + 3`:

| lane | launched | L = 2 / 4 → 9 · 11 runs | L = 3 / 6 → 10 · 13 runs |
|---|---|---|---|
| A1 | 57 | **6.3×** | 5.7× |
| EG2 | 61 | **5.5×** | 4.7× |

**Stated as it measures: ≈4.7× to 6.3× fewer agent runs on a rung the size of these two, with one
repair round.** A small rung that lands in one pass keeps the 7.1×-7.6× of the table above; the
three-round cap on a large rung is the floor, at 57/16 = 3.6× and 61/19 = 3.2×, still a reduction but
no longer the headline.

**The ratio's two sides are not the same quantity, and the mismatch flatters the proposal.** The
numerator is *launched* agents — what the corpus actually spent, failures and re-launches included.
The denominator is the procedure's *intended* arms, because the procedure has never been run and has
no journal. The corpus's own launch-to-intent overhead was 128 / 119 = **1.076**: for every 100
agents a script asked for, 108 processes started. Applying that overhead to the after-arms — which is
the like-for-like comparison — gives **5.9× · 5.3×** for A1 and **5.1× · 4.4×** for EG2, i.e. a range
of **≈4.4× to 5.9×**. That is the more conservative number and the one
[Limits](WORKFLOW-COST-LIMITS.md#what-in-this-set-is-an-estimate-rather-than-a-measurement) carries.
*(It is also, exactly, the range this document published last round on the script-text count — the
two errors were of the same size and opposite sign, which is a coincidence and not a check.)*

*(Earlier drafts: "at least 6×" at one landing pass per rung, then 3.7×-5.4× on the `await`-blind
count, then 4.4×-5.9× on the script-text count. Each correction flattered the proposal, which is not
a reason to state it any softer or any harder than it measures.)*

Wall clock is an **estimate, not a measurement** — see
[Limits § What in this set is an estimate](WORKFLOW-COST-LIMITS.md#what-in-this-set-is-an-estimate-rather-than-a-measurement).
Sized from the corpus's own measured pass durations — **2-agent passes ran 22 m-1 h 57 m; 4-agent
passes 1 h 27 m-5 h 40 m** — and at the pass counts above, with pass 0, each landing pass and the
verify pair at the 2-agent ceiling of 1 h 57 m and the repair round at the 4-agent ceiling of
5 h 40 m, the worst case is **1 h 57 m + 3 × 1 h 57 m + 1 h 57 m + 5 h 40 m = ~15 h 25 m for A1**
(L = 3) and **~21 h 16 m for EG2** (L = 6), against their measured 27 h 10 m and 28 h 11 m:
**≈1.3× to 1.8×**. The wall-clock saving is the weakest of the three and is stated last for that
reason.

*Both ceilings in that sentence were wrong before this round, and both were wrong in the direction
that flattered the estimate.* The 4-agent ceiling was written as 2 h 58 m while `a1r9.js` is a
4-agent pass at **5 h 40 m** — the pass this set elsewhere calls *"the campaign's most expensive
single pass"*. The 2-agent ceiling was written as 1 h 12 m over the four rung passes only, while
`split-final.js` is a 2-agent pass at **1 h 57 m** (`22:20` → `00:17`, from the same mtimes
[the yield curve](WORKFLOW-COST-EVIDENCE.md#the-yield-curve) already prints). At the true ceilings
the headline falls from ≈2.2-3.0× to ≈1.3-1.8×. It is also worth reading what the buckets do **not**
show: the **seven** 5-agent passes ran 0 h 56 m-2 h 42 m, i.e. entirely **under** the
4-agent ceiling, so agent count does not order wall clock and this remains an estimate rather than a
measurement.

*(An earlier draft read "the six 5-agent passes with a usable clock". There are seven among the 31
lane runs — three in A1, three in EG2, and `ui-plan-split.js`, which the "usable clock" qualifier was
quietly dropping; row P43 counts them from the run records. The conclusion does not depend on the
correction, and it survives a change of clock as well: on the harness's own `durationMs` the seven
span 0 h 56 m-2 h 42 m against the 4-agent passes' 0 h 18 m-2 h 57 m, so the 5-agent bucket sits
inside the 4-agent one on **both** clocks. That the two clocks disagree at all —
[Evidence](WORKFLOW-COST-EVIDENCE.md#what-the-lanes-cost-in-tokens-which-no-figure-in-this-set-had-before)
records `a1r9.js` at 5 h 40 m by mtime and 0 h 20 m by `durationMs` — is why the durations here stay
an estimate and why no provenance row measures one.)*

**What buying that back costs.** You do not build `ui_a1_source_census.rs` (4157 lines, born at 1403
at pass 8, rebuilt five times) or the +4218 lines of anchor census. You ship the same disclosed
limits — `ui_clock_tick` and `ui_visual_sink_on_add` uncovered, `migration_helpers.rs:623` ungated,
the two serialize load walks ungated — **earlier and shorter**. Every one of those shipped anyway
under the old workflow, after the passes that found them.

---

## Provenance of this document set

Every figure here was re-derived for this document from the artifacts, not copied from a summary:

* Line tables — `git show --numstat 2a10f3a4` and `git show --numstat e7a16fd9` in `D:/wt/ui`;
  `git diff --numstat f7c46c76 d272e1fd` in `D:/wt/reflect`. *(The set was drafted against
  `git diff --numstat HEAD` over an uncommitted EG2; that command no longer measures it.)*
* Comment/code split — a classifier over the added lines of each file (blank, `//`, `/* */`, code),
  run for this document. Results in
  [Evidence § The line ledger](WORKFLOW-COST-EVIDENCE.md#the-line-ledger-measured-not-quoted).
* Agent counts — `grep -c "agent("` per workflow script, plus each `.map()` fan-out's arity.
  *(The earlier `grep -c "await agent("` is blind to thunk-launched agents and under-counted the
  corpus by 13.)*
* Wall clock — script mtime → last-report mtime, ±5 min, includes queueing.
* Cumulative diff growth — quoted verbatim from each brief's own PREAMBLE.
* Corpus claims — quoted from the 108 report files in
  `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/3a949a0f-63dd-4304-b51e-cf1897780494/scratchpad/`,
  cited by basename.

Two figures from the input briefs **did not survive re-derivation** and are corrected here:
A1's production-code delta (493 gross → **40 non-comment**, of which 22 are an assertion macro), and
the cap-row census figure (**66/54/12 at pass 10, 84/70/14 at pass 11** — it moved because the passes
added ledgers, which is itself the defect class this plan is about).
