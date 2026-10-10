# The workflow cost plan — briefs, and the orchestrator's own rules

Index: [WORKFLOW-COST-PLAN.md](WORKFLOW-COST-PLAN.md) ·
evidence: [WORKFLOW-COST-EVIDENCE.md](WORKFLOW-COST-EVIDENCE.md) ·
procedure: [WORKFLOW-COST-PROCEDURE.md](WORKFLOW-COST-PROCEDURE.md) ·
limits: [WORKFLOW-COST-LIMITS.md](WORKFLOW-COST-LIMITS.md)

Much of the cost traces to what the orchestrator asked for. This file is the ask.

---

## The four questions

These go in the pass-0 and pass-1 briefs. Each is followed by what it catches in the corpus and how
many passes early.

### Q1 — the mutation question

> **For every gate this rung lands: name the production-source mutation it is meant to catch, apply
> that mutation with `Edit`, run the gate, and paste the failure output. Then plant the same defect
> class in *every branch of every system in the rung's set* — including the branches the fixture does
> not reach — and record the GREENs as well as the REDs.**

**This is not a guess. One pass in the corpus asked it, and it is the highest-yielding pass of either
rung.** EG2 pass 0's audit asked exactly this of the *plan's* gates, before a line of code existed,
and returned *"six of its ten gates observed something other than what they claim"* (`d26_block.md`).
The rung was refused rather than written and then unwritten. A1's pass 1 did not ask it and committed
`e7a16fd9` with a Miri gate that cannot fail and an allocation gate blind to half its subject.

**Catches at pass 0 or pass 1:** A1 `a1_attack.md` §3, §4, §6 and `a1r7_refute.md` F3
(`ui_visual_sink_on_add`) at **pass 1**; `a1r4_refute.md` §9 and `a1r6_refute.md` §6
(`ui_clock_tick`) at **pass 0**, whose plan names the system eleven times — **all six of A1's
class-2 findings, three of them 6 to 8 passes early.** *(An earlier draft read "five of them 4 to 8
passes early". Counted against
[the A1 findings table](WORKFLOW-COST-EVIDENCE.md#a1s-class-1-and-class-2-findings-individually), the
earliness is: the three `a1_attack.md` findings land at pass 2 and Q1 catches them at pass 1, so
**1** pass each; `a1r4_refute.md` §9 lands at pass 6 and Q1 catches it at pass 0, so **6**;
`a1r6_refute.md` §6 at pass 8, so **8**; `a1r7_refute.md` F3 at pass 9 caught at pass 1, so **8**.
Three at one pass early, three at six to eight. The saving is the same six findings either way; only
the claim about how far ahead they were caught was wrong, and it was wrong in the direction that
flattered Q1.)*
EG2 `eg2_refute.md` §5, §6 · `eg2_rerun.md` §8.2 · `eg2r2_refute.md` §2 · `eg2r7_refute.md` §7 —
**five of thirteen**, with `d26_block.md`'s six as the proof of concept.

It also pre-empts the repair chains those findings launched: A1 passes 5-7's floor/window work, EG2
pass 4's `target_archetype_id`.

### Q2 — the gate-exit question

> **Run every gate `CLAUDE.md` names, unpiped, with `$?` on the next line, and paste both.**

**Catches — and this question, not Q1, is what caught the corpus's two gate-found class-1 defects.**
`a1_attack.md` §1: `cargo clippy --workspace --all-targets -- -D warnings` was **RED at `e7a16fd9`**
(`EXIT=101`), and its single red left `boyko-render`'s whole lint surface unmeasured
(`grep -c 'Checking boyko-render'` = **0**). `eg2_refute.md` §4: `cargo test -p boyko-ecs
--all-targets --no-fail-fast` was **RED at the EG2 landing** — `ECS_ALLTARGETS_EXIT=101`, three
targets failing — over a permanent project-wide diagnostic regression the ballot had not priced.
Cost to ask: zero. It was not asked at A1 pass 1. See
[Evidence § what did catch things](WORKFLOW-COST-EVIDENCE.md#what-did-catch-things).

The unpiped clause is load-bearing and is already in this project's memory: `| tail` eats cargo's
exit code.

### Q3 — the standing-sentence question

> **For every doc sentence you leave standing over code you changed, state the measurement that makes
> it true today. A sentence you cannot measure is deleted, not reworded.**

**Catches:** `a1_attack.md` §2, §5, §7, §8 and `eg2_refute.md` §2 — the double-drop. That one is
exact: the ownership transfer **was** learned while writing gate 10 and was written into the *test
helper* — `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs:3054-3056` at `d272e1fd`,
the doc comment of `spawn_data` inside the `#[cfg(test)] mod` that opens at `:2992`, reading
*"its bytes are memcpy'd into the pool, which takes ownership — dropping the local too would count a
drop the engine never performed"* — instead of into the public doc comment, so a caller keeping its
own live value double-drops through a safe `pub fn`. Every gate fixture was `Copy`, so no gate could
have seen it.

### Q4 — the residue question

> **Do not write a positive claim about the reach of an instrument you just wrote. Write the residue
> — the list of what it cannot see — and make the gate print it.**

This is the rule that attacks the class that consumed the campaign. Both rungs' final classifications
are literally its violation — *"(c) — a positive assertion that is still false"* (`a1r10_refute.md`,
`a1r9_refute.md`) — and **both shipped within one pass of converting the assertions into residue**
(`eg2fin_verify.md`, `a1pre_attack.md`).

Corollary, five recurrences: **never write a figure about the edit set you are inside.** See
[Procedure § R4](WORKFLOW-COST-PROCEDURE.md#r4--never-write-a-figure-about-the-edit-set-you-are-inside).

---

## The landing brief template

```
RUNG:      <name>. One sentence: what lands.
WORKTREE:  <abs path> @ <branch>, HEAD <sha>. Never `git checkout/stash/restore/clean/reset`.
TOOLCHAIN: export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu

APPROVED GATE LIST  (from pass 0 — this is the whole list; do not add to it)
| gate | production mutation it must catch | mutation site |

REGISTRATION INVENTORY (from pass 0)
| system / fn / hook this rung registers or modifies | covering gate, or NONE |

OBLIGATIONS
- Q1: red-proof every gate above. Paste the failure output.
- Q1: plant the defect class in every row of the inventory. Record GREENs as well as REDs.
- Q2: run every gate CLAUDE.md names, unpiped, `$?` next line.
- Q3: every doc sentence left standing over changed code names its measurement, or goes.
- Q4: write the residue, not the reach. Every figure is emitted by a gate, never typed.
- Snapshot every file you touch to a path OUTSIDE the repo. Restore by `cp`, prove by `cmp`.

REPORT
- The red-proof table (gate | mutation | exit | pasted failure).
- The plant table, GREENs included.
- Gate exits, unpiped.
- The residue list.
- `git status --porcelain --untracked-files=all`, and the `cmp` proofs.

ESCALATE, DO NOT DECIDE: any VALUES or SCOPE question. Stop and hand it back.
```

### What a landing brief must NOT contain

| forbidden | corpus defect it caused |
|---|---|
| **A number the orchestrator did not measure this session** | see [Error 1](#error-1--supplying-a-number-the-orchestrator-did-not-measure) |
| **A mechanism, premise or framing asserted as established** | `a1r3_ruling.md:21` *"the brief's mechanism is wrong"*; `:44` *"Two of the brief's premises fall here"*; `:234` the brief's framing refuted; `split_refute.md:117` and `split_rerun.md:33`, independently: *"the brief's premise is inverted"* |
| **A commission for an instrument that is not a gate** | `a1r6.js` label `land:censuses` and `eg2r3.js` label `land:gate+census` commissioned the two instruments that between them cost 16 of 30 workflows and caught nothing |
| **A request to "repair" prose** | R2/R3: repairs of prose are the 51 % class. Ask for a re-derivation or a deletion, never a rewording |
| **A re-taught environment preamble** | roughly a third of each brief re-teaches toolchain shadowing, probe residue and snapshot discipline every pass. Put it in one checklist artifact and link it |

---

## The verify brief template

Two agents, same brief, launched together, neither seeing the other's output.

```
SUBJECT:  the landing report at <path>, and the tree at <worktree>.
ROLE A (re-run):   Reproduce every number from scratch. Derive BEFORE reading the landing's derivation.
ROLE B (adversary): Plant mutations. Try to make a gate green over a real defect.

BOTH:
- Do NOT fix anything. Report and restore; `cmp`-prove the restore.
- Classify every finding: 1 wrong production behaviour · 2 a gate that does not observe the defect
  it claims (landed: green over a mutation; specified: shown by audit not to observe it) ·
  3 introduced by a repair · 4 prose, no gate consequence · 5 instrument false about its own reach.
  The full definitions are [Evidence § The classes](WORKFLOW-COST-EVIDENCE.md#the-classes).
- A finding must carry: the input, the wrong output, and the command that shows it.
- An empty list is a SUCCESS. Do not manufacture class 4 and 5 findings to fill the report.
```

**That last line is a cost control with evidence behind it.** Of ~240 findings in the corpus, **13
are class 1 and 19 are class 2 — under 14 %.** The remainder is overwhelmingly the reviewer finding
something because it was asked to find something. The project's own standing note already says it:
*a critic finds because it wants to find; an empty list is a success.*

---

## What the orchestrator must stop doing

### Error 1 — supplying a number the orchestrator did not measure

**Measured** — `grep -rn "3-in-5\|3 in 5" *.js` over the workflow-script directory returns **five
lines in three files**: `a1-precommit.js:42` (*"rate of 3-in-5 repairs introducing a NEW false
statement"*), `eg2r.js:482`, and `research-record-truth.js` at `:93`, `:111` and `:144`. The last of
those is the refutation: *"The 3-in-5 figure is folklore from one campaign."*

**Three briefs carried the number as measured**, and the orchestrator's own later brief calls it
folklore. A fourth, `split-fix.js:161`, carries the *claim* with the number stripped — *"campaign has
a measured rate of repairs introducing a fresh falsehood. Assume it here."* — which is the same error
with the evidence removed, and is worse, because there is now nothing for the agent to check. The
agents then reasoned from it.

> **RULE.** A number in a brief carries the command that produced it, run this session, or it is
> written as an estimate with the word *estimate* in the sentence. No exceptions for numbers the
> orchestrator itself wrote in an earlier pass.

**Saves:** unquantifiable in the corpus, but it is the same class as the four refuted brief premises
below, which cost a ruling pass each to unwind.

### Error 2 — asserting a mechanism the agent must then refute

**Measured, in one pass's ruling alone (`a1r3_ruling.md`):** the brief's mechanism was wrong (`:21`),
two of its premises fell (`:44`), its `2.5 %` rate was measured at `5 %` (`:199`), and its
"115-day tween" framing was the wrong framing (`:234`). The ruling agent had to write *"Re-derived
from the predicate, independently of the brief's table"* (`:387`). In the plan-split, **both**
verifiers independently reported *"the brief's premise is inverted"* about line endings
(`split_refute.md:117`, `split_rerun.md:33`).

> **RULE.** A brief states the *question* and the *evidence available*, never the mechanism. Where
> the orchestrator has a hypothesis, it is labelled `HYPOTHESIS — refute or confirm`, and refuting it
> is a success, not a finding against the agent.

**Saves:** the plan-split's 3 passes / 9 agents / 3 h 40 m produced zero class-1 or class-2 findings
and spent two of them on an inverted premise.

### Error 3 — commissioning apparatus whose defects then consume passes

**Measured.** The orchestrator commissioned both censuses by name: `a1r6.js` agent label
`land:censuses`; `eg2r3.js` agent label `land:gate+census`. Together they are 8375 lines, 16 of 30
workflows, and zero production defects.

> **RULE.** The orchestrator may commission only instrument that satisfies the class rule — *a
> behaviour-preserving refactor of production source leaves it green*. A census, a cap, a coordinate
> check or a count fails that and is refused at commission time, through
> [R5](WORKFLOW-COST-PROCEDURE.md#r5--the-repair-budget), which is the one refusal path. When the
> orchestrator wants coverage of a shape rather than a behaviour, the answer is a **disclosure**, not
> a test.
> See [Procedure § What to stop writing](WORKFLOW-COST-PROCEDURE.md#what-to-stop-writing).

**Saves:** A1 passes 8-13 (**24 launched / 22 returned, 15 h 36 m**) and EG2 passes 5-14
(**38 launched / 34 returned, 19 h 26 m**) — 16 passes in **17 runs**, **62 agents launched**,
**56 returned**, ~35 h. Counted from the journal; the quantities are defined at
[Evidence § What "agent run" means](WORKFLOW-COST-EVIDENCE.md#what-agent-run-means--six-quantities-one-name).
*(An earlier draft read "16 workflows, 56 agent runs": 56 is the returned count, and it was reached
by counting the scripts, which cannot see that EG2 pass 11 ran twice.)*

### Error 4 — hand-editing documents between passes

The scratchpad holds dozens of orchestrator-side edit scripts (`patch*.py`, `docfix*.py`,
`amend1..10.py`, `fix4.py`, `anchor_fix.py`, `rederive*.py`). Each is an edit made outside a pass and
therefore outside every red-proof, every `cmp` restore proof and every verify pair. It is the same
operation as a repair, with none of [the repair discipline](WORKFLOW-COST-PROCEDURE.md#the-repair-discipline)
applied — and the corpus's most dangerous single artifact is exactly an unrestored between-pass edit:
a live `struct ZzIter` probe in tracked production source that **two reports certified absent**
(`a1r9_refute.md` F1).

> **RULE.** The orchestrator does not edit repository files. Every edit is made inside a pass, by an
> agent, under the snapshot-and-`cmp` obligation. If the orchestrator finds an error between passes,
> it goes into the next brief as a finding, not into the file as a patch.

**Saves:** A1 pass 11's 5 h 40 m — the campaign's most expensive single pass — whose top finding was
inherited residue rather than anything about the rung.

### Error 5 — running the next pass instead of deciding

Nineteen of thirty passes were launched with no rule saying they should be. The stopping rule was
introduced at `eg2-final.js`, the **fourteenth** pass of that lane, and both rungs shipped
immediately.

> **RULE.** Before launching pass N+1, the orchestrator writes one line: *the class-1 or class-2
> finding this pass exists to close.* If it cannot name one, the rung ships with a disclosure. The
> stopping rule is evaluated at pass 2 and at pass 3, never later — see
> [Procedure § The stopping rule](WORKFLOW-COST-PROCEDURE.md#the-stopping-rule).
