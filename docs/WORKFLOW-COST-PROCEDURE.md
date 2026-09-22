# The workflow cost plan — the procedure

Index: [WORKFLOW-COST-PLAN.md](WORKFLOW-COST-PLAN.md) ·
evidence: [WORKFLOW-COST-EVIDENCE.md](WORKFLOW-COST-EVIDENCE.md) ·
briefs: [WORKFLOW-COST-BRIEFS.md](WORKFLOW-COST-BRIEFS.md) ·
limits: [WORKFLOW-COST-LIMITS.md](WORKFLOW-COST-LIMITS.md)

**Rule of admission for this file:** every prescription names the corpus defect it prevents and the
cost it saves. A prescription that cannot do both is not here.

---

## The pass structure

**Three passes when the rung is clean and lands in one; more when it is not** — pass 3 repeats while
it is still reducing class-1 findings, to a hard cap of three rounds, and pass 1 is one implementer
per ≤1351 insertions, so the pass count grows with **L**. At the two measured rungs' retained sizes
that is **8 passes for A1** (L = 3) and **11 for EG2** (L = 6) at the cap. **5 agent runs at the
small-rung floor, 16 at A1's cap and 19 at EG2's**, against A1's measured **57 agents launched** and
EG2's **61** — the journal's count, defined at
[Evidence § What "agent run" means](WORKFLOW-COST-EVIDENCE.md#what-agent-run-means--six-quantities-one-name).
Those 5, 16 and 19 are *intended* launches, so the comparison is not like-for-like and
[Plan § The cost](WORKFLOW-COST-PLAN.md#the-cost-before-and-after) carries the overhead correction.
*(An earlier draft of this line read "up to six passes … 5 to 16 agent runs", which is the L = 1
ceiling the same file's [pass 1](#pass-1--the-landing) had already refuted, and then compared against
53 and 57, which counted the scripts rather than the agents.)*

### Pass 0 — the gate audit

**One agent. Before any code exists. It reads the plan and the tree; it writes nothing but a
verdict.**

Deliverables, all three mandatory:

1. **A gate table.** One row per gate the plan names: *the production mutation this gate must catch*,
   *the source location that mutation goes in*, and *does the gate as specified observe it — yes /
   no / cannot tell.*
2. **A registration inventory.** Every system, public fn, hook and observer the rung will register or
   modify, and for each the gate that covers it — **or the literal word `NONE`.** A `NONE` is not a
   blocker; an *omission* is.
3. **A verdict:** APPROVE with a corrected gate list, or REFUSE.

**The defect it prevents.** `d26_block.md` is this pass, run once, and it is the highest-yield pass
in the corpus: *"six of its ten gates observed something other than what they claim"* — measured
against the kernel before a line was written — plus an obligation belonging to no rung (four trybuild
fixtures flip at EG2; the rung's ten gates and five REDs mention the census **zero** times). EG2 was
REFUSED (`eg2.md`) rather than written and then unwritten.

**OMISSION — pass 0 DOES catch `ui_clock_tick`, and the draft that said otherwise measured the wrong
file.** That draft claimed the system is registered by the rung's *own new code* and that A1's plan
did not name it. Both halves are refuted in `D:/wt/ui`:
`git show e7a16fd9^:crates/boyko_ui/src/animation.rs | grep -c "add_system(ui_clock_tick)"` is **1**
— the registration predates the rung, which only appends `.key()` to that line — and
`git show e7a16fd9^:docs/UI-PLAN-ANIMATION.md | grep -c ui_clock_tick` is **11**. Requirement 2
covers every system the rung *registers **or modifies***, so a system the plan names eleven times
and the tree already contains is inside it twice over. The draft measured `0` because it grepped
`docs/UI-PLAN-ANIMATION-A1.md`, which the plan-split commit `615cda8f` created and which did not
exist at `e7a16fd9` — this file's own
*"a binder pins which file a name resolves to, never which file the sentence meant"*, reproduced
inside the repair written to record it.

**What pass 0 genuinely cannot catch is `ui_visual_sink_on_add`** — an `on_add` hook with **zero**
occurrences in the pre-landing source and **zero** in the pre-landing plan, registered by the rung's
own new code. That is the one the catch belongs to **pass 1**, where Q1's every-branch plant reaches
it, and it is what [Briefs § Q1](WORKFLOW-COST-BRIEFS.md#q1--the-mutation-question) attributes
there. Pass 0's evidence base is `d26_block.md` and nothing else; see
[Limits § L6](WORKFLOW-COST-LIMITS.md#l6--the-pass-0-audit-can-itself-be-wrong).

**The cost it saves.** EG2's six mis-aimed gates would otherwise have been written, landed, refuted
and repaired. The refusal cost **5 agent runs and 0 h 56 m** (`reflect-eg2-wf`, EG2 pass 1) against
the **13 further passes, 14 runs and 56 agents launched** that lane spent once the rung *was*
written — 61 launched minus pass 1's 5, from the journal at
[Evidence § What "agent run" means](WORKFLOW-COST-EVIDENCE.md#what-agent-run-means--six-quantities-one-name).
Four of those 56 are the 403 run that returned nothing, so 52 came back.
On A1, pass 0 also books the `ui_clock_tick` surface the OMISSION above re-attributes to it —
passes 6 and 8, **8 agent runs and 4 h 25 m**. The rest of A1's saving is pass 1's, below.

### Pass 1 — the landing

**One implementer per ≤1351 insertions of retained landing. Add a doc agent only if the rung carries
an owner decision** (a VALUES or SCOPE call) — not to write a landing note; see
[What to stop writing](#what-to-stop-writing).

**The ceiling is measured, not chosen.** The largest single-pass landing in the corpus is **1351
insertions** — EG2 pass 8 (`eg2r6.js`), `4429 → 5780` read from the pass-9 brief's own PREAMBLE.
A1 pass 8 is larger at **1816** (413 tracked, `2866 → 3279`, plus the 1403-line untracked census
recorded at `a1r6_landCode.md:7`), but that census is exactly the instrument
[Error 3](WORKFLOW-COST-BRIEFS.md#error-3--commissioning-apparatus-whose-defects-then-consume-passes)
refuses at commission time, so it does not set the ceiling for a landing that ships.
**A rung whose retained landing exceeds 1351 insertions gets a second landing pass, and one more per
further 1351.** Both measured rungs do: A1 retains 2425 insertions under the class rule (493
production + 1932 instrument) and EG2 retains 5136 (1620 + 3516) — **2 and 4 landing passes**, or
3 and 6 once the tripwire's prose allowance (≤ production + 200) is added. Sizing pass 1 at one
implementer per rung was a ceiling the corpus contradicts.

The implementer's report must contain, for every gate it lands:

* the red mutation **applied with `Edit`**, the gate run, and the **failure output pasted**;
* the same defect class planted in **every branch of every system in the rung's set**, including
  branches the fixture does not reach — **with the GREENs recorded as well as the REDs**;
* a restore proof per touched file (`cmp`, from a snapshot **outside the repo tree**).

**The defect it prevents.** **Eleven of the corpus's 13 class-1 findings were found by an agent
reading source and writing a throwaway probe. Two were found by landed gates — and by agents who
ran them and diagnosed what the red was masking**; see
[Evidence § what did catch things](WORKFLOW-COST-EVIDENCE.md#what-did-catch-things). This pass makes
the probe the deliverable instead of a by-product, and [Q2](WORKFLOW-COST-BRIEFS.md#q2--the-gate-exit-question)
makes running the landed gates unpiped an obligation rather than a hope. It is also the direct answer
to the corpus's "a gate whose red nobody watched is not a gate": A1's gate 6 was blind to the whole
reap loop and to two of four channels (`a1_attack.md` §4), and `miri_a1_tween`'s headline could not
fail on two independent counts (§3) — both are one planted `Vec::with_capacity(64)` away from
visible.

**This is where the second of A1's two never-closed surfaces is found.** `ui_visual_sink_on_add`
(`a1r7_refute.md` F3) is in no census at all and is named in no plan text, so only Q1's "plant the
defect class in every branch of every system in the rung's set" reaches it — the moment the rung's
own registration exists, which is this pass. The first surface, `ui_clock_tick` (`a1r4_refute.md`
§9, restated at `a1r6_refute.md` §6), is **pass 0's**: the plan names it eleven times and the
registration predates the rung, per the OMISSION under
[pass 0](#pass-0--the-gate-audit). Either way both are disclosed before pass 2.

The restore proof is not bureaucracy: A1 pass 11 inherited a **live `struct ZzIter` probe in tracked
production source** over two reports certifying its absence (`a1r9_refute.md` F1).

**The cost it saves.** A1 pass 2 spent 4 agents and 1 h 35 m re-discovering three gate blind spots
the implementer's own probes would have surfaced at landing. And A1 found `ui_visual_sink_on_add`
uncovered only at pass 9 and never closed it — **4 agent runs, 2 h 31 m**, for a disclosure this
pass writes in one line. The other surface, `ui_clock_tick` at passes 6 and 8 (8 agent runs,
4 h 25 m), is booked at [pass 0](#pass-0--the-gate-audit); between them the two surfaces cost **12
agent runs and 6 h 56 m** and neither was ever closed.

### Pass 2 — the verify pair

**Two agents, in parallel, which do not see each other's output.**

* **(a) Re-run.** Reproduce every number in the landing report from scratch. Do not read the
  landing's derivation before deriving your own.
* **(b) Adversary.** Plant mutations; try to make a gate green over a real defect.

**Decide pass 3 from their overlap, not from a feeling.** Two independent reviewers over one artifact
give a capture-recapture estimate: high overlap means few defects remain; low overlap means many do.

| overlap | action |
|---|---|
| both agents found the same findings, and none is class 1 or class 2 | **STOP.** Ship. |
| either found a class 1 or class 2 | **Pass 3**, scoped to that finding |
| the two found largely *disjoint* sets, all class 3/4/5 | **STOP and disclose.** Disjointness over prose findings is the signature of an unbounded residue, not of hidden bugs — see [Evidence § the two instruments that caught nothing](WORKFLOW-COST-EVIDENCE.md#the-two-instruments-that-caught-nothing) |

**The defect it prevents.** A1 pass 2 (`a1_attack.md`) returned 3 class-1 and 3 class-2 — the largest
single yield in that lane. EG2 pass 2 (`eg2_refute.md` + `eg2_rerun.md`) returned 4 class-1 and 3
class-2. Two agents at this position is where the corpus's money was made.

### Pass 3 — repair and verify, while it is still working

**Fires only if pass 2 returned class 1 or class 2. Its subject is the repair diff alone, not the
rung.** One implementer + the same two verifiers, **3 agent runs per round**.

> **The termination rule.** Pass 3 **repeats while each round strictly reduces the class-1 count**,
> to a **hard cap of three rounds**. A round that does not strictly reduce it does not get another.
> **The loop therefore terminates two ways — at the cap, or at a round that did not strictly
> reduce — and [the stopping rule](#the-stopping-rule) disposes of what is open in either case,
> identically.** An earlier draft wrote the withdrawal clause for the cap alone, which left the
> other exit with no defined transition: a plain class-1 bug outstanding after a non-reducing round
> was neither repairable (the rule forbids another round), nor escalatable (it is not a VALUES or
> SCOPE question), nor withdrawable (the cap was not reached). That is this campaign's own defect
> class — a rule whose predicate has a state it does not name — one level down, and it is fixed by
> the words *whether by cap or by a round that did not strictly reduce*, below.
>
> **When the class-1 count is disputed,** the round counts as *not strictly reducing*. The
> classification is a judgement this set itself calls *"a judgement … the counts are ±"*
> ([Evidence](WORKFLOW-COST-EVIDENCE.md#how-these-numbers-were-taken)), so a rule that keys on it
> must say which way an unresolved disagreement falls. It falls toward stopping, because the
> alternative is a loop that continues on an unresolvable premise.

**Why it is not "at most once", which is what an earlier draft of this file said.** "At most once"
leaves **no terminating state for the bugs in the repair itself** — and repairs having bugs is the
corpus's single best-attested fact. A1 pass 3's repair shipped a class-1 production regression, and
**that pass's own verify pair caught it inside the same pass** (`a1r_refute.md` §1 — the report was
filed against pass 4 until this round; see the erratum in
[Evidence](WORKFLOW-COST-EVIDENCE.md#a1s-class-1-and-class-2-findings-individually)). If pass 3 may
not fire again, that regression has nowhere to go but into the ship. The rule above gives the
repair's own defects a bounded home instead of an unbounded one, and the corpus shows a repair round
finding its own regression, which is the property being relied on.

**Why three, and why "strictly reduces".** The corpus's two lanes each needed **two** repair rounds
and no more: A1's class-1 findings land at passes 2, 3 and 4 and never again in 9 further passes;
EG2's at passes 2 and 3 and never again in 11. Three is one round past the measured need, which is the
whole margin the evidence supports. The *strictly reduces* clause is what stops the loop the
[repair discipline](#the-repair-discipline) describes — a round whose repairs introduce as many
class-1 findings as they close has demonstrated that repairing is not converging here, and one more
round is a prediction the corpus does not support.

The repair is governed by [The repair discipline](#the-repair-discipline) below.

**The defect it prevents.** A repair round must itself be verified: A1 pass 3's repair introduced a
**live production regression** — `start_tween_*(duration_ms = 0.0)` became a silent no-op in release,
with three doc blocks shipped saying the opposite (`a1r_refute.md` §1, pass 3's own adversary). EG2 pass 3 found three class-1
defects in the pass-2 landing (`eg2r_refute.md` §§1-3, including 12,440 bytes of stack per call in
release under a comment asserting the opposite).

**Why the cap is three and not more.** After each lane's fourth pass, no pass in 22 further tries
found a wrong production behaviour — A1 passes 5-13 (9) plus EG2 passes 5-14 (10) is 19, and the
three plan-split passes make 22. Two repair rounds exhausted both lanes; the third is margin.

---

## The stopping rule

### Why the corpus's own rule cannot be the rule

The rule finally written at `eg2-final.js` reads: *"(a) a wrong FACT — a count, a path, a claim about
the code. That blocks. (b) a wrong STRENGTH or an imprecise disclosure — if the remaining ones are of
this kind and are recorded honestly, say the rung ships."*

**Applied from pass 3, it defers almost nothing.** Every pass from 3 onward contains at least one
kind-(a) wrong fact, and most are the *repair's own* sentences: a `>= 5.24288e8` boundary transported
to four sites and both twins (`a1r3_refute.md` §3); a one-ULP-wrong closed form at three sites (§4);
`3.673e-36` occurring at no frame of the gate that pins it (`a1r3_rerun.md` §7, still live at
`a1r4_refute.md` §3); a false `#[cold] #[inline(never)]` claim in landed kernel source over a
12,440-byte unconditional frame (`eg2r_rerun.md` §2); `ArchetypeState` — **a type the tree does not
contain** — as the load-bearing symbol of a round's headline finding (`eg2fin_verify.md` §1).

**A rule keyed on "a wrong fact blocks" cannot stop a loop whose output is wrong facts.** What
actually stopped both rungs was a *writing* rule that converted assertions into disclosures, which
then gave the stopping rule something of kind (b) to pass. Both rungs shipped within one pass of that
conversion.

### The rule, stated so it applies at pass 2

> **SHIP when the last landing changed no non-comment line under `src/`, and the verify pair reported
> no wrong claim about production behaviour.**
>
> **ESCALATE, do not repair,** a **VALUES or SCOPE** question that survives pass 3: it goes to the
> owner as a decision. A class-1 defect that is *not* a VALUES or SCOPE question — a plain bug with a
> known fix — is **not** escalated: it is the subject of the next repair round, and if it is still
> open **when the repair loop terminates — whether by the cap of three rounds or by a round that did
> not strictly reduce the class-1 count** — the rung is **WITHDRAWN**, not shipped and not handed to
> the owner.
>
> Everything else open at STOP is written **once, as a disclosure**, and the rung ships.

**It is mechanical. Three commands, in this order:**

```
git status --untracked-files=all                       # FIRST — see the note below
git diff <landing>^ <landing> -- '*/src/*'             # CONTENT, not --numstat
git diff <landing>^ <landing> -- '*/src/*' | grep "^+" | grep -v "^+++" | sed "s/^+//" \
  | grep -cvE "^[[:space:]]*(//|$)"                    # the predicate. 0 = SHIP
```

Zero non-comment code lines means the passes are no longer about the engine. **The third command is
the one that answers the question**; an earlier draft stopped at the second, which prints the diff
and leaves a human to classify it — a predicate no command evaluates is not a mechanical rule. The
classifier is not new here: it is the same pipeline provenance rows P08, P09 and P10 already run to
produce this set's own `40`, `18` and `22`.

**Why `git status --untracked-files=all` runs first, and why the second command is not
`--numstat`.** Both failure modes are real; the demonstrations an earlier draft offered for them
were not, and the replacements below were run for this document:

* **An untracked file is invisible to `git diff`.** A1's 4157-line source census was untracked from
  pass 8 to pass 13, so `git diff` did not count it and six briefs' preambles under-reported the diff
  by exactly 4157 (`5386` in the last brief against the commit's `9543`). *`--untracked-files=all`
  rather than the default matters only when the new file is in a directory git is not already
  tracking, because the default collapses such a directory to one line.* **Neither case in this
  campaign is that case, and the earlier draft's re-measurement cannot show the difference:** plain
  `git status --porcelain` in `D:/wt/reflect` prints all six `WORKFLOW-COST-*` files individually
  (they sit in tracked `docs/` and `tests/`), and A1's census lived in `crates/boyko_ui/tests/`,
  which `git ls-tree e7a16fd9` shows was already tracked — so the default form would have named it
  too. The flag is the right default because it cannot be wrong; *"this campaign measured that the
  default was not enough"* is what was wrong.
* **`--numstat` counts lines, so it cannot see what the predicate asks.** The failure mode is
  **comment versus code**, and it is this set's own headline: A1's remediation is 493 gross added
  lines under `src/`, **40** non-comment, **five** of release behaviour. Demonstrated on a throwaway
  repo for this document: a landing that adds two `//` comment lines gives
  `git diff --numstat HEAD^ HEAD -- 'src/*'` → `2  0  src/lib.rs`, which is nonzero and reads
  *keep going*, while the classifier above gives **0**, which reads *SHIP* — and SHIP is right,
  because nothing about the engine changed. *(An earlier draft justified this bullet with a
  same-line token swap, claiming the numstat is identical. Re-run: as two alternative landings both
  read `1  0`, but as a swap **commit** it reads `1  1`, and either way the predicate answers
  "nonzero → keep going", which is correct. It also cited EG2 pass 4's `target_archetype_id` —
  **170 targets green** over a one-token mutation, `eg2r2_refute.md` §2 — which is a **test-suite**
  blindness, evidence for the [class rule](#the-ratios-and-the-rule-that-holds-them), not for a
  numstat one.)*

**EG2's last three briefs all carry the identical `8151 insertions(+), 223 deletions(-)`** — six
agent runs and 2 h 36 m over a net-zero diff, measured by reading the preambles of `eg2ship_land.md`,
`eg2cnt_land.md` and `eg2fin_land.md`. `a1r4_rerun.md` §Divergence B measured at pass 6 that
`animation.rs`'s entire non-comment delta was **inherited from pass 3**; six more passes followed.

**The escalation clause is not invented — it is what ended A1's class-1 chain, and its precedent is
a VALUES call, which is why the clause is restricted to one.** `a1r3_ruling.md` FORK B ruled the
immortal long-duration row a VALUES call: *"any compile-time constant is a conservative policy
choice, and 'what is the longest legitimate UI tween' is a product question. That is a VALUES call →
owner."* It was filed in `docs/OPEN-QUESTIONS.md`, deliberately **not** repaired, and the rung
shipped with it disclosed. That is the corpus's **only** instance of escalation, so the clause claims
exactly what it can support: escalating an ordinary bug to the owner would be an extrapolation from
zero cases, and it is also what
[L7](WORKFLOW-COST-LIMITS.md#l7--the-escalation-clause-moves-cost-to-the-owner) warns spends the
scarcer resource.

### What is closed by evidence, and what is deferred to a disclosure

| closes the rung | evidence required |
|---|---|
| every gate the plan names | its red mutation applied, run, and the failure output pasted (pass 1) |
| every registered system / fn / hook | a covering gate, **or** a `NONE` row written at the instrument and in the disclosure (pass 0) |
| the mandated repo gates | run unpiped, `$?` on the next line, output pasted (pass 1 and pass 2a) |
| every figure in the record | emitted by a gate at run time, or deleted ([R4](#r4--never-write-a-figure-about-the-edit-set-you-are-inside)) |

**Deferred to a disclosure, never to another pass:** an uncovered surface with a `NONE` row; an
instrument's residue; a gate that is narrower than its name; a VALUES or SCOPE question for the
owner. The corpus is unanimous that this is safe — **every such item shipped anyway**, after the
passes that found it:

| deferred item | class | shipped? |
|---|---|---|
| `ui_clock_tick` / `ui_visual_sink_on_add` uncovered (`a1r4_refute.md` §9 → `a1r6_refute.md` §6 → `a1r7_refute.md` F3) | 2 | **Yes** |
| four executable sites never inside the armed window, incl. "the ordinary frame in any real UI" (`a1r5_refute.md` §3) | 3/2 | Yes |
| `migration_helpers.rs:623` ungated (`eg2r7_refute.md` §7); the filtered-clone path (`eg2r9_refute.md` §1) | 2 | Yes |
| both serialize load walks ungated (`eg2cnt_land.md`) | 2 | Yes |
| the `g18`/`g18b` `newly_added` gap; `g19`'s single-membership scope (`eg2fin_verify.md`) | 2 | Yes |

**None is class 1. The rule changes when the disclosure is written, not what ships.**

---

## The repair discipline

Class-3 findings — defects a repair of this campaign introduced — are the owner's "endless
rewrites". Four rules, each with the corpus defect it prevents.

### R1 — a repair gets the landing's observer-before-edit treatment

Before a repair lands, its own gate must be **red-proven**: apply the defect the repair claims to
close, run the gate, paste the failure. A repair that closes a *documentation* defect has no gate and
therefore may not claim a behaviour changed.

**Prevents.** A1 pass 3's repair shipped a class-1 production regression with three doc blocks
asserting the opposite (`a1r_refute.md` §1). It also prevents the inverse: `a1r3_refute.md` §1 —
*"D1's repair re-created the exact defect it was convened to fix"*; the widening made the `None` arms
execute and the new floor made them unobserved, so **32 allocations were invisible 40/40** while 1
allocation reds 30/30.

**Saves.** A1 passes 4 and 5 — 9 agent runs, 3 h 30 m — were spent on the pass-3 repair.

### R2 — a repair is re-derived from the criterion, never substituted from a list

If a claim is wrong at site A, you may not repair it by pasting the corrected wording at sites B, C
and D. **Re-apply the criterion at every site and report the derivation**, including where it
disagrees with the previous pass.

**Prevents — and this is the strongest evidence in the corpus.** The claim "this defect cannot be
written here" was patched three times running from a substitution list and was wrong each time. Pass
13 (`eg2-count.js`, 2 agents, 1 h 12 m) re-derived all eight sites from the criterion instead:

> *"My derivation is FIVE / THREE, not three-and-five, and it disagrees with the last pass on sites 7
> and 8. All five were compiled and run."* — `eg2cnt_land.md` Act 1

The re-derivation found **two genuinely ungated sites** (`load_writer.rs:716` and `:855`, both
serialize load walks) that three passes of patching had missed, and it corrected the record's own
"six of its eight" to "three of its eight".

The same defect in miniature: *"C3's replacement rationale is pasted across four sites on the word
'identically-shaped', and that word is false in the direction that changes the answer"*
(`a1r2_refute.md` §2).

**Saves.** One re-derivation pass replaces an unbounded number of patch passes, and produces a better
answer than any of them.

### R3 — a claim that cannot be re-derived mechanically is deleted, not repaired

If the repair is "reword the sentence", the sentence goes. A claim survives only if a gate can emit
it, a compiler can check it, or it is a decision (which cannot be wrong, only superseded).

**Prevents.** The 4157-line source census exists because a hand-written residue list kept being
refuted: 13 items → 15 items across passes, and both rungs' final classifications read *"(c) — a
positive assertion that is still false"* (`a1r10_refute.md`, `a1r9_refute.md`). `a1r7_refute.md` F2
records the terminal form: *"The 'EXECUTABLE witness' is satisfied by mentioning the name. One line
converts the re-run's decisive red into a green, with the witness running zero times."*

**Saves.** A1 passes 8-13 — **24 agents launched, 22 returned, 15 h 36 m** — are the history of not deleting one
residue list.

### R4 — never write a figure about the edit set you are inside

A count of things in the current diff is a **pre-state** by the time the edit finishes. Emit it from
the gate at run time (`--nocapture`) and cite the gate; never type it into prose.

**Prevents.** Five recurrences in the corpus (`eg2r3_refute.md` §7 — *"false BECAUSE writing it added
the citations it counts"*; `eg2r5_refute.md` §6; `eg2r8_refute.md` §2; `a1r10_rerun.md` F3;
`a1r5_refute.md` §4). The cleanest instance is the cap census's own headline, which **moved because
the passes that wrote it added ledgers**: `66 cap row(s); 54 … 12` at pass 10 → `84 … 70 … 14` at
pass 11. Both readings were correctly measured; both were written into prose that the next pass
falsified.

**Saves.** Passes 12-14 of EG2 — 6 agent runs, 2 h 36 m over a net-zero diff — reconciling figures
that a `--nocapture` line would have kept true for free.

### R5 — the repair budget

**A repair that would add more instrument than the production lines it protects is refused; write a
disclosure instead.** State the residue and stop.

**R5 is also the refusal mechanism for the [class rule](#the-ratios-and-the-rule-that-holds-them).**
An instrument that a behaviour-preserving refactor of production source would turn red is refused
here — at commission time by the orchestrator
([Error 3](WORKFLOW-COST-BRIEFS.md#error-3--commissioning-apparatus-whose-defects-then-consume-passes)),
and at repair time by the implementer — and the answer is a disclosure. There is one refusal path,
not two.

**Prevents.** The two censuses: 8375 lines of instrument protecting **40 and 752** executable
production lines respectively — **792 combined** — zero production defects caught, 16 of 30 workflows
consumed.

---

## What to stop writing

### The categories that should not have existed

| category | measured size | why it goes |
|---|---|---|
| **Enumerative census over source shape** (`ui_a1_source_census.rs`) | 4157 lines, 13 tests, 6 passes | It pins the *shape of the implementation*, so every edit invalidates it and every re-derived figure is a fresh chance to write a false sentence. Its residue is **open by construction** — passes 8-12 each found one more construct it cannot see (a macro-introduced branch, a `&&` short-circuit, an `impl Drop`, an operator-trait impl, a fifth). **Zero production defects in 6 passes.** |
| **Anchor / coordinate census over prose** (`internal_docs_anchors.rs`, +4218) | 1991 → 6167 lines, 8 → 23 tests, 7 passes | Every documented catch is *"anchors pointing into the file I had just edited… created by me"* (`eg2r3_landCode.md`). **83 % of its cap rows are caps over an empty population** (`84 cap row(s) … 70 sit at 0 over an empty live set`). |
| **The landing note** — a plan section per adversarial pass | **1121 of `docs/UI-PLAN-ANIMATION-A1.md`'s 1426 lines** in `D:/wt/ui` — the note opens at `:306` (`**LANDING NOTE — A1 landed 2026-08-27..28 in NINE parts…**`) and runs to EOF, so `1426 − 306 + 1 = 1121`; sections are literally titled *"Part 11 — the ELEVENTH pass"* (`:1383`) | It is a record of defects already fixed. Nobody reads it; the decision it encodes is two pages. |
| **Owner-facing blocks about the instrument** | **1135 of 1881** `OPEN-QUESTIONS.md` lines (60 %), the largest single block being 580 lines + a 580-line Russian twin, entirely about the anchor gate's own claims | The owner channel is for VALUES and SCOPE calls. The instrument's residue belongs at the instrument, in one paragraph. |
| **The Russian twin of instrument prose** | 1955 of 5944 prose lines (32.9 %), re-verified in ≥14 agent runs | `docs/ru/` is owner-granted for **documents the owner reads**. Parity tests sameness, never truth. Mirror the decisions and the owner questions; do not mirror residue lists. |

**OMISSION — the landing-note row's file name resolves differently before and after the plan-split,
and the numstat points at the wrong one.** `git show --numstat 2a10f3a4` in `D:/wt/ui` attributes
**1292 insertions to `docs/UI-PLAN-ANIMATION.md`**, so a reader re-deriving the row from the commit
lands there. The landing note is **not** in that file today: the plan-split moved it, and
`wc -l docs/UI-PLAN-ANIMATION.md docs/UI-PLAN-ANIMATION-A1.md` now prints **393** and **1426**, with
`grep -n ELEVENTH docs/UI-PLAN-ANIMATION*.md` matching only
`docs/UI-PLAN-ANIMATION-A1.md:1383`. The row above names the
file the sentence is in, not the file the commit changed — which is this campaign's own *"a binder
pins which file a name resolves to, never which file the sentence meant"*, arriving in the document
that records it.

### The ratios, and the rule that holds them

**The raw ratio is not the target.** SQLite runs at ~590 test lines per library line and is not in
trouble. What distinguishes the corpus's good instrument from its bad is **durability**, not size:
`retained_id_walk_pool_skip.rs` is 621 lines over 75 production lines (8.3:1) and closed a
release-present crash in **one workflow, never revisited**; `ui_a1_source_census.rs` is 4157 lines
and was rebuilt five times.

So the target is stated as a **class rule with a ratio tripwire**:

> **Class rule.** **A behaviour-preserving refactor of production source must leave the instrument
> green.** Rename a local, rename a system, reorder two independent statements, extract a helper:
> the behaviour is unchanged, so a gate stays green and only an instrument pinned to *shape* goes
> red. A census over source shape, a cap over a population, a count of citations — each fails this,
> and none of them ships.
>
> **The refusal mechanism is [R5](#r5--the-repair-budget)**, applied at commission time as well as at
> repair time: an instrument that fails the class rule is refused, and a **disclosure** is written
> instead.
>
> **Tripwire.** Prose per rung ≤ **1×** the production lines it documents, and the landing note
> ≤ **200 lines**. Crossing either is not forbidden — it is the trigger to ask which category above
> the excess belongs to.

**Why the rule is stated in this direction, and not as "a mutation to production source can turn it
red", which is what an earlier draft of this file said.** That form is satisfied by the very
instrument it was written to exclude. `ui_a1_source_census.rs` pins function names as string
literals — `const WALKED: &[&str]` at `:399` lists `"ui_visual_tick"` and `"ui_tween_reap"`, and
`const SITES` at `:501` carries 22 rows of `func: "ui_visual_tick"`
(`git show 2a10f3a4:crates/boyko_ui/tests/ui_a1_source_census.rs | grep -n 'const WALKED\|func: "ui_visual_tick"'`
in `D:/wt/ui`). Rename `ui_visual_tick` and the census goes **red** while every behavioural gate
stays green — so under the old form it qualified as a gate. Under the form above it does not, because
a rename is behaviour-preserving. `retained_id_walk_pool_skip.rs` passes both forms.

**What the class rule alone would have removed:** A1 6089 → 1932 instrument lines (−68 %); EG2
7734 → 3516 (−55 %); **combined 13,823 → 5,448, −61 %** — and by the corpus's own record, **zero**
production defects lost, because the removed 8375 lines caught none.

**What the tripwire would have removed:** A1 prose 2961 → ~450; EG2 2983 → ~500. Retained in full:
the 709 lines of genuine owner escalation (the unregistered `ui_render_discovery` that blocks A4; the
+3.8 % per-animating-row cost; `UiTweenScratch.done` as a `std::Vec` in a `Resource`;
`#[require(C)]` semantics for dense `C`; CI's Miri sweep silencing eleven tests). **Those are 38 % of
the owner channel and none of them came from the record loop.**
