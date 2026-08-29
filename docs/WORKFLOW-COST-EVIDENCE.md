# The workflow cost plan — evidence

Index: [WORKFLOW-COST-PLAN.md](WORKFLOW-COST-PLAN.md) ·
procedure: [WORKFLOW-COST-PROCEDURE.md](WORKFLOW-COST-PROCEDURE.md) ·
briefs: [WORKFLOW-COST-BRIEFS.md](WORKFLOW-COST-BRIEFS.md) ·
limits: [WORKFLOW-COST-LIMITS.md](WORKFLOW-COST-LIMITS.md)

---

## How these numbers were taken

| number | how |
|---|---|
| insertions per file | `git show --numstat 2a10f3a4` in `D:/wt/ui`; `git diff --numstat f7c46c76 d272e1fd` in `D:/wt/reflect` |
| comment vs code | a classifier over the added lines only: blank; `//`-leading; `/* … */` including block state; everything else is code. Run for this document over each file named below |
| agent runs per pass | **two different quantities, and the pass tables print the weaker one.** `grep -c "agent("` over each workflow script plus the arity of each `.map()` fan-out gives *intended* launches; the harness journal gives *launched*, *returned*, *failed* and *superseded*. Definitions and the per-lane counts: [What "agent run" means](#what-agent-run-means--six-quantities-one-name) |
| wall clock per pass | script mtime → last-report mtime. **±5 min**, and it includes queueing. The two lanes ran concurrently, so lane totals sum to more than calendar time |
| cumulative diff growth | quoted verbatim from each brief's own PREAMBLE — the state that pass *inherited* |
| findings | read from the `*_refute.md` / `*_verify.md` / `*_rerun.md` reports and cited by basename and section |

**OMISSION — the instrument that counted agent runs could not see 13 of the 119 it should have
counted, and every agent figure in this set moved when it was replaced.** *(And its replacement,
below, is still a count of the script text. The count of agents is
[further down](#what-agent-run-means--six-quantities-one-name), from the harness journal, and it is
128 — not 119.)*

`grep -c "await agent("` was this set's definition of an agent run. It is blind to the form these
scripts use for a parallel fan-out, where the call is wrapped in a thunk and awaited once by
`parallel(...)`:

```js
const audits = (await parallel(LENSES.map(l => () =>
  agent(CONTEXT + l.prompt, { label: 'audit:' + l.key, ... })
))).filter(Boolean)                              // two runs, zero "await agent("
```

Five of the thirty scripts launch agents that way. `ui-a1r-remediation-wf_d7f3bbbf-104.js` has
`() => agent(` at its `verify:rerun` and `verify:refute` sites; `reflect-eg2-round2-wf` has five
such sites; `eg2r.js` two; and the two audit fan-outs (`ui-a1-wf`, `reflect-eg2-wf`) launch **one
call site once per element of a two-element list**, so even counting call sites under-reports them
by one each.

**The proof those agents ran is this document's own citations.** `a1r_refute.md` and `a1r_rerun.md`
are on disk and are exactly the two thunk-launched verifiers of `ui-a1r-remediation-wf`; and
`d26_block.md` — the single artifact the whole Pass 0 case rests on — opens *"D26 — measured at the
EG2 AUDIT"*, i.e. it was written by one of `reflect-eg2-wf`'s two map-launched audit agents. The set
cited reports whose authors its own count excluded.

**The replacement instrument, run for this document**, in the workflow-script directory:

```sh
# call sites, plus the extra run each two-element .map() fan-out contributes
echo $(( $(cat <the lane's scripts> | grep -c "agent(") \
       + $(grep -c "key: '" ui-a1-wf_0c113a2f-027.js) - 1 ))          # A1  -> 53
echo $(( $(cat <the lane's scripts> | grep -c "agent(") \
       + $(grep -c "^  \['" reflect-eg2-wf_f70c0232-045.js) - 1 ))    # EG2 -> 57
```

It is pinned by provenance rows P05, P06, P07 and P26, which previously re-ran the blind command and
were green over it.

| | the `await`-blind count | call sites + `.map()` arity |
|---|---|---|
| A1 | 49 | **53** |
| EG2 | 48 | **57** |
| corpus | 106 | **119** |

*(Both columns are counts of the workflow source. The right-hand column was labelled "measured" for
one round; it measures what the scripts intend to launch.)*

The correction raised the agent-run saving rather than lowering it. It is recorded here because the
direction is not the point: **the four-link provenance chain binds prose → command → tree and has no
link asking whether the command measures the thing the prose names**, which is this set's own *gate
that cannot fail*, arriving inside the gate written against it.

**And 53, 57 and 119 are still the script text.** Every figure in that table is derived by reading
the workflow sources; none of them is a count of agents. The section below replaces them.

---

## What "agent run" means — six quantities, one name

**This is the definition the whole set uses.** Every other document cites this section rather than
restating it: [Plan § The cost](WORKFLOW-COST-PLAN.md#the-cost-before-and-after),
[Procedure § The pass structure](WORKFLOW-COST-PROCEDURE.md#the-pass-structure),
[Briefs § Error 3](WORKFLOW-COST-BRIEFS.md#error-3--commissioning-apparatus-whose-defects-then-consume-passes),
[Limits § estimate](WORKFLOW-COST-LIMITS.md#what-in-this-set-is-an-estimate-rather-than-a-measurement).

Every count this set published before this round — 49, 53, 57, 106, 119 — was read out of the
workflow **scripts**. A script states how many agents a pass *intends* to launch. It cannot state how
many were launched, how many came back, how many the API refused, or how many ran and had their
output thrown away because the harness re-launched the same prompt. The harness records all of that:
one JSON line per event in `<session>/subagents/workflows/wf_<id>/journal.jsonl`.

| term | where the harness says it | what it prices |
|---|---|---|
| **intended** | *neither record* — derived from the script source | what a pass asks for. Every figure this set published before this round is this quantity, and it is the correct denominator for nothing but a description of the script |
| **launched** | journal, `"type":"started"` | one agent **process** — one context window, one bill. **The denominator for cost incurred.** A re-launch is a second process and is counted again, because it was billed again |
| **returned** | journal, `"type":"result"` | the agent delivered a report the workflow consumed. **The denominator for work delivered** — every finding this document cites was written by a returned agent |
| **failed** | journal, `"type":"failed"` | the API refused the agent. No report |
| **superseded** | journal: `started` with no terminal line, whose key a later `started` resolves | the agent ran, spent its context, and the harness discarded its output and re-launched the same prompt |
| **slots** | run record, `"agentCount"`; equivalently the journal's **distinct `key` per run** | the number of *places* the workflow had for an agent, counting a prompt re-launched after a discard **once**. Not a cost denominator — it is the quantity the second record carries, and therefore the one the two records can be checked against each other on |

`launched = returned + failed + superseded`, and the sum is checked by the provenance rows below.
**`slots ≤ launched`**, and the gap is exactly the re-launches: 123 slots against 128 processes over
the 31 lane runs. **The gap is 5 and the superseded column reads 4**, which is not an inconsistency:
four re-launches left no terminal line at all, and the fifth — `a1r9.js`'s one key with three
`started` lines — ended its second attempt in a `failed` and is therefore already counted in the
failed column. A re-launch is visible in two different columns depending on how its predecessor died,
which is precisely why the slot count has to be taken from `key`s and not by subtracting columns.

⚠️ **These are counted over *records*, not over distinct `key`s, and that is deliberate.** A retry
shares its predecessor's `key`, so counting distinct keys would price two billed processes as one —
which is right for *slots* and wrong for *cost*. The reverse error is just as easy: `key` is a hash of
the agent's input, so two agents given an identical prompt collide onto one key and a distinct-key
count silently merges them. **Neither count is "the" agent count; each is right for one question**,
and every figure below says which it is using.

**Which use needs which, stated at each use rather than assumed to be one number:**

* **The cost table** and the **cost factor** price tokens spent, so both take **launched**.
* **The yield curve's** findings are all authored by returned agents, so the *findings* side is
  **returned**.
* **The zero-yield headline** is work delivered against cost incurred, so it needs **both ends**, and
  it is stated below on both. They do not differ much, and that is the point: correcting the
  denominator does not rescue the corpus.

### Per lane, counted from the journal

Each lane's scripts map to journal directories through the run metadata's `scriptPath`
(`<session>/workflows/wf_<id>.json`); the 31 directories are named in provenance rows P31-P37.

| lane | passes | **runs** | intended (script) | **slots** (both records) | **launched** | **returned** | failed | superseded |
|---|---|---|---|---|---|---|---|---|
| A1 | 13 | 13 | 53 | **53** | **57** | **51** | 3 | 3 |
| EG2 | 14 | **15** | 57 | **61** | **61** | **57** | 4 | 0 |
| plan-split | 3 | 3 | 9 | **9** | **10** | **9** | 0 | 1 |
| **total** | **30** | **31** | **119** | **123** | **128** | **117** | **7** | **4** |

The **slots** column is the only one in this set carried by two records that were written independently
of each other; [the cross-record check](#the-cross-record-check--the-sets-first-figure-with-two-carriers)
below is what establishes that, and rows P38-P41 pin it.

**Passes and runs are not the same number.** EG2 pass 11 (`eg2r9.js`) ran **twice** — once as
`wf_4d554e0b-df4`, whose four agents all failed, and again as `wf_60fc1374-906`, whose four returned.
A count over scripts sees one pass, because only the surviving script is on disk. That is the whole
of the 14-vs-15 gap, and it is why the failed run below was never in any total.

**A1's intended 53 is exactly right, and that is a coincidence worth naming.** A1's slots are 53 and
the plan-split's are 9 — the script-derived instrument recovers the intended count precisely. For EG2
it does not: the slots are **61** against an intended 57, because the retried script was edited
between its two runs and the text instrument can only read the surviving version. So the replacement
instrument of the previous round measures *intent* correctly and measures *expenditure* not at all.

*(EG2's key count is **61 per run** and **60 across the lane**, and the difference is not an error in
either: one of `eg2r9.js`'s four prompts was byte-identical in both of its runs, so that prompt hashes
to one `key` in two different journals. Two agents were launched and two were billed, which is why the
per-run count is the one the slots column and rows P38-P41 use. An earlier draft of this paragraph
printed the 60 without saying which count it was — the same ambiguity, one layer down, that this whole
section exists to remove.)*

### The cross-record check — the set's first figure with two carriers

**The harness writes two records, and until this round every provenance row read at most one of them.**

* `<session>/workflows/wf_<id>.json` — **one per run**, written when the run ends, carrying
  `agentCount`, `totalTokens`, `totalToolCalls`, `durationMs`, `status`.
* `<session>/subagents/workflows/wf_<id>/journal.jsonl` — **one line per event**, appended as each
  agent starts and finishes.

Neither is derived from the workflow source, and neither is derived from the other. Rows P38-P41
compute the slot count **from both** and compare them in a single command: `agentCount` summed over a
lane's run records, against the journal's distinct `key` count summed per run. They agree exactly —
**A1 `53 = 53`, EG2 `61 = 61`, all 31 lane runs `123 = 123`.**

**Why this row is worth more than the figure it pins.** Every other row in this table re-runs a
command over the artifact the sentence was derived from. That is one carrier wearing two names: a
misreading is made identically on both sides, so the row is *incapable* of disagreeing, and it is
precisely how this set's two worst figures — the `await`-blind agent count and the yield curve's
denominator — stayed green through two rounds while being wrong. A row whose two sides were written
by different parts of the machine, at different times, for different purposes, can disagree. That
property is not visible in a row's output, so it cannot be gated; it can only be chosen when the row
is written, and stated when it is.

**The check is not vacuous, and row P41 is the proof.** The same command over two finished runs from
other campaigns in this session returns
`wf_9168b84b-530 17!=23 wf_d748568f-2c2 7!=9` — the two records disagree there, by 6 and by 2. The
cause is the key-collision hazard named above: `day-ship-review` launched several verification agents
on byte-identical prompts, so the journal merges them onto one key while `agentCount` does not. **A
check that returns the same answer on every population it is pointed at is measuring nothing**; this
one returns a mismatch on the first population outside the lanes, which is the evidence that the
agreement on the lanes is a fact about the lanes and not a property of the arithmetic.

**Three limits, stated here rather than left to be found.**

1. **This check cannot run on another machine, and it fails rather than skips.** Both records live
   outside the repository under this session's path. On any other checkout rows P38-P41 red, on the
   same terms as `EXECUTED_ROWS` and for the same reason: a check that quietly skips a population is
   this campaign's signature defect. They are also the most perishable rows here — the session's
   working files are cleaned up eventually, and these go red on this machine when they are.
2. **Both records see workflow-launched agents only.** An agent the orchestrator started directly
   through its own Agent tool belongs to no workflow, so it is in no journal *and* in no run record.
   The agreement is between two views of the same population, not a census of the campaign's agents,
   and nothing available here can measure the difference.
3. **The comparison counts distinct `key` per run, not journal records.** Counting records conflates
   a retry with an agent and would put the two sides on different units — 128 against 123 — producing
   a mismatch that means nothing. This is not a hypothetical: it is the error the first attempt at
   this row made, and it is the reason the `slots` row exists in the table above.

### What the lanes cost in tokens, which no figure in this set had before

**An agent count is a proxy for spend. The run records carry the spend itself**, and it does not
depend on which of the six quantities above you pick:

| lane | runs | tokens | tool calls |
|---|---|---|---|
| A1 | 13 | **11,348,796** | 4,903 |
| EG2 | 15 | **14,147,330** | 6,005 |
| plan-split | 3 | 1,559,864 | 578 |
| **total** | **31** | **27,055,990** | **11,486** |

**27.1 M tokens to land two rungs.** Row P42 pins the total. This is the number every ratio in
[Plan](WORKFLOW-COST-PLAN.md#the-cost-before-and-after) is a proxy for, and it is worth more than the
proxy: it is measured directly, it needs no definition of "agent run", and it does not move when the
definition is argued about. The proposal's side of that ratio still has no token figure, because the
procedure has never been run — so the factor stays stated in agents, and this table is what a future
round should replace it with once there is an after to measure.

*(Wall clock is deliberately absent from this table. The run records carry `durationMs`, and it
disagrees with the mtime-derived clocks this set publishes — `a1r9.js` is 5 h 40 m by mtimes and
0 h 20 m by the harness's own field, because the two measure different things. Mixing them would
produce exactly the kind of figure this set exists to warn about, so the clocks are left where they
are, on the basis [Limits](WORKFLOW-COST-LIMITS.md#what-in-this-set-is-an-estimate-rather-than-a-measurement)
already states.)*

### What could not be attributed, and is therefore not folded in

**31** of the session's journal directories are the 30 passes above, and those 31 are finished and
frozen — they are the only ones any figure here or any provenance row touches. The session held
**59** directories when this was written and holds more now; the other **28** belonged to other
campaigns in the same session — the UI sprites ladder (`ui-a0`, `ui-s4`, `ui-s4-round2`,
`ui-s4-round3`, `ui-s5`, `ui-s6`, `ui-advanced-campaign`), the reflection campaign (`reflect-b0`,
`reflect-eg0`, `reflect-eg1`, `reflect-c7`, `reflect-c8`, `reflect-c9`), the aether reference, two
ship reviews, the ignore-reason census, this document set's own repair workflows — and **3 had no run
metadata at all**, being in flight or aborted before it was written, so the split is
`31 + 25 + 3 = 59`. **None of them is evidence for either lane and none is counted here.** A run that cannot be assigned to a lane is not a small
rounding error to fold in: the failure this whole set is about is a confident total taken over a
population nobody looked at.

One of the 28 is adjacent rather than unrelated: `kernel-retained-id-filter` (`wf_42a88859-066`,
3 launched / 3 returned) is the spun-off fix the EG2 pass table already lists with a `—` in its pass
column. It stays outside the 61, exactly as that table has it.

**Corpus-wide totals are deliberately not pinned.** They **rise while the session is running**: the
session held 59 journal directories when this section was first written, 60 an hour later, and it
reads 323 launched / 288 returned / 7 failed at the latest count. A provenance row over them would red
on the next workflow. Only the 31 finished lane runs are pinned, and any corpus-wide figure quoted
anywhere in this set is a floor, not a total.

**The corpus is also where the two records stop agreeing, and that is the more useful fact.** Over the
56 runs that have both records, the journal counts **292** slots and the run records **284** — a gap of
8, and *not* spread thinly: it is entirely inside two runs, `wf_9168b84b-530` (17 against 23) and
`wf_d748568f-2c2` (7 against 9), both from other campaigns, both launching agents on byte-identical
prompts that the journal's `key` merges. **The lanes agree because the lanes have no such collisions,
not because the two records are the same number by construction** — which is exactly what a
cross-check is for, and exactly what the lane rows would be worth nothing without.

⚠️ **A tempting wrong version of this check was written first and is recorded here so it is not
written again.** Comparing the journal's distinct-`key` *results* (277) against the summed
`agentCount` (284) appears to agree at 284 if the journal side is counted over all 59 directories
while the `agentCount` side is counted over the 56 that have run records. **The agreement is an
artifact of the two sides having different populations**, and it dissolves the moment both are taken
over the same 56 runs. `agentCount` does not count returns at all: `wf_4d554e0b-df4` has
`agentCount` 4 and zero results. A cross-check between two carriers is worth nothing until the
population is the same on both sides — the same defect, one level up, as the confident totals this
set is about.

### The eleven agents that produced nothing — the cost this set never counted

**7 failed and 4 superseded, out of 128 launched: 8.6 % of the two rungs' and the plan-split's agent
launches returned no report at all.** Not one of them appears in any figure this set published
before this round, because every one of those figures was read out of a script, and a script cannot
be wrong about an agent it launched — it can only be silent about what happened next.

**The clearest case is an authentication failure that killed a whole pass.** EG2 pass 11's first run,
`wf_4d554e0b-df4`, launched four agents — `land:second-datum`, `land:record`, `verify:rerun`,
`verify:ship-call` — and the run metadata records the same log line against all four:

> `failed: Failed to authenticate. API Error: 403 Request not allowed`

Its four `result` fields are `null`, `null`, `null`, `null`; the journal reads **4 launched,
0 returned, 4 failed**. It spent **174,396 tokens** and its longest agent had written a 563 KB
transcript before the refusal. The pass was re-run as
`wf_60fc1374-906`, which is the run every table in this set records as "EG2 pass 11, 4 agents,
2 h 42 m". **The 403 run cost a pass's worth of tokens and left no trace in the accounting**, because
the accounting was a `grep` over `eg2r9.js` and `eg2r9.js` has no idea it ran twice.

**The superseded four are the same defect with the discard on our side rather than the API's.**
A1 pass 1 launched its two-lens audit pair, then launched the *same two prompts again* an hour and a
half later and consumed only the second pair's results; A1 pass 11 and the plan-split's last pass did
the same once each. The four discarded agents left 414 KB, 431 KB, 641 KB and 129 KB of transcript
on disk — **1.6 MB of work that ran, was paid for, and was thrown away.** The journal shows the shape
exactly: two `started` on one key, one `result` on the second of them.

**Where this lands on the zero-yield figure.** The eleven are zero-yield by construction — an agent
that returns no report cannot carry a finding — so they belong in the numerator on the launched
basis and they are already excluded from the returned basis. They raise the fraction on both:
[the yield curve](#the-yield-curve) below reads **67 of 128 launched** where the set previously read
60 of 119 intended. The correction makes the set's own thesis stronger, and it is the only correction
in three rounds that does.

**The wall clock does not move, and that is a second hole rather than a reprieve.** Every duration in
this set is `script mtime → last-report mtime`. A run whose agents all failed writes no report, so
its clock is invisible to the instrument that measures clocks — the 403 run's time is in nobody's
total, including the `~59 h`. The set's wall-clock figures are therefore floors on the same argument
as its agent counts, and [Limits](WORKFLOW-COST-LIMITS.md#what-in-this-set-is-an-estimate-rather-than-a-measurement)
says so at its own site.

**OMISSION — three provenance commands do not reproduce as first written, and are re-pinned above and
here.**

* **`git diff --numstat HEAD` no longer measures EG2.** It did when this set was drafted, because
  EG2 was an uncommitted working tree; EG2 landed as `d272e1fd` and the command now returns nothing
  for it. Everywhere below, EG2's diff is `f7c46c76 → d272e1fd`. The **"tracked + untracked" splits**
  in the ledger (717 + 903 production, 4451 + 3283 instrument) are therefore a **pre-state**: today
  all of it is tracked, and the split is retained only because it is what the passes saw.
* **The test counts were grep-sensitive, and the unanchored pattern OVER-counts.** An earlier draft
  of this bullet said it under-counts and that anchoring makes indented `#[test]` attributes vanish.
  Measured, on all three files: **not one** of the unanchored-only hits is an indented attribute.
  They are doc comments, line comments and string literals — `internal_docs_anchors.rs` today has
  four, of which three are the file's own test-detector (`.any(|l| l.trim() == "#[test]")` and the
  message it prints); `ui_a1_source_census.rs` has eight, all prose about what a `#[test]` means.
  `grep -c "#\[test\]"` gives 9 / 27 / 21 for `internal_docs_anchors.rs` at `f7c46c76`, the same file
  today, and `ui_a1_source_census.rs` at `2a10f3a4`; the anchored `grep -c "^#\[test\]"` gives
  **8 / 23 / 13**, and those are the counts this set now uses. **Confirmed at run time:**
  `cargo test -p boyko-engine --test internal_docs_anchors` prints `running 23 tests`, not 27.
  Provenance row P20 pinned the inflated `21` with the unanchored command and was green over it.
* **`SQLite runs at ~590 test lines per library line`**
  ([Procedure § The ratios](WORKFLOW-COST-PROCEDURE.md#the-ratios-and-the-rule-that-holds-them)) is
  an **external** figure. Nothing in this repository can re-derive it, and no command here does.

**Two caveats stated before the numbers, not after.**

1. **A1 pass 1's wall clock is unreliable** (mtime arithmetic gives 14 h 56 m, which is queueing, not
   work). It is excluded from every A1 wall-clock total below.
2. **The classification into "class 1/2" versus the rest is mine, by origin**, and it is a judgement
   over ~240 findings. The counts are ±. **Every class-1 and class-2 assignment below is
   individually cited**, so the classification can be checked finding by finding rather than trusted
   in aggregate.

---

## The classes

| class | definition |
|---|---|
| **1** | a wrong *production behaviour* — the shipped code does the wrong thing |
| **2** | a *gate that does not observe the defect it claims to catch*, in either of two carriers: **landed** — a mutation to production source leaves it green; or **specified** — the gate does not yet exist, and an audit of its specification shows it cannot observe the mutation it names |
| **3** | a defect **introduced by a repair of this campaign** |
| **4** | prose with no gate consequence |
| **5** | an instrument making a **false claim about its own reach** |
| **env** | measurement-integrity or environment |

Classes 1 and 2 are the only ones that describe the engine. Classes 3 and 5 describe text this
campaign wrote about itself.

**Class 2 carries both forms deliberately, and this document set once printed a narrower definition
than it scored against.** The narrow form — *"a mutation to production source leaves it green"* — is
not applicable to **7 of the 19 class-2 findings**: the whole of EG2 pass 1 (`d26_block.md`'s six
mis-aimed gates, plus the obligation belonging to no rung) was measured **before a line was
written**, so there was no production source to mutate and no gate to leave green. The two carriers
are the same finding — *this gate will not see the defect it is for* — separated only by whether the
gate has been typed yet.

**The alternative was to re-score and drop those 7, and it was rejected**, because they are not
noise: they are the entire evidence base for
[Pass 0](WORKFLOW-COST-PROCEDURE.md#pass-0--the-gate-audit), the procedure's highest-leverage step,
and dropping them would have deleted the evidence rather than corrected the definition. The split is
therefore printed instead of hidden: **7 specified-gate findings (all EG2 pass 1) and 12 landed-gate
findings**, and every row below says which it is by its pass and its citation. Nothing else in this
set changes under either choice — all 13 class-1 findings and the stopping point they establish are
untouched, because class 1 has one definition and always did.

---

## Rung A1 — `D:/wt/ui`, `e7a16fd9` → `2a10f3a4`

| # | pass (script) | agents | wall | inherited insertions | class 1+2 |
|---|---|---|---|---|---|
| 1 | `ui-a1-wf_0c113a2f` | 5 | *(excluded)* | → `e7a16fd9` | — |
| 2 | `ui-a1-red-ledger-wf` | 4 | 1 h 35 m | `e7a16fd9` | **6** |
| 3 | `ui-a1r-remediation-wf` | 5 | 2 h 17 m | → 1204 | **1** |
| 4 | `a1r2.js` | 4 | 1 h 27 m | 1204 → 1673 | **1** |
| 5 | `a1r3.js` | 5 | 2 h 03 m | 1673 → 2102 | 0 |
| 6 | `a1r4.js` | 4 | 1 h 56 m | 2102 → 2505 | **1** |
| 7 | `a1r5.js` | 4 | 2 h 16 m | 2505 → 2866 | 0 |
| 8 | `a1r6.js` — **creates the source census** | 4 | 2 h 29 m | 2866 → 3279 | **1** |
| 9 | `a1r7.js` — census → AST | 4 | 2 h 31 m | 3279 → 4028 | **1** |
| 10 | `a1r8.js` | 4 | 2 h 54 m | 4028 → 4555 | 0 |
| 11 | `a1r9.js` | 4 | **5 h 40 m** | 4555 → 4993 | 0 |
| 12 | `a1r10.js` | 4 | 1 h 40 m | 4993 → 5386 | 0 |
| 13 | `a1-precommit.js` | 2 | 0 h 22 m | 5386 → commit | 0 |
| | **total** | **53** | **~27 h 10 m** | 0 → 9543 | **11** |

**The `agents` column is *intended* launches, read from the scripts.** The journal counts A1 at
**57 launched, 51 returned, 3 failed, 3 superseded** across the same 13 runs; the two rows that
differ are pass 1 (7 launched, 3 returned) and pass 11 (6 launched, 4 returned). Definitions and the
per-lane table: [What "agent run" means](#what-agent-run-means--six-quantities-one-name).

**The 4157-line gap.** The last brief's preamble reads `5386 insertions(+)`; the commit is 9543.
The difference is exactly 4157 — `ui_a1_source_census.rs`, which was **untracked**, so `git diff`
did not count it for six passes. The instrument's true size was invisible to the instrument the
orchestrator used to watch the diff.

### A1's class-1 and class-2 findings, individually

| pass | cls | finding | citation |
|---|---|---|---|
| 2 | 1 | `cargo clippy --workspace --all-targets -- -D warnings` is **RED at `e7a16fd9`** on A1's own `ui_visual_tick`, and the one red leaves `boyko-render`'s entire lint surface unmeasured | `a1_attack.md` §1 |
| 2 | 1 | A discovery reader ordered **before** `ui_visual_tick` loses **every** animating frame (`[1,0,0,…]` vs `[1×20]`); `sprite.rs:38-42` **at `e7a16fd9`** ships the sentence "a repaint one frame late" | `a1_attack.md` §2 |
| 2 | 1 | A non-finite `inv_duration` makes a channel **permanently un-reapable** — 50 frames, `0x7fc00000`, row live | `a1_attack.md` §9 |
| 2 | 2 | `miri_a1_tween::retained_scratch_survives_a_despawn` **cannot fail on two independent counts** | `a1_attack.md` §3 |
| 2 | 2 | Gate 6 is blind to the **whole reap loop** and to **two of four channels**: `Vec::with_capacity(64)` planted in each → `EXIT=0` | `a1_attack.md` §4 |
| 2 | 2 | The `STORAGE_IS_DENSE` const-assert sits on the one component **not** in `ui_pack_inputs!`'s guarded list | `a1_attack.md` §6 |
| 3 | 1 | Pass 3's repair made `start_tween_*(duration_ms = 0.0)` a **silent NO-OP in release** where it used to snap to the endpoint; three doc blocks shipped saying the opposite — found **inside the same pass**, by that pass's own thunk-launched adversary | `a1r_refute.md` §1 |
| 4 | 1 | An **accepted** `duration_ms = f32::MAX` is strictly worse than a refused one: 1 immortal row, **49/49** `set_if_neq` bumps vs the refused `+inf`'s 0 | `a1r2_refute.md` §1 |
| 6 | 2 | `ui_clock_tick` — third member of `UiAnimationSet` — has **no per-frame allocation gate anywhere**; alloc planted → GREEN 12/12 | `a1r4_refute.md` §9 |
| 8 | 2 | (same site) `ui_clock_tick` is in **neither** census and structurally invisible to the alloc gate | `a1r6_refute.md` §6 |
| 9 | 2 | `ui_visual_sink_on_add` is in **no census at all**; a second `if` + a planted alloc → `EXIT=0, 349 passed` | `a1r7_refute.md` F3 |

**ERRATUM — the `a1r_refute.md` row was filed one pass late, and the pass it belongs to was scored
as a zero.** `ui-a1r-remediation-wf` launched **five** labelled agents (`rule:fix-spec`,
`land:code+gates`, `land:docs+guard`, `verify:rerun`, `verify:refute`), and the scratchpad holds the
five `a1r_*.md` reports they wrote. `a1r_refute.md` is the last of those, and its first line reports
the tree it audited: *"restored and verified identical to how I found it (`e7a16fd9`, 10 modified
files, `1204/138`, zero untracked)"* — **1204 is pass 3's own output**, the figure the pass table
above records as pass 3's `→ 1204`. Pass 4 (`a1r2.js`) inherited that tree and its reports are
`a1r2_refute.md` / `a1r2_rerun.md`. Both cells were wrong: pass 3 is **5 agents and one class-1
finding**, not 3 agents and zero.

**This strengthens the case for the repair loop rather than weakening it.** The finding is a
regression that pass 3's *own repair* introduced, and pass 3's *own* verify pair caught it before
the pass ended. That is the property
[Procedure § Pass 3](WORKFLOW-COST-PROCEDURE.md#pass-3--repair-and-verify-while-it-is-still-working)'s
termination rule is built on, and the corpus demonstrates it one level earlier than this document
previously claimed.

**The last three rows are two uncovered surfaces — `ui_clock_tick` stated at pass 6 and restated at
pass 8, `ui_visual_sink_on_add` at pass 9 — 12 agent runs and 6 h 56 m. Neither was ever closed;
both shipped as disclosed limits.**

**ERRATUM — and the pass that would have caught them is pass 0 for one and pass 1 for the other; an
earlier draft assigned both to pass 1, on a reason that is refuted by the tree.** That draft said
`ui_clock_tick` is registered by the rung's **own new code** and that A1's plan did not name it.
Both halves are false, measured in `D:/wt/ui`:

* `git show e7a16fd9^:crates/boyko_ui/src/animation.rs | grep -c "add_system(ui_clock_tick)"` → **1**.
  The registration is in the tree **before** the rung lands; A1 only appends `.key()` to that line.
* `git show e7a16fd9^:docs/UI-PLAN-ANIMATION.md | grep -c ui_clock_tick` → **11**. The plan pass 0
  would read names the system eleven times, including two of its own predicted red mutations
  (*"Register `ui_clock_tick` in `CoreSchedule::Fixed` instead of `Main`"*, *"Drop `ui_clock_tick`
  from `UiAnimationPlugin::build`"*).

  **The current tree yields exactly the false sentence, which is how it survived.** The plan-split
  commit `615cda8f` moved A1's text into `docs/UI-PLAN-ANIMATION-A1.md`, which did not exist at
  `e7a16fd9`; the file that did exist now has **0** hits and the split-off file has 4. This is this
  campaign's own *"a binder pins which file a name resolves to, never which file the sentence
  meant"*, reproduced inside the repair that was written to record it — see
  [Procedure § What to stop writing](WORKFLOW-COST-PROCEDURE.md#what-to-stop-writing).

So **pass 0 reaches `ui_clock_tick` twice over**: requirement 2 covers every system the rung
*registers or modifies*, the plan names it eleven times, and pass 0 reads the tree the registration
is already in. Passes 6 and 8 — 8 agent runs, 4 h 25 m — are the price of not asking.

**`ui_visual_sink_on_add` is the one pass 0 genuinely cannot reach**, and it is the pass-1 catch:
`git grep -c ui_visual_sink_on_add e7a16fd9^ -- crates/` returns nothing and
`git show e7a16fd9^:docs/UI-PLAN-ANIMATION.md | grep -c ui_visual_sink_on_add` is **0**, while at
`e7a16fd9` it is a new `on_add` hook the rung itself registers
(`components.rs` `#[component(storage = "dense", on_add = crate::animation::ui_visual_sink_on_add)]`).
It appears in no plan text, so only [Q1](WORKFLOW-COST-BRIEFS.md#q1--the-mutation-question)'s
every-branch plant at [pass 1](WORKFLOW-COST-PROCEDURE.md#pass-1--the-landing) finds it. *(For the
set membership: `grep -n "in_set(UiAnimationSet)" crates/boyko_ui/src/animation.rs` in `D:/wt/ui`
returns `:414` `ui_clock_tick`, `:418` `ui_visual_tick`, `:419` `ui_tween_reap`; `a1r4_refute.md:125`
calls `ui_clock_tick` "the third member", which is the same set counted in the other direction.)*

**OMISSION — two coordinates in the A1 table are pinned to a revision and do not resolve against the
current tree.** `sprite.rs:38-42` is the state at **`e7a16fd9`**, which is the tree
`a1_attack.md` §2 was written against
(`cd D:/wt/ui && git show e7a16fd9:crates/boyko_ui/src/sprite.rs | sed -n '38,42p'` prints the
"never a lost repaint, but a repaint one frame late" sentence). In the tree today that sentence
survives only as a quoted-and-repudiated line at `sprite.rs:61`, under a heading at `:36` reading
*"a wrong order LOSES the repaint, it does not delay it"* — the finding was applied. Separately, the
`animation.rs:807-822` `ZzIter` coordinate quoted below under
[the yield curve](#the-yield-curve) resolves against **no commit at all**: the probe lived in pass
11's uncommitted working tree and was removed before `2a10f3a4`
(`grep -rn ZzIter crates/` in `D:/wt/ui` hits only comments inside `ui_a1_source_census.rs`). It is
kept as a quotation of `a1r9_refute.md` F1 and must not be read as a live citation.

---

## Rung EG2 — `D:/wt/reflect`, `f7c46c76` → `d272e1fd`

| # | pass (script) | agents | wall | inherited insertions | class 1+2 |
|---|---|---|---|---|---|
| 1 | `reflect-eg2-wf` (audit; landing REFUSED) | 5 | 0 h 56 m | — | **7** |
| — | `kernel-retained-id-filter-wf` (the spun-off fix) | 3 | — | → `f7c46c76` | **1** |
| 2 | `reflect-eg2-round2-wf` | 8 | 2 h 38 m | → 979 | **7** |
| 3 | `eg2r.js` | 5 | 2 h 29 m | 979 → 1456 | **3** |
| 4 | `eg2r2.js` | 5 | 2 h 42 m | 1456 → 1939 | **1** |
| 5 | `eg2r3.js` — **anchor census begins** | 4 | 2 h 39 m | 1939 → 2586 | 0 |
| 6 | `eg2r4.js` | 4 | 2 h 12 m | 2586 → 3492 | 0 |
| 7 | `eg2r5.js` | 4 | 2 h 04 m | 3492 → 4429 | 0 |
| 8 | `eg2r6.js` | 4 | 2 h 06 m | 4429 → 5780 | 0 |
| 9 | `eg2r7.js` | 4 | 2 h 09 m | 5780 → 6317 | **1** |
| 10 | `eg2r8.js` | 4 | 2 h 58 m | 6317 → 7179 | 0 |
| 11 | `eg2r9.js` | 4 | 2 h 42 m | 7179 → 8151 | 0 |
| 12 | `eg2-ship.js` | 2 | 0 h 35 m | 8151 → **8151** | 0 |
| 13 | `eg2-count.js` | 2 | 1 h 12 m | 8151 → **8151** | **1** |
| 14 | `eg2-final.js` — **stopping rule introduced** | 2 | 0 h 49 m | 8151 → **8151** | 0 |
| | **total** | **57** | **~28 h 11 m** | 0 → 12,337 | **21** |

**The `agents` column is *intended* launches, and pass 11 is one pass in two runs.** The journal
counts EG2 at **61 launched, 57 returned, 4 failed, 0 superseded** across **15** runs: `eg2r9.js`
ran once as `wf_4d554e0b-df4`, whose four agents all failed on a 403, and again as `wf_60fc1374-906`,
the run whose 4 agents and 2 h 42 m the row above records. See
[the eleven agents that produced nothing](#the-eleven-agents-that-produced-nothing--the-cost-this-set-never-counted).

**The last three passes changed nothing.** All three briefs carry the identical
`8151 insertions(+), 223 deletions(-)` — measured by reading the preambles. Six agent runs and
2 h 36 m over a net-zero diff.

### EG2's class-1 and class-2 findings, individually

| pass | cls | finding | citation |
|---|---|---|---|
| 1 | 2 | **Six of the rung's ten planned gates observe something other than what they claim** — measured against the kernel *before a line was written*. The rung was REFUSED | `d26_block.md`; refusal `eg2.md` |
| 1 | 2 | An obligation belonging to *no rung*: four trybuild fixtures flip at EG2, the census has one `compile_fail` glob and no `t.pass()`, so the mandated workspace gate is RED at landing. The rung's own ten gates and five REDs mention it **zero** times | `d26_block.md` |
| — | 1 | `add_tag`/`remove_tag` **panic in release** on any entity whose archetype ever hosted a dense component. Fixed by six `is_signature_id` guards → `f7c46c76` | `d26_block.md`; `kfix.md` |
| 2 | 1 | `add_component_by_id` **silently ignores `#[require(...)]`** while its typed twin does not | `eg2_refute.md` §1 |
| 2 | 1 | The ownership transfer is in **no doc comment** — a caller keeping its own live value **double-drops through a safe `pub fn`**: `total drops = 2`, both profiles. Every gate fixture is `Copy`, so none could see it | `eg2_refute.md` §2 |
| 2 | 1 | `remove_component_by_id`'s **DENSE arm enters no `DeferredScopeGuard` and never drains**; the leaked command surfaces during a later unrelated call, hook fires at depth 0 | `eg2_refute.md` §3 |
| 2 | 1 | `AddOutcome::Added` collides with `query::Added<T>` in rustc's path-trimming table — 3 red targets and a **permanent project-wide diagnostic regression** the ballot did not price | `eg2_refute.md` §4 |
| 2 | 2 | Gate 10's stated CI home **cannot build** (`#![cfg(miri)]` + no `--no-fail-fast`) → it never executes in CI | `eg2_refute.md` §5 |
| 2 | 2 | Gate 12 is the only gate on the dense × migration class and **asserts the direction that passes** | `eg2_refute.md` §6 |
| 2 | 2 | No gate asserts a **dense-hosting** entity survives a table attach; `mark_arch_present` is in none of the three by-id walks | `eg2_rerun.md` §8.2 |
| 3 | 1 | `add_component_by_id` drops the **entire** `#[require]` expansion when the caller's id is dense; `g13` cannot see it | `eg2r_refute.md` §1 |
| 3 | 1 | Entity-targeted observers fire through the typed twins and **not** through either new public method | `eg2r_refute.md` §2 |
| 3 | 1 | **12,440 bytes of stack on every call in release**, including the rejection path, under a comment asserting the opposite | `eg2r_refute.md` §3 |
| 4 | 2 | The dense `#[require]` branch's `target_archetype_id` is load-bearing for query visibility and **nothing gates it** — 170 targets green over a one-token mutation | `eg2r2_refute.md` §2 |
| 9 | 2 | `migration_helpers.rs:623` is a dense presence-bit seed **no test covers** (re-discovery of pass 2's `eg2_rerun.md` §8.2) | `eg2r7_refute.md` §7 |
| 13 | 2 | **Both serialize load walks (`load_writer.rs:716` and `:855`) are expressible and ungated** — found by re-deriving all eight sites from the criterion, disagreeing with the previous pass. Probes compiled and run; `boyko-serialize --all-targets` EXIT=0 over both | `eg2cnt_land.md` Act 1 |

**The pass-13 row is the single most important finding in this corpus for the repair discipline, and
it is deliberately not used to argue for thirteen passes.** It was produced by *re-deriving the
criterion*, a method that costs one pass and applies at any pass — see
[Procedure § R2](WORKFLOW-COST-PROCEDURE.md#r2--a-repair-is-re-derived-from-the-criterion-never-substituted-from-a-list).
Three earlier passes had *patched* the same claim from a substitution list and been wrong three times
running; the re-derivation produced **five expressible sites, not three**, and named the two the
patches had missed.

---

## The yield curve

Class 1 + class 2 findings, by pass:

```
A1    P1  P2  P3  P4  P5  P6  P7  P8  P9  P10 P11 P12 P13
       -   6   1   1   0   1   0   1   1   0   0   0   0     = 11

EG2   P1  P2  P3  P4  P5  P6  P7  P8  P9  P10 P11 P12 P13 P14
       7   7   3   1   0   0   0   0   1   0   0   0   1   0  = 20  (+1 in the spun-off fix workflow)
```

Counted from the two tables above, treating `d26_block.md`'s *"six of its ten gates"* as the six
class-2 findings it names. The two tables carry **13 class-1 rows** and **14 class-2 rows**; the six
for one on that single row make the finding totals **13 class-1 and 19 class-2 over both rungs.**
Both row counts are pinned by provenance rows P28 and P29, because this curve is the sole evidence
for the stopping point and nothing re-ran it before.

**Scored against the class-2 definition [above](#the-classes), both carriers.** The 7 at EG2 pass 1
are the **specified** carrier — a plan's gates audited before any code existed; the other 12 are the
**landed** carrier. The curve is the sole evidence for the stopping point, so it is stated here
against the definition the document prints and not a narrower one: under the landed carrier alone the
totals read **13 class-1 and 12 class-2**, EG2's P1 column reads `0` instead of `7`, and **every
bullet below is unchanged**, because none of them turns on a pass-1 class-2 count.

* **Wrong production behaviour ends at A1 pass 4 and EG2 pass 3.** No pass after that found one, in
  either lane.
* **All 13 class-1 findings and 14 of the 19 class-2 findings fall in passes 1-4**, at **18** of 53
  *intended* runs in A1 (passes 1-4 = 5 + 4 + 5 + 4) and 23 of 57 in EG2 (5 + 8 + 5 + 5). Counted from
  the journal instead — [the six quantities](#what-agent-run-means--six-quantities-one-name) — A1's
  passes 1-4 are **20 of 57 launched** (7 + 4 + 5 + 4) and **16 of 51 returned**, and EG2's are
  **23 of 61 launched** and **23 of 57 returned**. The claim survives every basis; only the
  arithmetic moves.
* The 5 that fall outside are: `ui_clock_tick` at A1 passes 6 and 8; `ui_visual_sink_on_add` at A1
  pass 9; EG2's `migration_helpers.rs:623` at pass 9, which is a re-discovery of its own pass-2
  finding; and EG2's pass-13 re-derivation. **Three of the five belong to a finding the corpus
  states more than once** — the two `ui_clock_tick` rows and the `migration_helpers.rs:623`
  re-discovery.

**Passes with zero class-1 and zero class-2 yield:** A1 passes 5, 7, 10, 11, 12, 13 (6 of 13
passes, **25 launched / 23 returned**, **~14 h 55 m** of 27 h 10 m). EG2 passes 5, 6, 7, 8, 10, 11,
12, 14 (8 of 14, **32 launched / 28 returned** — pass 11 twice, see below — **~16 h 05 m** of
28 h 11 m). Plus the plan-split's 3 passes / **10 launched / 9 returned** / **3 h 40 m** (script
mtime → last-report mtime: `ui-plan-split.js` 20:31 → `split_*.md` 21:46; `split-fix.js` 21:51 →
`splitfix_*.md` 22:19; `split-final.js` 22:20 → `splitfin_*.md` 00:17).

**Combined: 17 of 30 passes (18 of 31 runs), 67 of 128 agent launches — 52 % — and 60 of 117 agent
returns — 51 %; ~34 h 40 m of ~59 h, 59 % of the wall clock.** Stated on both bases because the
headline is work delivered against cost incurred and those are two different denominators; see
[the six quantities](#what-agent-run-means--six-quantities-one-name). The launched basis is the
higher of the two because EG2 pass 11 ran twice — the second run is the one the tables record, and
the first is the 403 that killed four agents. Every earlier printing of this line used the *intended*
count and read `60 of 119`; the numerator 60 survives as the returned-basis numerator by coincidence,
not by construction.

*(Three earlier drafts read "19 of the 30 passes — 67 of 116 agent runs", then "18 … 63 of 106
… ~37 h of ~59 h", then "17 … 60 of 119". The pass set lost A1 pass 3, which was scored zero only
because its adversary's report had been filed under pass 4; the first two run denominators were the
blind `await`-only count and the third was the script-text count. The `67 of 116` of the first draft
and the `67 of 128` above are unrelated numbers that happen to share a numerator. The wall-clock
arithmetic: 14 h 55 m + 16 h 05 m + 3 h 40 m = 34 h 40 m; 27 h 10 m + 28 h 11 m + 3 h 40 m =
59 h 01 m.)*

⚠️ **The wall-clock leg is the weakest of the three, and is the only headline figure no provenance
row binds.** It is derived from **file mtimes** — first-script to last-report — which measures the
elapsed wall time of a pass including every gap in which nothing ran. The harness records the actual
figure, `durationMs`, in each run record, and the two disagree by **17×** on the very pass
contributing the largest block of this numerator: A1 pass 11 is 5 h 40 m by mtime and 0 h 20 m by
`durationMs`. The set now also has a denominator it calls superior to both — **tokens**, measured
directly and independent of how "agent run" is defined.

The round-2 verification put the same 18 runs at **~52 %** on `durationMs` and **~48 %** on tokens.
**Those two figures are recorded here as reported, not as measured by this document** — neither has
been re-derived at this site and neither carries a row, which is exactly the provenance this set
refuses everywhere else. What is certain is the direction: **59 % is the highest of the three
available bases and the only unpinned one, and the set's own newer evidence argues for a lower
number.** Re-basing it onto `durationMs` and tokens, with a row for each, is owed work — recorded
here rather than done, because doing it is a measurement pass and not a prose edit.

The most expensive single pass is A1 pass 11 — 4 agents by the script, **6 launched and 4 returned**
by the journal, **5 h 40 m** — and its top finding is a probe *it inherited from pass 10's own
landing*:

> `crates/boyko_ui/src/animation.rs:807-822` ships a live `struct ZzIter` probe in **tracked
> production source**, detaching an `#[allow]` and a doc block from `ui_tween_reap`. *"both prior
> reports certify a tree that does not exist on disk"*, and the census row covering it *"asserts the
> restore that did not happen"*. — `a1r9_refute.md` F1

That is the campaign's most dangerous single artifact, and it was manufactured by the campaign.

---

## The line ledger, measured not quoted

### The gross table reproduces

| | production | test / instrument | prose |
|---|---|---|---|
| **A1** (`2a10f3a4`) | 448 src + 45 manifest = **493** | **6089** | **2961** |
| **EG2** (`f7c46c76` → `d272e1fd`) | 717 + 903 = **1620** | 4451 + 3283 = **7734** | **2983** |

### The production column is mostly prose

Added lines classified for this document:

| file | added | blank | comment | **code** |
|---|---|---|---|---|
| A1 `crates/boyko_ui/src/animation.rs` | 298 | 1 | **279** | **18** |
| A1 `crates/boyko_render/src/ui/gather.rs` | 99 | 2 | 75 | **22** |
| A1 `crates/boyko_ui/src/sprite.rs` | 35 | 0 | 35 | **0** |
| A1 `crates/boyko_ui/src/layout.rs` | 6 | 0 | 6 | **0** |
| A1 `crates/boyko_ui/src/text/measure.rs` | 10 | 0 | 10 | **0** |
| **A1 total** | **448** | 3 | **405** | **40** |
| EG2 `…/commands/migration_helpers.rs` | 641 | 39 | 273 | 329 |
| EG2 `…/ecs_master/seam_by_id.rs` (untracked) | 903 | 39 | 470 | 394 |
| EG2 `…/ecs_master/component_api.rs` | 43 | 2 | 26 | 15 |
| EG2 `…/component_registry/tags.rs` | 28 | 1 | 15 | 12 |
| EG2 `…/ecs_master/mod.rs` | 5 | 1 | 2 | 2 |
| **EG2 total** | **1620** | 82 | **786** | **752** |

**A1's 22 `gather.rs` code lines are a `const _: () = assert!` macro** — a compile-time assertion,
i.e. an instrument that happens to live under `src/`. The 18 in `animation.rs` are, in full:
**twelve** are the `#[cold] #[inline(never)] fn invalid_tween_duration` helper whose entire body is a
`debug_assert!` (2 attribute lines, the `fn` line, the 8-line `debug_assert!`, the closing brace),
one is an `#[allow]` attribute, one moves the completion boundary, and **four are the guard** —
12 + 1 + 1 + 4 = 18. Enumerated by
`git show 2a10f3a4 -- crates/boyko_ui/src/animation.rs | grep "^+" | grep -v "^+++" | sed 's/^+//' |
grep -vE '^\s*(//|$)' | cat -n` in `D:/wt/ui`, which prints exactly 18 numbered lines. *(An earlier
draft said "seven", and its own enumeration then summed to 13 rather than 18.)*

**So the twelve remediation passes' release-behaviour delta is five lines**, and the instrument
around them is 6089.

### The two instruments that caught nothing

| instrument | born | shipped | tests | production defects caught |
|---|---|---|---|---|
| `crates/boyko_ui/tests/ui_a1_source_census.rs` | pass 8, **1403 lines / 4 tests** (`a1r6_landCode.md`, quoted) | **4157 lines / 13 tests** | 13 | **0** |
| `tests/internal_docs_anchors.rs` | pre-existing, **1991 lines / 8 tests** at `f7c46c76` | **6167 lines / 23 tests** | 23 | **0** |

Together **8375 added lines — 61 % of the campaign's 13,823 instrument lines** — and they were the
subject of **16 of the 30 passes — 17 runs, 62 agents launched, 56 returned, ~35 h** (A1 passes 8-13
= 6 passes / 24 launched / 22 returned / 15 h 36 m; EG2 passes 5-14 = 10 passes in **11 runs** /
38 launched / 34 returned / 19 h 26 m). The plan-split adds a further 3 passes, 10 launched and
9 returned, devoted to the plan documents.

**Their recorded "catches" are self-inflicted, verbatim from the corpus:**

* `eg2r3_landCode.md`: *"the census caught it — 7 stale anchors pointing into the file I had just
  edited. These were created by me"*
* `eg2r5_landText.md`: *"The census caught five by content on the first re-run"* — displaced by the
  same pass's insertions
* `eg2r6_landCode.md`: *"The census caught 7 of them."*

**And they are unbounded by construction.** The source census pins the *shape* of the source, so
every pass found one more construct it cannot see, and added a row and a longer disclosure:
`a1r6_refute.md` finds a macro-introduced branch and a `&&` short-circuit; `a1r7_refute.md` F1 finds
*"a fourth termination condition in an `impl Drop`… invisible to ALL SEVEN census tests"*;
`a1r8_refute.md` finds an operator-trait impl; `a1r10_refute.md` finds a fifth. **An enumeration over
source shape has an open residue; it cannot terminate on its own merits, only on a rule.**

The same is true of the cap census. Its headline figure **moved between passes because the passes
added ledgers**: `66 cap row(s); 54 … 12` at pass 10 (`eg2r8_landCode.md`) → `84 cap row(s) in this
file; 70 sit at 0 over an empty live set, 14 over a live one` at pass 11 (`eg2r9_landText.md`, and
re-measured identically in `eg2ship_land.md`, `eg2fin_land.md`). **83 % of the cap rows are caps over
an empty population.** `eg2r8_rerun.md` §(c) then found that *"the literal cap rows written in the
file are 45 … plus 4 scalars"*, so even "66 cap rows in this file" is true only under one definition
of *row*.

### What did catch things

| instrument | lines | what it caught |
|---|---|---|
| **`crates/boyko_ecs/tests/retained_id_walk_pool_skip.rs`** (`f7c46c76`) | 621 (391 code) over **75** production lines (18 code) | The session's one unambiguous production bug: `Archetype::component_ids()` retains ids owning no pool; six migration sites unwrapped it; **release-present panic reachable from plain `spawn`/`insert`/`remove`/`add_tag`**. **One workflow. Never revisited.** |
| `crates/boyko_render/tests/ui_a1_sink_reaches_discovery.rs` | 100 | The ordering defect — but *after the fact*; the defect was found by a throwaway probe (`a1_attack.md` §2) |

**Two of the corpus's 13 class-1 findings were caught by landed gates — the repository's own mandated
ones — and the other eleven by an agent reading source and writing a throwaway probe. In both gate
cases the agent's contribution was RUNNING the gate and diagnosing what its red was masking.** That
is the observation the procedure is built on, and it is not the one an earlier draft of this page
made: *"Not one was found by a landed gate"* is refuted by this page's own class-1 table, two rows up.

| # | the landed gate | its red | what the agent added |
|---|---|---|---|
| A1 pass 2 | `cargo clippy --workspace --all-targets -- -D warnings`, named as a build command by `CLAUDE.md` at `a1_attack.md:13` | `EXIT=101` at `e7a16fd9`, three `error:` lines, `boyko-ui` lib and lib-test failing to compile under clippy (`a1_attack.md` §1). *The run pasted at `:17-18` is the narrowed `cargo clippy -p boyko-ui --all-targets -- -D warnings`, not the `--workspace` form — an earlier draft cited those two lines for the `--workspace` command they do not contain* | that the one red **masked a whole crate**: `-p boyko-ui -p boyko-render --all-targets --keep-going` → `EXIT=101` with `grep -c 'Checking boyko-render'` = **0**; with the one-line fix applied, `EXIT=0` and `Checking boyko-render` = 1 |
| EG2 pass 2 | `cargo test -p boyko-ecs --all-targets --no-fail-fast` | `ECS_ALLTARGETS_EXIT=101`, `error: 3 targets failed` — `compile_fail_chunk`, `enable_filter_compile_fail`, `query_change_detection_compile_fail` (`eg2_refute.md` §4) | that the cause was **not** four stale fixtures but a permanent project-wide diagnostic regression — `AddOutcome::Added` colliding with `query::Added<T>` in rustc's path-trimming table — which the ballot had not priced |

**This changes the design's justification, and the change is not cosmetic.** If no landed gate ever
caught anything, the only rational instrument is the throwaway probe, and
[Pass 1](WORKFLOW-COST-PROCEDURE.md#pass-1--the-landing)'s "make the probe the deliverable" is the
whole answer. What actually happened is narrower and more useful: **the gates that caught things were already
there and already red — unrun in A1's case, run-but-undiagnosed in EG2's.** A1 pass 1 did not ask
[Q2](WORKFLOW-COST-BRIEFS.md#q2--the-gate-exit-question) at all. EG2's landing agents *did* run
theirs and reported the exit honestly — `eg2_refute.md:115`: *"Both landing agents disclosed this
honestly and I confirm their counts to the unit. What neither said: …"* — and what the adversary
added was the **diagnosis**, which is what this table's own last column says. *(An earlier draft
wrote "and nobody had run them", which holds for A1 and not for EG2.)* So the procedure needs *two*
obligations,
not one — [Q1](WORKFLOW-COST-BRIEFS.md#q1--the-mutation-question) for the probe, and
[Q2](WORKFLOW-COST-BRIEFS.md#q2--the-gate-exit-question) for running every gate `CLAUDE.md` names,
unpiped, with `$?` on the next line. Q2 costs zero to ask and was not asked at A1 pass 1. Deleting
instrument (the [class rule](WORKFLOW-COST-PROCEDURE.md#the-ratios-and-the-rule-that-holds-them))
therefore removes census apparatus this campaign built; it must not be read as removing the standing
repo gates, which are the only instrument in this corpus with a nonzero class-1 yield.

---

## The prose ledger

**5944 lines across the two rungs.** Measured splits:

**The Russian mirror is 1955 of 5944 — 32.9 %** (A1 `docs/ru/OPEN-QUESTIONS.md` 814; EG2 1120 +
`docs/ru/README.md` 21). Parity between the twins was re-verified in at least 14 separate agent runs.
The standing note on that instrument is already in the project's memory: parity tests *sameness*,
never truth.

**The owner-facing channel is 60 % about the instrument, not the engine.** Block boundaries taken
by extracting the headings from the added-lines stream of each `docs/OPEN-QUESTIONS.md` diff:

| rung | added | about the engine | about the record / the instrument |
|---|---|---|---|
| A1 | 784 | 325 (41 %) — the unregistered `ui_render_discovery`; the ~6-day immortal tween; the flaky pre-existing `zero_alloc` suite | **459 (59 %)** — "the no-syntax edge class is a LIST"; "the residue is FIFTEEN, not thirteen"; "the census now claims only what it MEASURES"; "widening `GATED_DOCS` does NOT close the anchor-rot class" |
| EG2 | 1097 | 384 (35 %) + 37 index — `DenseStore::arch_presence`; `#[require(C)]` for dense `C`; CI's Miri sweep silencing eleven tests; the observers' remove half | **676 (62 %)** — "TWELVE `.rs` → `.rs` citations out of bounds" (96) and **"The round that gated the corpus's counts wrote two ungated ones of its own, and all four record coordinates were stale one round later" (580)** |
| **both** | **1881** | **709 (38 %)** | **1135 (60 %)** |

That 580-line block has a 580-line Russian twin. **1160 lines — more than the 792 executable
production lines of both rungs combined** (A1's **40** plus EG2's **752**, from
[the line ledger](#the-production-column-is-mostly-prose) above; an earlier draft wrote 752, which is
EG2's column alone). Its seven sub-sections are titled: a pre-state left in the
present tense · a claim about the instrument's own REACH · four record coordinates that had all
moved · a stated non-coverage · every measurable claim with its disposition · the rung's stated
limit · what the commit message must disclose. **None of it is about the ECS.**

**The 38 % that is genuine owner escalation is not noise and must survive** — see
[Procedure § What to stop writing](WORKFLOW-COST-PROCEDURE.md#what-to-stop-writing).

---

## The provenance table, re-run by a gate

Every figure above was measured; this table is what keeps it measured. `tests/workflow_cost_docs.rs`
parses the block below and, for each row, asserts a **four-link chain**:

1. `ctx` — a verbatim fragment of prose — occurs in `doc`, outside this block. *(A prose edit reds.)*
2. `fig` occurs inside `ctx`. *(The sentence is bound to its number.)*
3. `fig` occurs inside `out`. *(The number is bound to the command's output.)*
4. `cmd`, run in `cwd`, exits 0 and prints exactly `out`. *(The output is bound to the tree.)*

Break any link and the gate goes red naming the row. This is
[R4](WORKFLOW-COST-PROCEDURE.md#r4--never-write-a-figure-about-the-edit-set-you-are-inside) made
mechanical: a figure that a command cannot reproduce does not stay in the prose.

`cwd` is one of **six** roots the test resolves: `reflect` (this worktree), `ui` (`D:/wt/ui`),
`scripts` (the workflow-script directory), `scratch` (the report scratchpad), `journal` (the
harness's per-run event log, `<session>/subagents/workflows/`), and — newest — `runmeta` (the
harness's per-run summary records, `<session>/workflows/`), which rows P38–P43 use. **Rows whose
root is absent are not skipped** — the test asserts the number of rows it actually executed against
a literal, so a missing root reds with the row ids rather than passing over them. *(`scratch` was
for a time documented and used by no row, until P27 took it. A root nothing exercises is a claim
about the gate's reach that the gate does not make.)*

⚠️ **This sentence has now been stale twice, and the second time the gate itself was the correct
copy** — the test declared six roots and six rows used `runmeta` while this paragraph still said
five and still called `journal` "new this round". It is the site that tells a reader how far the
gate reaches, so a reader who trusts it is licensed to trust everything downstream of it. When a
root is added, this sentence is part of the change, not a follow-up to it.

**A fifth link the chain does not have, stated here because two rounds found it the hard way:
nothing asks whether `cmd` measures the thing `ctx` names.** Rows P05, P06, P07 and P26 re-ran
`grep -c "await agent("` and were green for as long as that command was the wrong instrument; then
they re-ran a better command over the same workflow sources and were green for another round while
the sources still could not answer the question. The chain binds prose → command → tree; it cannot
bind command → *meaning*, and no addition to it can.

### P31-P37 are the first rows in this set with a carrier independent of the thing they check

**Every row before them re-reads the artifact the figure was derived from.** P05 counts agents by
grepping the workflow scripts, and the sentence it certifies was written by grepping the workflow
scripts. That is one carrier wearing two names: the row cannot disagree with the prose, because a
mistake about what the scripts mean is made identically on both sides. It is exactly why the agent
count was wrong through two adversarial rounds and green in the gate through both of them.

**P31-P37 read `journal.jsonl`, which the harness writes as the agents run.** Nothing that produced
the figures produced the journal, and no reading of a script can change what is in it. When the
script-derived count and the journal disagree — as they do, at 119 against 128 — the disagreement is
information, where two script-derived counts agreeing is not. **The principle generalises past this
row: a check is worth what its carrier is independent of, and adding a second command over the same
artifact adds a command, not a check.**

**Two limits, at the row rather than in a footnote.**

* **This root is not in the repository.** `journal` resolves under the session scratchpad path, like
  `ui`, `scripts` and `scratch` — so on any other machine, or after this session's directory is
  cleaned up, these rows **red**; they do not skip. That is the same trade the module header of
  `tests/workflow_cost_docs.rs` already states for the other three external roots, and the same
  answer applies: if it ever matters more than the guarantee, delete the file — do not lower the
  executed-row floor to match a green.
* **A journal counts workflow-launched agents and nothing else.** Agents the orchestrator started
  directly through its own Agent tool — outside any workflow — appear in no `journal.jsonl` and in no
  figure here. **These rows are not a census of the campaign's agents; they are a census of its
  workflow-launched agents**, which is what every "agent run" in this set has always meant, and the
  two are not known to be the same set.

```provenance
id:  P01
doc: WORKFLOW-COST-PLAN.md
ctx: A1's remediation commit `2a10f3a4` is 9543 insertions
fig: 9543 insertions
cwd: ui
cmd: git show --shortstat --format="" 2a10f3a4 | sed "s/^ *//"
out: 25 files changed, 9543 insertions(+), 236 deletions(-)

id:  P02
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | **total** | **57** | **~28 h 11 m** | 0 → 12,337 | **21** |
fig: 12,337
cwd: reflect
cmd: git diff --shortstat f7c46c76 d272e1fd | sed "s/^ *//" | sed "s/12337/12,337/"
out: 28 files changed, 12,337 insertions(+), 307 deletions(-)

id:  P03
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | **A1 total** | **448** | 3 | **405** | **40** |
fig: 448
cwd: ui
cmd: git show --numstat --format="" 2a10f3a4 | awk '$3 ~ /^crates\/.*\/src\// {s+=$1} END{print s}'
out: 448

id:  P04
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | **EG2 total** | **1620** | 82 | **786** | **752** |
fig: 1620
cwd: reflect
cmd: git diff --numstat f7c46c76 d272e1fd | awk '$3 ~ /src\// {s+=$1} END{print s}'
out: 1620

id:  P05
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | A1 | 13 | 13 | 53 | **53** | **57** | **51** | 3 | 3 |
fig: 53
cwd: scripts
cmd: echo $(( $(cat ui-a1-wf_0c113a2f-027.js ui-a1-red-ledger-wf_26c81a18-dfe.js ui-a1r-remediation-wf_d7f3bbbf-104.js a1r2.js a1r3.js a1r4.js a1r5.js a1r6.js a1r7.js a1r8.js a1r9.js a1r10.js a1-precommit.js | grep -c "agent(") + $(grep -c "key: '" ui-a1-wf_0c113a2f-027.js) - 1 ))
out: 53

id:  P06
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | EG2 | 14 | **15** | 57 | **61** | **61** | **57** | 4 | 0 |
fig: 57
cwd: scripts
cmd: echo $(( $(cat reflect-eg2-wf_f70c0232-045.js reflect-eg2-round2-wf_578ffc29-14d.js eg2r.js eg2r2.js eg2r3.js eg2r4.js eg2r5.js eg2r6.js eg2r7.js eg2r8.js eg2r9.js eg2-ship.js eg2-count.js eg2-final.js | grep -c "agent(") + $(grep -c "^  \['" reflect-eg2-wf_f70c0232-045.js) - 1 ))
out: 57

id:  P07
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | plan-split | 3 | 3 | 9 | **9** | **10** | **9** | 0 | 1 |
fig: 9
cwd: scripts
cmd: cat ui-plan-split.js split-fix.js split-final.js | grep -c "agent("
out: 9

id:  P08
doc: WORKFLOW-COST-PLAN.md
ctx: it adds **40 lines**
fig: 40
cwd: ui
cmd: git show 2a10f3a4 --format="" -- 'crates/*/src/*' | grep "^+" | grep -v "^+++" | sed "s/^+//" | grep -cvE "^[[:space:]]*(//|$)"
out: 40

id:  P09
doc: WORKFLOW-COST-EVIDENCE.md
ctx: 12 + 1 + 1 + 4 = 18
fig: 18
cwd: ui
cmd: git show 2a10f3a4 --format="" -- crates/boyko_ui/src/animation.rs | grep "^+" | grep -v "^+++" | sed "s/^+//" | grep -cvE "^[[:space:]]*(//|$)"
out: 18

id:  P10
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **A1's 22 `gather.rs` code lines are a `const _: () = assert!` macro**
fig: 22
cwd: ui
cmd: git show 2a10f3a4 --format="" -- crates/boyko_render/src/ui/gather.rs | grep "^+" | grep -v "^+++" | sed "s/^+//" | grep -cvE "^[[:space:]]*(//|$)"
out: 22

id:  P11
doc: WORKFLOW-COST-EVIDENCE.md
ctx: more than the 792 executable
fig: 792
cwd: reflect
cmd: echo $((40 + 752))
out: 792

id:  P12
doc: WORKFLOW-COST-PROCEDURE.md
ctx: **1121 of `docs/UI-PLAN-ANIMATION-A1.md`'s 1426 lines**
fig: 1121
cwd: ui
cmd: awk 'END{print NR-305}' docs/UI-PLAN-ANIMATION-A1.md
out: 1121

id:  P13
doc: WORKFLOW-COST-PROCEDURE.md
ctx: now prints **393** and **1426**
fig: 393
cwd: ui
cmd: wc -l < docs/UI-PLAN-ANIMATION.md | tr -d " "
out: 393

id:  P14
doc: WORKFLOW-COST-PROCEDURE.md
ctx: 1292 insertions to `docs/UI-PLAN-ANIMATION.md`
fig: 1292
cwd: ui
cmd: git show --numstat --format="" 2a10f3a4 | awk '$3=="docs/UI-PLAN-ANIMATION.md"{print $1}'
out: 1292

id:  P15
doc: WORKFLOW-COST-EVIDENCE.md
ctx: quoted-and-repudiated line at `sprite.rs:61`
fig: 61
cwd: ui
cmd: grep -n "one frame late" crates/boyko_ui/src/sprite.rs | cut -d: -f1
out: 61

id:  P16
doc: WORKFLOW-COST-BRIEFS.md
ctx: lines in three files**
fig: three
cwd: scripts
cmd: grep -rl "3-in-5" *.js | wc -l | tr -d " " | sed "s/^3$/three/"
out: three

id:  P17
doc: WORKFLOW-COST-BRIEFS.md
ctx: migration_helpers.rs:3054-3056
fig: 3054-3056
cwd: reflect
cmd: awk 'NR>=3054 && NR<=3056' crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs | grep -c "takes ownership" | sed "s/^1$/3054-3056/"
out: 3054-3056

id:  P18
doc: WORKFLOW-COST-EVIDENCE.md
ctx: returns `:414` `ui_clock_tick`, `:418` `ui_visual_tick`, `:419` `ui_tween_reap`
fig: 414
cwd: ui
cmd: grep -n "in_set(UiAnimationSet)" crates/boyko_ui/src/animation.rs | head -1 | cut -d: -f1
out: 414

id:  P19
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **4157 lines / 13 tests**
fig: 4157
cwd: ui
cmd: git show 2a10f3a4:crates/boyko_ui/tests/ui_a1_source_census.rs | wc -l | tr -d " "
out: 4157

id:  P20
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **4157 lines / 13 tests**
fig: 13
cwd: ui
cmd: git show 2a10f3a4:crates/boyko_ui/tests/ui_a1_source_census.rs | grep -c "^#\[test\]"
out: 13

id:  P21
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **6167 lines / 23 tests**
fig: 6167
cwd: reflect
cmd: wc -l < tests/internal_docs_anchors.rs | tr -d " "
out: 6167

id:  P22
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **1991 lines / 8 tests** at `f7c46c76`
fig: 1991
cwd: reflect
cmd: git show f7c46c76:tests/internal_docs_anchors.rs | wc -l | tr -d " "
out: 1991

id:  P23
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | 621 (391 code) over **75** production lines (18 code) |
fig: 621
cwd: reflect
cmd: git show f7c46c76 --numstat --format="" | awk '$3=="crates/boyko_ecs/tests/retained_id_walk_pool_skip.rs"{print $1}'
out: 621

id:  P24
doc: WORKFLOW-COST-PROCEDURE.md
ctx: is **1351 insertions** — EG2 pass 8 (`eg2r6.js`), `4429 → 5780`
fig: 1351
cwd: scripts
cmd: echo $(( $(grep -o "5780 insertions" eg2r7.js | head -1 | cut -d" " -f1) - $(grep -o "4429 insertions" eg2r6.js | head -1 | cut -d" " -f1) ))
out: 1351

id:  P25
doc: WORKFLOW-COST-PROCEDURE.md
ctx: `const SITES` at `:501` carries 22 rows of `func: "ui_visual_tick"`
fig: 22
cwd: ui
cmd: git show 2a10f3a4:crates/boyko_ui/tests/ui_a1_source_census.rs | grep -c 'func: "ui_visual_tick"'
out: 22

id:  P26
doc: WORKFLOW-COST-EVIDENCE.md
ctx: at **18** of 53 *intended* runs in A1 (passes 1-4 = 5 + 4 + 5 + 4)
fig: 18
cwd: scripts
cmd: echo $(( $(cat ui-a1-wf_0c113a2f-027.js ui-a1-red-ledger-wf_26c81a18-dfe.js ui-a1r-remediation-wf_d7f3bbbf-104.js a1r2.js | grep -c "agent(") + $(grep -c "key: '" ui-a1-wf_0c113a2f-027.js) - 1 ))
out: 18

id:  P27
doc: WORKFLOW-COST-EVIDENCE.md
ctx: 10 modified files, `1204/138`, zero untracked
fig: 1204/138
cwd: scratch
cmd: head -1 a1r_refute.md | grep -o "1204/138"
out: 1204/138

id:  P28
doc: WORKFLOW-COST-EVIDENCE.md
ctx: The two tables carry **13 class-1 rows**
fig: 13
cwd: reflect
cmd: grep -cE '^\| [^|]+ \| 1 \|' docs/WORKFLOW-COST-EVIDENCE.md
out: 13

id:  P29
doc: WORKFLOW-COST-EVIDENCE.md
ctx: and **14 class-2 rows**
fig: 14
cwd: reflect
cmd: grep -cE '^\| [^|]+ \| 2 \|' docs/WORKFLOW-COST-EVIDENCE.md
out: 14

id:  P30
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | **EG2 total** | **1620** | 82 | **786** | **752** |
fig: 752
cwd: reflect
cmd: git diff f7c46c76 d272e1fd -- '*/src/*' | grep "^+" | grep -v "^+++" | sed "s/^+//" | grep -cvE "^[[:space:]]*(//|$)"
out: 752

id:  P31
doc: WORKFLOW-COST-EVIDENCE.md
ctx: The journal counts A1 at **57 launched, 51 returned, 3 failed, 3 superseded** across the same 13 runs
fig: 57 launched
cwd: journal
cmd: cat wf_0c113a2f-027/journal.jsonl wf_26c81a18-dfe/journal.jsonl wf_d7f3bbbf-104/journal.jsonl wf_ff0ecd3b-d2e/journal.jsonl wf_f20790c2-73d/journal.jsonl wf_f2b04d7a-f1a/journal.jsonl wf_d5cfec2f-abf/journal.jsonl wf_5710bbc2-543/journal.jsonl wf_5fa11c84-2b1/journal.jsonl wf_1e0cf27c-f62/journal.jsonl wf_fdbcb66f-0d2/journal.jsonl wf_659b2d2f-75b/journal.jsonl wf_db0a2bc6-ada/journal.jsonl | grep -c '"type":"started"' | sed 's/$/ launched/'
out: 57 launched

id:  P32
doc: WORKFLOW-COST-EVIDENCE.md
ctx: The journal counts A1 at **57 launched, 51 returned, 3 failed, 3 superseded** across the same 13 runs
fig: 51 returned
cwd: journal
cmd: cat wf_0c113a2f-027/journal.jsonl wf_26c81a18-dfe/journal.jsonl wf_d7f3bbbf-104/journal.jsonl wf_ff0ecd3b-d2e/journal.jsonl wf_f20790c2-73d/journal.jsonl wf_f2b04d7a-f1a/journal.jsonl wf_d5cfec2f-abf/journal.jsonl wf_5710bbc2-543/journal.jsonl wf_5fa11c84-2b1/journal.jsonl wf_1e0cf27c-f62/journal.jsonl wf_fdbcb66f-0d2/journal.jsonl wf_659b2d2f-75b/journal.jsonl wf_db0a2bc6-ada/journal.jsonl | grep -c '"type":"result"' | sed 's/$/ returned/'
out: 51 returned

id:  P33
doc: WORKFLOW-COST-EVIDENCE.md
ctx: counts EG2 at **61 launched, 57 returned, 4 failed, 0 superseded** across **15** runs
fig: 61 launched
cwd: journal
cmd: cat wf_f70c0232-045/journal.jsonl wf_578ffc29-14d/journal.jsonl wf_bdc3afc1-308/journal.jsonl wf_31440cb2-f48/journal.jsonl wf_461d1a48-f12/journal.jsonl wf_b536399f-3a2/journal.jsonl wf_79ca0106-d99/journal.jsonl wf_007968d3-b85/journal.jsonl wf_404f7e21-e85/journal.jsonl wf_f18cb9cf-cf3/journal.jsonl wf_4d554e0b-df4/journal.jsonl wf_60fc1374-906/journal.jsonl wf_a4f812d6-68e/journal.jsonl wf_93f45510-014/journal.jsonl wf_3cc77530-a4d/journal.jsonl | grep -c '"type":"started"' | sed 's/$/ launched/'
out: 61 launched

id:  P34
doc: WORKFLOW-COST-EVIDENCE.md
ctx: counts EG2 at **61 launched, 57 returned, 4 failed, 0 superseded** across **15** runs
fig: 57 returned
cwd: journal
cmd: cat wf_f70c0232-045/journal.jsonl wf_578ffc29-14d/journal.jsonl wf_bdc3afc1-308/journal.jsonl wf_31440cb2-f48/journal.jsonl wf_461d1a48-f12/journal.jsonl wf_b536399f-3a2/journal.jsonl wf_79ca0106-d99/journal.jsonl wf_007968d3-b85/journal.jsonl wf_404f7e21-e85/journal.jsonl wf_f18cb9cf-cf3/journal.jsonl wf_4d554e0b-df4/journal.jsonl wf_60fc1374-906/journal.jsonl wf_a4f812d6-68e/journal.jsonl wf_93f45510-014/journal.jsonl wf_3cc77530-a4d/journal.jsonl | grep -c '"type":"result"' | sed 's/$/ returned/'
out: 57 returned

id:  P35
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **7 failed and 4 superseded, out of 128 launched: 8.6 % of the two rungs' and the plan-split's agent launches returned no report at all.**
fig: 128 launched
cwd: journal
cmd: cat wf_0c113a2f-027/journal.jsonl wf_26c81a18-dfe/journal.jsonl wf_d7f3bbbf-104/journal.jsonl wf_ff0ecd3b-d2e/journal.jsonl wf_f20790c2-73d/journal.jsonl wf_f2b04d7a-f1a/journal.jsonl wf_d5cfec2f-abf/journal.jsonl wf_5710bbc2-543/journal.jsonl wf_5fa11c84-2b1/journal.jsonl wf_1e0cf27c-f62/journal.jsonl wf_fdbcb66f-0d2/journal.jsonl wf_659b2d2f-75b/journal.jsonl wf_db0a2bc6-ada/journal.jsonl wf_f70c0232-045/journal.jsonl wf_578ffc29-14d/journal.jsonl wf_bdc3afc1-308/journal.jsonl wf_31440cb2-f48/journal.jsonl wf_461d1a48-f12/journal.jsonl wf_b536399f-3a2/journal.jsonl wf_79ca0106-d99/journal.jsonl wf_007968d3-b85/journal.jsonl wf_404f7e21-e85/journal.jsonl wf_f18cb9cf-cf3/journal.jsonl wf_4d554e0b-df4/journal.jsonl wf_60fc1374-906/journal.jsonl wf_a4f812d6-68e/journal.jsonl wf_93f45510-014/journal.jsonl wf_3cc77530-a4d/journal.jsonl wf_3e2c9b5d-deb/journal.jsonl wf_81e23ac0-d79/journal.jsonl wf_02ed3939-a80/journal.jsonl | grep -c '"type":"started"' | sed 's/$/ launched/'
out: 128 launched

id:  P36
doc: WORKFLOW-COST-EVIDENCE.md
ctx: 67 of 128 agent launches
fig: 67
cwd: journal
cmd: cat wf_f20790c2-73d/journal.jsonl wf_d5cfec2f-abf/journal.jsonl wf_1e0cf27c-f62/journal.jsonl wf_fdbcb66f-0d2/journal.jsonl wf_659b2d2f-75b/journal.jsonl wf_db0a2bc6-ada/journal.jsonl wf_461d1a48-f12/journal.jsonl wf_b536399f-3a2/journal.jsonl wf_79ca0106-d99/journal.jsonl wf_007968d3-b85/journal.jsonl wf_f18cb9cf-cf3/journal.jsonl wf_4d554e0b-df4/journal.jsonl wf_60fc1374-906/journal.jsonl wf_a4f812d6-68e/journal.jsonl wf_3cc77530-a4d/journal.jsonl wf_3e2c9b5d-deb/journal.jsonl wf_81e23ac0-d79/journal.jsonl wf_02ed3939-a80/journal.jsonl | grep -c '"type":"started"'
out: 67

id:  P37
doc: WORKFLOW-COST-EVIDENCE.md
ctx: the journal reads **4 launched, 0 returned, 4 failed**
fig: 4 launched, 0 returned, 4 failed
cwd: journal
cmd: echo "$(grep -c '"type":"started"' wf_4d554e0b-df4/journal.jsonl) launched, $(grep -c '"type":"result"' wf_4d554e0b-df4/journal.jsonl) returned, $(grep -c '"type":"failed"' wf_4d554e0b-df4/journal.jsonl) failed"
out: 4 launched, 0 returned, 4 failed

id:  P38
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **A1 `53 = 53`, EG2 `61 = 61`, all 31 lane runs `123 = 123`.**
fig: 53 = 53
cwd: runmeta
cmd: a=0;j=0;for d in wf_0c113a2f-027 wf_26c81a18-dfe wf_d7f3bbbf-104 wf_ff0ecd3b-d2e wf_f20790c2-73d wf_f2b04d7a-f1a wf_d5cfec2f-abf wf_5710bbc2-543 wf_5fa11c84-2b1 wf_1e0cf27c-f62 wf_fdbcb66f-0d2 wf_659b2d2f-75b wf_db0a2bc6-ada; do a=$((a+$(grep -o '"agentCount":[0-9]*' $d.json|cut -d: -f2)));j=$((j+$(grep -o '"key":"[^"]*"' ../subagents/workflows/$d/journal.jsonl|sort -u|wc -l)));done;if [ $a = $j ];then echo "$a = $j";else echo "MISMATCH agentCount=$a journalKeys=$j";fi
out: 53 = 53

id:  P39
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **A1 `53 = 53`, EG2 `61 = 61`, all 31 lane runs `123 = 123`.**
fig: 61 = 61
cwd: runmeta
cmd: a=0;j=0;for d in wf_f70c0232-045 wf_578ffc29-14d wf_bdc3afc1-308 wf_31440cb2-f48 wf_461d1a48-f12 wf_b536399f-3a2 wf_79ca0106-d99 wf_007968d3-b85 wf_404f7e21-e85 wf_f18cb9cf-cf3 wf_4d554e0b-df4 wf_60fc1374-906 wf_a4f812d6-68e wf_93f45510-014 wf_3cc77530-a4d; do a=$((a+$(grep -o '"agentCount":[0-9]*' $d.json|cut -d: -f2)));j=$((j+$(grep -o '"key":"[^"]*"' ../subagents/workflows/$d/journal.jsonl|sort -u|wc -l)));done;if [ $a = $j ];then echo "$a = $j";else echo "MISMATCH agentCount=$a journalKeys=$j";fi
out: 61 = 61

id:  P40
doc: WORKFLOW-COST-EVIDENCE.md
ctx: **A1 `53 = 53`, EG2 `61 = 61`, all 31 lane runs `123 = 123`.**
fig: 123 = 123
cwd: runmeta
cmd: a=0;j=0;for d in wf_0c113a2f-027 wf_26c81a18-dfe wf_d7f3bbbf-104 wf_ff0ecd3b-d2e wf_f20790c2-73d wf_f2b04d7a-f1a wf_d5cfec2f-abf wf_5710bbc2-543 wf_5fa11c84-2b1 wf_1e0cf27c-f62 wf_fdbcb66f-0d2 wf_659b2d2f-75b wf_db0a2bc6-ada wf_f70c0232-045 wf_578ffc29-14d wf_bdc3afc1-308 wf_31440cb2-f48 wf_461d1a48-f12 wf_b536399f-3a2 wf_79ca0106-d99 wf_007968d3-b85 wf_404f7e21-e85 wf_f18cb9cf-cf3 wf_4d554e0b-df4 wf_60fc1374-906 wf_a4f812d6-68e wf_93f45510-014 wf_3cc77530-a4d wf_3e2c9b5d-deb wf_81e23ac0-d79 wf_02ed3939-a80; do a=$((a+$(grep -o '"agentCount":[0-9]*' $d.json|cut -d: -f2)));j=$((j+$(grep -o '"key":"[^"]*"' ../subagents/workflows/$d/journal.jsonl|sort -u|wc -l)));done;if [ $a = $j ];then echo "$a = $j";else echo "MISMATCH agentCount=$a journalKeys=$j";fi
out: 123 = 123

id:  P41
doc: WORKFLOW-COST-EVIDENCE.md
ctx: `wf_9168b84b-530 17!=23 wf_d748568f-2c2 7!=9` — the two records disagree there, by 6 and by 2
fig: wf_9168b84b-530 17!=23 wf_d748568f-2c2 7!=9
cwd: runmeta
cmd: o="";for d in wf_9168b84b-530 wf_d748568f-2c2; do a=$(grep -o '"agentCount":[0-9]*' $d.json|cut -d: -f2);j=$(grep -o '"key":"[^"]*"' ../subagents/workflows/$d/journal.jsonl|sort -u|wc -l);o="$o$d $a!=$j ";done;echo "${o% }"
out: wf_9168b84b-530 17!=23 wf_d748568f-2c2 7!=9

id:  P42
doc: WORKFLOW-COST-EVIDENCE.md
ctx: | **total** | **31** | **27,055,990** | **11,486** |
fig: 27,055,990
cwd: runmeta
cmd: for d in wf_0c113a2f-027 wf_26c81a18-dfe wf_d7f3bbbf-104 wf_ff0ecd3b-d2e wf_f20790c2-73d wf_f2b04d7a-f1a wf_d5cfec2f-abf wf_5710bbc2-543 wf_5fa11c84-2b1 wf_1e0cf27c-f62 wf_fdbcb66f-0d2 wf_659b2d2f-75b wf_db0a2bc6-ada wf_f70c0232-045 wf_578ffc29-14d wf_bdc3afc1-308 wf_31440cb2-f48 wf_461d1a48-f12 wf_b536399f-3a2 wf_79ca0106-d99 wf_007968d3-b85 wf_404f7e21-e85 wf_f18cb9cf-cf3 wf_4d554e0b-df4 wf_60fc1374-906 wf_a4f812d6-68e wf_93f45510-014 wf_3cc77530-a4d wf_3e2c9b5d-deb wf_81e23ac0-d79 wf_02ed3939-a80; do grep -o '"totalTokens":[0-9]*' $d.json|cut -d: -f2; done|awk '{s+=$1} END{print s}'|sed ':a;s/\B[0-9]\{3\}\>/,&/;ta'
out: 27,055,990

id:  P43
doc: WORKFLOW-COST-PLAN.md
ctx: the **seven** 5-agent passes ran 0 h 56 m-2 h 42 m
fig: seven
cwd: runmeta
cmd: n=0;for d in wf_0c113a2f-027 wf_26c81a18-dfe wf_d7f3bbbf-104 wf_ff0ecd3b-d2e wf_f20790c2-73d wf_f2b04d7a-f1a wf_d5cfec2f-abf wf_5710bbc2-543 wf_5fa11c84-2b1 wf_1e0cf27c-f62 wf_fdbcb66f-0d2 wf_659b2d2f-75b wf_db0a2bc6-ada wf_f70c0232-045 wf_578ffc29-14d wf_bdc3afc1-308 wf_31440cb2-f48 wf_461d1a48-f12 wf_b536399f-3a2 wf_79ca0106-d99 wf_007968d3-b85 wf_404f7e21-e85 wf_f18cb9cf-cf3 wf_4d554e0b-df4 wf_60fc1374-906 wf_a4f812d6-68e wf_93f45510-014 wf_3cc77530-a4d wf_3e2c9b5d-deb wf_81e23ac0-d79 wf_02ed3939-a80; do grep -q '"agentCount":5,' $d.json && n=$((n+1)); done; case $n in 7) echo seven;; *) echo "$n";; esac
out: seven
```
