# Open questions for the owner

Difficulties, disputable calls and things I did not understand — written down as they arise so the
owner can read them later and weigh in, rather than finding them buried in a report after the
decision was already made.

> Russian version: [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md). **This file is the source of
> truth**; editing either side updates the other in the same commit. See [`ru/README.md`](ru/README.md).

**Convention.** Newest first. Each item states the situation, the options, and what it blocks. An
item is marked `RESOLVED` with the date and the owner's decision rather than deleted — the record of
*why* a call was made outlives the call. Perf and architecture forks are decided without asking, with
numbers; what lands here is VALUES, SCOPE, and anything genuinely unclear.

---

## 2026-08-09 — Rung 3d shipped two zones where the corpus specified a 90.8 KiB `RoundRecord` column. Reversible; the owner should know what it costs.

**A SCOPE call, disclosed rather than asked**, because the fork itself was a perf/architecture one
and those are mine to decide with numbers. What lands here is the one thing the numbers do not
settle: a specified public surface (`Profiler::rounds(back) -> &[RoundRecord]`) does not exist, and
that is the owner's to reverse if the lost quantity matters.

**What the corpus asked for.** `RoundRecord { frame, round, dispatched, begin, end }`, 24 B × 121
frames × `MAX_ROUNDS_PER_FRAME = 32` = **90.8 KiB** of the reservation, keeping *"dispatch shape
only: rounds per frame, wave width, round span"*.

**What shipped.** Two zone sites on the dispatcher — `__round` (Span) and `__round_width`
(Counter) — from which rounds per frame is `__round`'s `count`, round span is its
`total`/`min`/`max`, and wave width is `__round_width`'s. All three named quantities, per frame,
with distributions rather than a single row each.

**Why, in the order the reasons actually weigh.**

1. **The write path.** This is the decisive one and it is not an optimisation argument. The
   dispatcher does **not** hold `&mut EcsMaster` while a round is in flight — the `UnsafeEcsCell` it
   minted is shared with the workers. Writing a column from there needs either a second published
   pointer into the reservation, written by a thread the fold's `&mut` does not cover, or a
   per-schedule scratch buffer flushed after the run — profiling state owned by the scheduler. A
   lane push has neither problem and is the mechanism rung 3c already blessed for `SystemSpan`.
2. **No truncation.** `MAX_ROUNDS_PER_FRAME = 32` would have counted the 33rd round of a
   deep-dependency schedule as *dropped* rather than measured, with its own drop class. Two zones
   truncate at nothing.
3. **90.8 KiB and one drop class not spent.**

**What is lost, and it is a real thing.** The **correlation** between one round's width and that
same round's span — "was the widest round also the longest?" Per-frame aggregates cannot answer it.
Nothing in the profiling corpus asks that question today, which is why the call went this way.

**To reverse:** restore `RoundRecord` with a scratch-and-flush path in `ExecutorScratch`, accept
the 32-round truncation and its counter, and keep the two zones or drop them. Say the word.

*(Two smaller departures ride along and need no decision: `Interval.sys` is `Interval.zone`, because
`sys_of` is gone and zone → system resolves at report time; and `G8` has no SKIP clause, because
`ThreadPoolBuilder::num_threads(2)` never consults the machine, so fewer than two workers would be a
threadpool defect rather than an environment to excuse. Both are argued in
`docs/diagnostics/profiling/05-LADDER-GATES.md`.)*

---

## 2026-08-09 — The dev residency budget has 1.3 MiB of headroom, not the 9 MiB the corpus's table implies. `J1`'s `Z` contradiction, now with a number.

**Recorded, not asked** — it is `J1`'s to settle and was already logged at rung 3a. What is new is a
measurement, and the measurement changes how urgent it looks.

`profiling_residency` now prints its configuration. On this box, armed, analysis ON:
**total 14 667 776 B (reservation 14 614 528, statics 53 248)** against a 16 MiB dev budget.

The corpus's own dev rows are **6.67 MiB** (analysis off) and **7.05 MiB** (on) — roughly half. The
gap is *not* the new interval ring, which is 262 144 B of it. It is `D8`'s `Z = 1024` against the
shipped `ENGINE_ZONE_SLOTS = 4096`: the five columns come to 21 B × 4096 × 121 = 10 407 936 B where
the table budgets 21 B × 1024 × 121 = 2 601 984 B.

So the sizing table and the shipped constant have disagreed by a factor of four since rung 2, and
the consequence is that the dev budget is at **87 % utilisation** rather than the ~44 % the table
suggests. `J1` owns the fix — either `ENGINE_ZONE_SLOTS` comes down to 1024, or every sizing row
in `profiling/01-EMISSION-STORAGE.md` is recomputed at 4096 and the retail budget re-derived with
it. Nothing is blocked today; the headroom is just much thinner than it reads.

---

## 2026-08-08 — The whole `92xx` code block described the wrong eighteen conditions. Repaired; recorded because of HOW it hid.

**Recorded, not asked.** The repair direction was not a judgement call and it is already committed.
What belongs here is the mechanism, because it is a hole in this project's own gate design and it
will recur.

**What was wrong.** L2 reserved eighteen `92xx` rows for the profiler — correctly, and for a good
reason — and then wrote eighteen plausible summaries composed from the code *numbers*. Sixteen of
them name conditions the profiling corpus does not have. `W9207` is the sharpest case: the corpus
pins it as **invariant TSC absent** in five documents, and logging's own `W0101` was **struck in its
favour**, so the invented summary ("a GPU query pool returned fewer results than were issued") left
the engine's only invariant-TSC code naming something else while the condition it was struck for had
no code at all. `9213` is `E9213` in the corpus (six mentions, four files) and was seeded `W9213`.

**Why nothing caught it, and this is the part worth keeping.** The registry has seven checks and all
seven were green. A `Pending` row owes **no doc page** (check 2 is `Live`-only) and **no emitter**
(check 3a is `Live`-only) — both narrowings are correct on their own terms, and L2 argued for them
in writing: *"otherwise L2 would owe eighteen pages for codes with no emitters, which is doc-rot
manufactured by a gate."* That reasoning is still right. But the two narrowings together mean a
`Pending` row's summary is compared against **nothing**, by construction — and the registry's own
check-4 message already names this defect class: *"inventing a summary here is how three rows of
this registry came to disagree with the messages the engine prints."* The registry documented the
failure mode, then shipped it at six times the scale, in the one status where no check could look.

**The generalisation, which is not repaired and is the reason this is written down.** A `Pending`
row is a **promise with no gate on its content**. Today the only thing that will ever check a `92xx`
summary is the rung that flips it `Live`, i.e. between one and fourteen rungs from now. Rung 2 flips
seven of them and reads the other eleven; the remaining eleven are still un-compared, and if a later
rung flips one without re-reading the corpus, the invented sentence ships.

**Two ways to close it, neither taken here** (both are bigger than rung 2 and one is a VALUES call):

* **(a) A check that pins every row's summary against the corpus.** Mechanically: for each `92xx`
  row, require its condition text to appear in `docs/diagnostics/profiling/05-LADDER-GATES.md`'s
  §Integration list. Cheap, and it would have caught all sixteen. Cost: it couples the registry's
  wording to a document's wording, which is a second statement of one fact — the exact shape this
  corpus deletes elsewhere.
* **(b) Do not seed summaries at all** — a `Pending` row carries the rung and an empty summary, and
  the summary is written when the emitter is. Structurally correct, and it is what
  `FORWARD_DECLARED` already does for the seventeen logging codes. Cost: `explain()` returns nothing
  useful for a `Pending` code, which is a real loss for anyone reading a corpus document today.

**Blocks nothing.** Rung 2 landed with the summaries repaired and seven rows flipped `Live` with
their pages. The other eleven are correct as of this commit and un-gated after it.

---

## 2026-08-08 — L5 shipped, and it weakened a specified latency bound. No decision needed unless you want it back.

**Recorded, not asked.** This is an architecture fork I decided with the tree in front of me; it is
here because it makes a *specified* number worse, and a number that quietly got worse is the thing
this file exists to prevent.

**The situation.** The logging corpus says `log_drain_system` runs "in `Last`", and states the
in-frame latency bound as **one frame** under `Scheduled` / "sink park + one frame" under `Thread`.
This engine has no `Last`. `CoreSchedule` is a **closed set of two** (`Main`, `Fixed`) and its own
doc gives the intended answer — *"finer-grained structure WITHIN a schedule is what Phase-15 sets
are for."*

**What shipped.** The drain runs in `Main`, `in_set(LogSet)`, and `LogPlugin::build` interns the set
so a host's `.before(LogSet)` resolves regardless of plugin add-order.

**What it costs.** With no ordering edge the scheduler may place the drain anywhere in the frame, so
a record emitted *after* it appears in the next frame's ring. **Each specified bound gains one
frame** for a host that does not add the edge. The drain has no data conflict with anything, so
nothing forces it late on its own.

**Three ways to get the frame back**, none taken, because all three are bigger than L5:

* **(a) Do nothing** — document the edge and let each host add `.before(LogSet)`. What shipped.
  Costs one line per host; costs a frame if a host forgets, silently.
* **(b) Add a third `CoreSchedule` variant.** Honest, matches the corpus verbatim, and touches the
  frame driver, the routing methods and every `add_systems_in` call site. An engine change to serve
  one subsystem.
* **(c) Give the engine a standing `EngineSet::Last`** that the app plugins order everything before.
  Cheaper than (b) and useful beyond logging, but it is a scheduling convention the engine does not
  have yet, so introducing it from the logging seam is the tail wagging the dog.

**Blocks nothing.** L16's `G15` is where the bound is actually measured, and it must be measured
against whichever of these is true then.

---

## RESOLVED 2026-08-08 — the owner chose (b): raise the budget. Q1 stands, no code changes.

**Decision:** accept the footprint and raise the gate's shipping budget from **1024 KiB to
1280 KiB**. `LANE_COUNT` stays 80 in every profile; `REGION_CAPACITY` stays 128; D15 keeps
committing the sample slab at first `arm`. **No source change of any kind.**

**Why it is the right call, in the owner's own framing.** The alarming number measured a
*reservation*, not what a machine holds. The row is now printed in three columns instead of one
that conflated them:

| Column | Shipping profiler | Meaning |
|---|---|---|
| declared / reserved | **1 208.2 KiB** | address space; free in any practical sense on 64-bit |
| committed at `arm` | **≈ 1 142 KiB** | the reservation, taken when diagnostics are turned **on** |
| resident, flag off | **≈ 0** | nothing armed, no lane claimed, no `.bss` page touched |

The owner asked whether this costs runtime or RAM. It costs neither in the shipped default: with
the flags off a site pays **one `.bss` byte load and one predicted branch**, and above the compile
ceiling it pays nothing at all — the site and its argument expressions are deleted. That per-site
floor is the only thing a runtime flag cannot remove, and no budget choice touches it.

**Consequences applied:** `G23a` and `G23b` are **unblocked** — they assert against 1280 KiB and
have a reachable green state again. Options (d) `REGION_CAPACITY = 64` and (a) per-lane lazy commit
are **retained below as levers**, not as work: pull one only if a measurement later says the
*committed* figure is too high.

*(Original entry follows, unedited — the record of why the call was made outlives the call.)*

---

## 2026-08-08 — ⚠️ Q1 raised the shipping diagnostics footprint by 1.07 MiB, and the profiler's "≤ 1 MiB retail" headline is now FALSE

**This supersedes the ≈ 2.08 MiB figure in the round-3 entry below.** The correct joint figure is
**≈ 3.15 MiB**.

**What happened.** At rung D0 I resolved architect blocker **Q1** by deleting `LANE_COUNT`'s build
profile axis — it was 32 in the shipping profiles while the quantity it indexes,
`boyko_threadpool::MAX_WORKERS = 64`, is unconditional, so 32 was unsound and below the topology's
own floor of 66. The resolution (80 in every profile, `455c074`) was correct and I stand behind it.
**What I did not do at the time was propagate its cost**, and four cells across the two plans were
sized by that constant:

| Cell | Was (32 lanes) | Is (80 lanes) | Kind |
|---|---|---|---|
| profiling `LANES` (`.bss`) | 8 KiB | 20 KiB | reserved |
| profiling **sample slab** | 192 KiB | **480 KiB** | **committed at first `arm`** |
| logging `LOG_LANES` (`.bss`) | 512 KiB | 1.25 MiB | reserved |
| logging `SAMPLE_CTR` (`.bss`) | 16 KiB | 40 KiB | reserved |

Profiler half **908.2 → 1 208.2 KiB**; logger half **1 220.26 → 2 012.26 KiB**; joint
**2.08 → 3.15 MiB**. The `dev` figures did not move at all, because `dev` was already at 80 —
**which is exactly why nothing caught this: every check that looked at one row looked at the row
that was still right.**

**Two things follow that are not "the number got bigger".**

1. **The profiler is now over its OWN budget**, not just the joint one: 1.18 MiB against a stated
   ≤ 1 MiB. Gates **G23a and G23b assert that bound**, so both now fail at the baseline — they have
   no reachable green state until this is answered.
2. **Only 288 KiB of the +1.07 MiB is committed memory.** The rest is `.bss` reserved extent whose
   resident cost is per *touched* lane, which is the property that made 80-everywhere affordable in
   the first place. The committed part is entirely the profiler's sample slab, which **D15** commits
   for all `LANE_COUNT` lanes at first arm.

### The call

> **RECOMMENDATION, final: (b) — raise the budget and restate the row honestly.** The owner asked
> the right question: *is 3.15 MiB actually a lot?* It is not, and more importantly **the figure
> measures the wrong thing.** It is a **reservation**. What a machine actually holds is:
>
> | Configuration | Resident |
> |---|---|
> | shipped title, diagnostics flag OFF (the default) | **~0** — every table is demand-zero `.bss` that nothing touches, nothing is armed, nothing is committed |
> | shipped title, diagnostics ON | the profiler's sample slab (480 KiB, committed at `arm`) plus the logger's *touched* lanes and staging — order of **1 MiB**, not 3.15 |
>
> Address space on 64-bit is free in any practical sense, and 1 MiB resident against a single
> 2048² RGBA8 texture at 16 MiB is not a trade worth buying with code. So the cheapest fix of all
> is the one that changes no code and no constant: **raise the gate's shipping budget** (1024 →
> 1280 KiB covers 1208.2 with headroom) and print the row in three columns — reserved,
> committed-when-armed, resident-when-off — instead of one number that conflates them.
>
> That also unblocks G23a/G23b immediately, which (d) and (a) do only after a code change.
> **(d) and (a) below are kept as the levers to pull if a measurement later says the committed
> figure is too high**, not as things to do now.

- **(d) — kept as the cheap lever, no longer the recommendation.** Set `REGION_CAPACITY = 64` in the shipping
  profiles instead of 128. The slab is `LANE_COUNT × 2 regions × REGION_CAPACITY × 24 B`, so
  `80 × 2 × 64 × 24` = **240 KiB** instead of 480, and the row lands at
  `66 + 240 + 636 + 6.8 + 11.4 + 8` = **968.2 KiB — under the 1024 KiB budget**, with Q1 intact.

  **Cost:** a region holds 64 samples instead of 128 before the fold must drain it, so a shipping
  build drops samples earlier under a burst. In shipping the tier is `Always` only, so the sample
  stream there is already an order of magnitude thinner than in `dev`.

  **What it does not cost:** not one line of code, not one branch on the hot path, not one gate
  re-specified. `REGION_CAPACITY` is *already* a per-profile constant — unlike `LANE_COUNT`, whose
  axis Q1 deleted as unsound — so this is the knob doing the job it exists for.

  *I proposed (a) first because I was looking at where the bytes are rather than at what is
  cheapest to remove. The order is the other way round: the constant that exists for this, then
  code.*

- **(a) — now the fallback, if measurement later shows 64 samples per region is too shallow.**
  Commit sample regions **per lane on first use** instead of all 80 at
  arm.

  **Performance is not the problem with (a); the gate is.** The hot path already loads
  `buf: AtomicPtr<Sample>` on every sample, so a null test is one `test`+`jz`, predicted
  not-taken after a lane's first sample — call it zero. The commit itself is one syscall per lane
  on a `#[cold]` path, ten of them over a process. Two real costs, though: the syscall lands
  **inside a frame** (a worker's first zone, during the first frames, where frame times are
  already noisy — mitigable by committing at `arm()` for lanes that already exist, since the pool
  is built before `arm`), and it adds unsafe surface to the profiler's hottest path. The one that
  matters most: **G23a/G23b stop being able to assert a single armed total.** The figure becomes
  warm-up-dependent, and a crisp gate becomes a "after N frames" gate. A shipping title on an 8-core box claims roughly `workers + dispatcher + host ≈ 10` lanes, so
  ≈ 60 KiB of slab instead of 480 — the profiler is back under 1 MiB **with Q1 intact and no
  constant changed**. It edits D15 ("committed once at first arm, never freed"), which is a
  shipped-behaviour decision, which is why it is yours and not mine.
- **(b)** Accept 3.15 MiB reserved / ~1.2 MiB committed and restate the headline.
- **(c)** Cut a table instead: logging's `LOG_LANES` (1.25 MiB), `SINK_OUT` (256 KiB), or the
  profiler's dynamic-zone arenas (96 KiB per the profiling plan, 40 KiB per `SEAM.md` — that
  divergence is still open and is the profiling plan's to close).

**Blocks:** profiling rungs 2 and 10 (G23a/G23b). Does **not** block logging L0 or anything on the
substrate ladder.

**The lesson, recorded because it is the fourth time this shape has appeared in this campaign:** a
total that is a perfect sum of its printed operands proves nothing about whether the *operands* are
current. 2.08 MiB was correct arithmetic over two halves the substrate had already invalidated. The
check that would have caught it is not "does the total add up" but "has anything this total depends
on been decided since it was written".

---

## 2026-08-06 — ⚠️ MEASURED: synchronization validation is not live, so the `-ValidationOn` leg proves nothing about barriers

**A genuine missing barrier changed no pixel and emitted no message.** Executed while resolving
piece 2's first step, which existed precisely to find this out.

The probe: delete the ONLY declared read of a resource with exactly one reader — the HZB pyramid's
mip `d-1` read — while the dispatch that reads it stays. Pass 0 writes mip 5, pass 1 reads mip 5, no
derived dependency.

| | messages | `SYNC-HAZARD-*` | golden |
|---|---|---|---|
| baseline (×2, same build) | 19 | — | byte-identical |
| **real missing barrier** | **19** | **none** | **byte-identical** |

The feature bit IS requested in `boyko_rhi_vulkan/src/device.rs`, but the instance chain degrades
**silently** when `VK_EXT_validation_features` is absent, and the whole 19-message baseline is
`vkCreate*`-time — nothing in it was ever produced by a recorded frame.

**Why this is here rather than merely recorded.** It is not a piece-2 fact. It says that the
engine's validation leg — the instrument this campaign has been leaning on since the P1-2
`-ValidationOn` repair — covers object, descriptor and format legality and **nothing about
synchronization**. Every "validation clean" claim in the campaign's commit messages is true and
narrower than it reads.

**Options.** (a) Leave it, and gate barrier correctness structurally (pin the derived barrier stream
by FIELDS, which is what piece 2's G4 now does). (b) Find out whether `VK_EXT_validation_features` is
genuinely absent on this device or merely not reaching the layer, and fix it if it is the latter —
this is a ~1-hour investigation and would restore a general-purpose instrument. (c) Both.

**My recommendation is (c), with (a) first**, because (a) is already specified and blocks nothing,
while (b) is worth doing before piece 3 — that piece adds the first pyramid READER, and a
read-after-write across two passes is exactly the hazard class the layer would catch and the golden
cannot.

⚠️ **A methodological note worth as much as the finding.** The FIRST probe was inconclusive by
construction: it deleted one of SIX declared readers of the same image, so siblings still carried
both the transition and the dependency and nothing was tested. Its negative result would have been
recorded as "the extension is absent on this device" — a true statement reached by an invalid route.
When probing for a missing dependency, count the OTHER declared accesses to that resource first.

---

## 2026-08-05 — CI's release leg is red, and two of the classes are STRICTNESS calls

Found while preparing the P1-5a baseline, which needs a leg that passes. Running CI's own command —
`cargo test --workspace --all-targets --release --exclude boyko_demo --exclude bench-bevy-vs-boyko`
([ci.yml:62](../.github/workflows/ci.yml), `:103`) — **fails on six targets**. Two more appear on a
second run, which is itself the diagnosis: those are flaky, not release-specific.

Six were mechanical and are **fixed**: five `#[should_panic]` tests over `debug_assert!` guards
missing `#[cfg(debug_assertions)]` (boyko_math, boyko_ecs, boyko_render ×3, plus two in
boyko_rhi_vulkan), and one missing `VB_PINS` entry that was mine — `vb_mesh_hzb` from VG R3 P1-2.

Two classes are left, and both are decisions about how strict a gate should be rather than
architecture forks, so they are here rather than taken.

### 1. `boyko_shaderdsl --test eval_byte_identity` — three failures, on the NaN SIGN BIT alone

`NaN (0xffc00000)` vs `NaN (0x7fc00000)`. Both quiet NaNs; the values are identical (a NaN is not
even equal to itself); only the sign differs.

This is the same family as what gate G3 measured on the depth pyramid the same day: **the sign of a
zero and the sign of a NaN are exactly the two bits no `<` in a program can observe**, which is why
hardware and optimisers are free to move them — G3 caught a driver fusing a compare-and-select into
a hardware min whose ±0 tie-break differs. Expecting either bit to be stable between `-O0` and `-O3`
is not well founded.

**Options.** (a) Compare NaN as "both are NaN" rather than by bits. (b) Canonicalise the sign before
comparing. (c) Leave it, and accept that the eDSL's release leg is not a gate.

**My recommendation is (a).** The contract the eDSL exists to enforce is about VALUES, and the sign
of a NaN is not a value. It costs nothing on the finite domain, which is the whole domain that
matters, and it stops a real gate from being permanently red — which is worse than a slightly
narrower one, because a red gate nobody can fix is a gate nobody reads.

### 2. Two global-state tests that are flaky under parallel execution

`boyko-scene bundles_s6::interner_is_off_the_per_frame_path` reads the process-global
`identity::interner_len()`. `boyko-ui zero_alloc::unchanged_frame_layout_pair_allocates_zero_over_baseline`
reads a global allocation counter — and reported a delta of **minus one**, an improvement its
`assert_eq!` cannot express while its own message says "no more than".

Both pass alone, pass under `--test-threads=1`, and pass in debug. They fail only in release with
default parallelism.

**Options.** (a) A serial guard in each test file. (b) `--test-threads=1` for those binaries in CI.
(c) Make the UI assertion `<=`, matching its own message, and serialise only the scene one.

**My recommendation is (a) plus the `<=` repair**, because the harness flag would slow every test in
those crates to fix two, and because an equality assertion that fails on an improvement will fail
again the next time somebody improves it.

---

## 2026-08-05 — SCOPE: the pyramid needs a core framegraph change (per-subresource sync state)

**Decided and under way, not blocking — recorded because it grows piece 1 beyond what its plan
scoped.** Step P1-5 (declare the HZB build passes) cannot be written against the framegraph as it
stands, and the framegraph says so itself.

**The wall.** `framegraph/graph.rs:360-445` carries `INVARIANT HZB-SUBRESOURCE-UNIFORM`: every
access to one `ResId` must declare the same `(base_mip, mip_count, base_layer, layer_count)`,
because `FrameGraph::state` is a `Vec<ResSync>` indexed by `ResId` alone and `transition` never
receives the span. The HZB build needs, on ONE image in ONE pass, a read of mip `6p-1` and a write
of mips `[6p, 6p+n)`.

That comment was written in advance, names this exact pass, and prescribes the answer:

> "PER-SUBRESOURCE TRACKING IS THE CORRECT LONG-TERM ANSWER, and this assert is its TRIGGER … the
> HZB build writes mip k while reading mip k-1. When that pass is authored, it trips this assert.
> That is the INTENDED way to discover the work … The response is to build per-subresource
> tracking, never to relax the condition until it goes quiet."

**It is not merely a debug assert.** In release the assert is compiled out and the derivation is
genuinely wrong, traced at 512×512: pass 0 first-touches mips [0,6) so only those leave `UNDEFINED`;
pass 1 then writes mips [6,10) with the state claiming GENERAL for the whole ResId, so the derived
barrier has `old_layout == new_layout` and mips 6..9 are **never transitioned** while the dispatch
writes them through storage descriptors declared `GENERAL`. Every extent with
`prev_pow2(max(W,H)) >= 64` reaches it.

**The workaround I rejected.** Three ResIds aliasing one `VkImage` over disjoint mip spans is
uniform by construction and needs no framegraph change. I turned it down for two reasons. It is
literally what the invariant's own text forbids ("not by making the declarations agree by hand"),
and it is a dead end one piece later: piece 3's cull selects a pyramid LEVEL per instance, and a
per-pass ResId cannot be named by a dynamic level. Taking it would be the interim design deferred to
later that this project has ruled out.

**What it costs.** A new step P1-5a ahead of P1-5, touching the core state machine every render path
compiles through. The byte-identity argument is strong — `SubRange::color_mips` is called by nothing
today and every existing `image_access` site passes `base_mip: 0, mip_count: 1`, so a per-mip
machine should fold to today's behaviour barrier-for-barrier — but "should" is why it gets its own
gate rather than a golden pin, which cannot see a redundant or a missing barrier.

**Nothing is blocked on an answer.** Architecture forks are mine to decide; this is here because the
SCOPE grew, and the owner may prefer piece 1 to stop at "allocated and compiled" and hand the
framegraph work to its own campaign. Say so and I will split it.

---

## 2026-08-04 — ⚠️ `golden.ps1 -ValidationOn` never enabled the validation layer on ANY `boyko-app` pin

**Found while gating VG R3 P1-2, and it is the vacuum-green shape again.** The switch is the
engine's validation-audit instrument; the campaign records a "Validation-ON audit — COMPLETE"
milestone that ran through it. On the 22 of 25 pins that boot through `boyko_app`, it could not
fail.

**Mechanism.** The backend gates the layer on a conjunction —
`config.enable_validation && BOYKO_DISABLE_VALIDATION unset` (`boyko_rhi_vulkan/src/device.rs:2350`)
— and `boyko_app`'s runner hardcoded `enable_validation: false`. `-ValidationOn` only ever
*stripped the env var*, i.e. satisfied the second conjunct while the first stayed false. The layer
was never requested, no messenger existed, and the scan for `[vk-validation]` lines therefore
reported **"clean (0 messages)"** unconditionally.

The runner's own doc said so, in a passage read as a design note rather than as a gate defect:
"The shipped runner does NOT request the validation layer… a debug validation knob arrives with a
later rung."

**Measured, not inferred.** I built a `512×512` image with `mip_levels: 12` (the legal max is 10).
`vkCreateImage` returned SUCCESS and the audit reported clean. With the fix, the same corruption
draws the exact message: `vkCreateImage(): pCreateInfo->mipLevels (12) must be less than or equal
to 10`.

**Fixed here**, because P1-2's load-bearing gate depends on it: `BOYKO_ENABLE_VALIDATION` opts the
runner in (absent ⇒ boot byte-identical to before), and `-ValidationOn` now sets it alongside the
strip.

**⚠️ WHAT IT REVEALED, AND THE OPEN QUESTION.** With the layer actually live, the `vb_mesh` pin
emits **19 validation messages** — a baseline nobody has seen:

| count | message |
|---|---|
| 9 | `vkCreateComputePipelines()`: compute shader uses descriptor `[Set 1, …]` |
| 6 | `vkCreateGraphicsPipelines()`: vertex attribute at location 1/2 not consumed by vertex shader |
| 1 | **`vkDestroyDevice(): VkDevice has 13 leaked objects that have not been destroyed`** |
| 1 | `vkCreateShaderModule()`: SPIR-V capability `Geometry` declared without the feature |
| 1 | `vkCreateShaderModule()`: SPIR-V capability `DemoteToHelperInvocation` declared without the feature |
| 1 | duplicate-limit warning |

The pyramid is not implicated: armed and unarmed logs are **byte-identical** after handle
normalization, so P1-2 contributes zero. But the two shader-capability messages and the 13 leaked
objects are real, they are on the flagship VB pin, and one of them is a resource leak.

**The question is SCOPE, not method.** Options: (a) I audit and fix the 19 now, before continuing
the pyramid — it is a leak and two feature-declaration bugs on the main path; (b) I finish piece 1
and take the validation baseline as its own campaign afterwards; (c) I fix only the leak now and
defer the rest. I lean (b): the 19 predate this work by a long way, the pyramid is proven clean
against them, and interleaving an unbounded audit into a decomposition that was created
*specifically* to keep scope local would undo the decomposition. But it is your call — this is
scope, and the leak is the kind of thing that gets worse while it waits.

---

## RESOLVED 2026-08-03 — the HZB feature design does not converge in one piece: decomposed

**Situation, measured over three review rounds rather than felt.**

| round | prior items closed | new blockers | new majors |
|---|---|---|---|
| 1 | — | 8 | — |
| 2 | 10 YES / 6 PARTIAL / 0 NO | 3 | ~12 |
| 3 | 31 YES / 12 PARTIAL / 2 NO | 6 | 11 |

Each round genuinely resolves most of what the last one raised, and each raises about as much
again. After round 3 **every substantive step carries a blocker** (S4, S6a, S6b, S7, S9); the four
clean steps are gates and records that depend on the blocked ones. So unlike the foundation case
there is no independent subset to land.

The new blockers have also changed CHARACTER, which is the useful signal. They are no longer "the
algorithm is wrong" — they are collisions with shipped invariants: a fourth route by which the
design disarms rung R2d-6 (doubling the survivor list breaks the very const-assert added in R2d-4
to prevent an out-of-bounds device read); an `+INFINITY` fixture vertex reaching a second, unfenced
host consumer on the shipped VB path; a capability that is a per-frame ECS fact gating objects
minted at boot with no seam named between them.

**What I read from that.** The feature is simply larger than one design pass can hold. The
foundation converged in a single round each because S1/S2/S3 were small, independent and
self-contained — not because the process was better there.

**Proposal.** Decompose the feature the way the foundation already was, and give each piece its own
design + review round:

1. **The pyramid alone** — allocate, build, gate against the S3 host oracle. No cull integration of
   any kind. It is self-contained, its oracle already exists, and its own blockers are local.
2. **The capability and the raster split alone**, inert — the second scope drawing nothing, proven
   byte-identical on the pins.
3. **The cull integration**, once 1 and 2 are shipped and the collisions above are concrete rather
   than predicted.
4. **The arming**, with the drawn-set gate.

**The cost, stated.** Four design rounds instead of one, and the feature lands later. **The
alternative cost**, also stated: a fourth whole-feature round that on this evidence resolves ~30
items and raises ~6 more.

**OWNER'S DECISION: decompose.** The four pieces above each get their own design + review round
and land independently, starting with the pyramid alone. Recorded so a later reader sees the four
rounds were a deliberate structure, not a design that kept failing.

**Blocks.** Nothing.

---

## RESOLVED 2026-08-02 — the depth-complexity fixture is Khronos Sponza (delegated to me)

**Situation.** Decision (b) above commits to a separate fixture with real occlusion. What it should
contain was not decided, and it is an asset question I raised rather than settled on my own — the owner then delegated it back to me.

The VG corpus is seven fetched Khronos glTF sample assets arranged on a 5x3x3 grid — chosen for
triangle density, and it has almost no occlusion by construction. A depth-complexity fixture wants
the opposite: large occluders with substantial geometry behind them. The classic choices are Sponza
(an interior with a colonnade that occludes heavily) or Bistro; neither is in the manifest today.

**OWNER'S DECISION: delegated to me.** Chosen: **Sponza, from the same Khronos glTF-Sample-Assets
family the density corpus already draws on**, fetched into `assets/vg_occlusion/` under its own
manifest, gitignored and content-pinned exactly as `assets/vg_corpus/` is.

**Why Sponza and not Bistro or Intel Sponza.**

- *Same source family as the existing corpus*, so the fetch script's shape, the licence posture and
  the gitignore precedent all transfer. No new infrastructure and no new licensing question — the
  three things that would otherwise make this a multi-session detour.
- *Same loader path*: glTF/`.glb` through `GlbMeshLoader`, already exercised by every corpus asset.
- *It has the right OCCLUSION STRUCTURE*, which is the whole point: a colonnade plus an upper
  gallery, so a camera at floor level down the nave has its far half hidden behind columns. That is
  exactly what the density corpus lacks by construction.
- *Size*. This session already hit zero free space at a 73 GB `target/`. Bistro is ~2.4 GB, and its
  glTF conversions vary in provenance — which matters more here than usual, because this repo pins
  by content hash and a pin on an artifact nobody can re-derive is not a pin.
- *Comparability*: Sponza is the published occlusion/GI benchmark, so a number measured on it means
  something to a reader outside this repository.

**The risk I am taking, stated rather than discovered later.** ONE scene is the same
vacuous-selection exposure the corpus notes warn about — a single framing can be chosen to flatter.
Mitigation is the corpus's own: several committed camera paths spanning degrees of occlusion (down
the nave = heavy; from the gallery = moderate; outside looking in = little), with the WEAKEST
binding, exactly as `orbit_mid` binds the density corpus. A win claimed off the heavy framing alone
would be the defect, not the fixture.

**Still to do when it is built.** The `source_url` / `archive_sha256` / per-file `glb_sha256` pins
are filled from the first verified fetch, the way `CORPUS.toml`'s were — not written from a guess.

**Blocks.** Any occlusion perf claim. Blocks no implementation work, and is not on the HZB critical
path.

---

## RESOLVED 2026-08-02 — Occlusion perf claim: option (b), a separate depth-complexity fixture

**Situation.** The VG corpus is a triangle-density instrument, deliberately recomposed at rung R0b′
to measure density rather than occlusion. Measured ceiling on it: **1 of 44 drawn instances at
`orbit_mid`** (the binding framing) and 11 of 31 at `approach_close`. A min-reduced HZB can only
reject instances that win zero pixels, so those are hard upper bounds — and they bound more than
occlusion, since an instance also wins zero pixels when it is sub-pixel.

So the HZB and two-pass occlusion work now in flight can be built correctly and gated for
correctness, but **no occlusion speed-up can be demonstrated on any content in this tree**.

**Options.** (a) Ship it correctness-gated with no perf claim, as rung R2d shipped structural —
honest, and leaves the claim unmade until content exists. (b) Add a scene with real depth complexity
(an interior, a street) as a *separate* perf fixture, kept out of the density corpus so the two
instruments cannot contaminate each other. (c) Accept the claim will be made by whatever project is
built on the engine, not by this repository.

**OWNER'S DECISION: (b).** Build a scene with real depth complexity — an interior or a street — as
a SEPARATE perf fixture, deliberately kept out of the density corpus so the two instruments cannot
contaminate each other. Until it exists, the HZB rung ships correctness-gated with no speed claim.

**Follow-on, and it needs an asset decision** — recorded as its own item below rather than assumed.

---

## RESOLVED 2026-08-02 — K2: option (c), stays deferred

**Situation.** The virtual-geometry campaign's kill criterion K2 requires a Nanite reference table.
It has never been produced (UE is not installed; I cannot install it — the flow requires accepting an
EULA and creating an account, which I must not do). K2's own text says an unproducible baseline
*forces a scope restatement*: an absolute target instead of a relative one.

**Options.** (a) Owner installs UE and produces the table. (b) Restate the goal against an absolute
target (frame time at a stated triangle count and error bound) and record K2 as taken by its own
escape hatch. (c) Leave deferred and keep the goal formally unfalsifiable.

**OWNER'S DECISION: (c).** Stays deferred; the goal remains formally unfalsifiable, knowingly. No
rung is blocked by it. Recorded rather than quietly dropped, so a future reader does not mistake the
campaign's silence on K2 for K2 having been satisfied.

---

## RESOLVED 2026-08-02 — Cross-frame occlusion soundness

**The worry was mine and it was misframed.** A previous-frame pyramid is indeed not conservative —
but only for a ONE-pass cull. In two-pass it is never the last word: soundness lives entirely in the
late pass, which tests against a pyramid built from THIS frame's depth. The early pass is an
*unverified heuristic* whose only job is to fill the depth buffer with a good occluder set; its
mistakes cost late-pass work and never cost geometry. The theorem quantifies over every possible
early-pass output, so nothing about the early pass has to be proven at all.

Confirmed against practice rather than assumed: UE5 Nanite, Assassin's Creed Unity (SIGGRAPH 2015),
Granite, Bevy 0.16 and Unity 6's GPU Resident Drawer all have this same structure.

Full statement and proof: [VG-R3-HZB-PLAN.md](VG-R3-HZB-PLAN.md) §1. **No owner decision needed.**

---

## RESOLVED 2026-08-02 — HZB: option (a), then REVISED TO (b) — foundation first

**Situation.** With soundness settled, the implementation design was reviewed and returned REJECTED
by both reviewers. The blockers are real, not stylistic — among them: the design revives
frustum-culled instances and thereby deletes rung R2d-6's arming; unknown mesh bounds produce a
PERMANENT false reject for any streaming-in mesh, surviving both passes; the one gate that can see a
false reject cannot be built as specified, because `vb_depth` carries no `TRANSFER_SRC` and the
readback path it depends on is listed UNVERIFIED while being load-bearing; and the pyramid build,
being compute, must split the VB raster's single dynamic-rendering scope in two, which the plan does
not address — a naive second scope would `LOAD_OP_CLEAR` away the early pass.

Full list: [VG-R3-HZB-PLAN.md](VG-R3-HZB-PLAN.md) §5.

**Options.** (a) One more design revision round against the 8 blockers, then implement — the same
loop that took rung R2d from 8 blockers to shipped. (b) Implement the uncontroversial foundation
first (S1 the RHI `TextureView`, S2 the framegraph guard, S3 the host oracle) while the cull design
is revised — these three are independently useful and none depends on the disputed parts.
(c) Park the rung.

**OWNER'S DECISION: (a).** One full revision round against all 8 blockers, then implement in step
order. My own recommendation had been (b) — land the uncontroversial foundation first — and it was
not taken; (a) is the same loop that carried rung R2d from 8 blockers to shipped, and it keeps the
step order intact rather than interleaving foundation work with a design still in motion.

**REVISED TO (b) the same day, by the owner, after the revision round returned.** The round closed
every prior blocker (10 YES / 6 PARTIAL / **0 NO**) and produced 3 NEW blockers plus a dozen majors
— and every one of them lands in the FEATURE: candidate routing, a capability predicate missing its
`mesh_leg` conjunct, a boot clear needing a `TRANSFER_DST` the image is not created with, an
un-ringed per-frame UBO, an unobservable `prev_view_proj`, an unexecutable anti-vacuity clause.

**Not one lands against the foundation** — the RHI `TextureView`, the framegraph subresource guard,
or the host oracle. Three design rounds, zero blockers there. That is evidence rather than
preference, and it inverts my original reason for recommending (b): it was a hunch then, it is a
measurement now.

Some blockers exist BECAUSE the foundation does not: one says outright that a step's acceptance
cannot be executed at that step because the instrument does not exist yet. Building the foundation
first removes a class of objections rather than postponing it.

**So: implement S1 (RHI `TextureView`), S2 (framegraph subresource guard) and S3 (host oracle) now.**
Each is independently correct, needed regardless of whether occlusion culling is ever armed, and
none depends on a disputed part. The feature's design continues to settle against its remaining
blockers, on a foundation that by then exists.

**Blocks.** The occlusion feature only. The foundation proceeds.

---

## 2026-08-07 — TWO THINGS PIECE 3 CANNOT DECIDE FOR ITSELF

VG R3 piece 3 is COMPLETE and pushed (`b6337dd`..`6a9a7f9`). Two items are blocked on the owner,
and neither is a defect.

### 1. Four new pins are UNBLESSED, and blessing is not mine to give — **RESOLVED: blessed @e160434**

**The owner reviewed the BMPs and signed off; all four legs now record
`85b7d378…4d2913d9` and re-verify green.** The review raised one real question — the corner
spheres look stretched — which was investigated before blessing, not waved through: the
silhouettes are ellipses with RADIAL major axes at `1/cos θ ≈ 1.18` (FOV_Y = 52°), and a
pixel-exact Bevy 0.14 replica of the two corner spheres reproduced the same ellipses to 0.2 px.
Rectilinear perspective, not a defect. The original record follows.

`goldens/PINS.toml` gained `vb_occ_mixed_off`, `vb_occ_mixed_keep`, `vb_occ_mixed` and
`vb_occ_mixed_late`, every `sha256_*` seeded with the literal `PENDING`. That is the path the file's
own header prescribes for adding a leg by hand; `golden.ps1` reports "NO PIN recorded" and exits 2
on all four rather than passing. **Verified, all four.**

All four render the SAME image:

    actual = 85b7d3788130a8bb65f0b5b92ba86c71499bd7a4babe7d6900a711944d2913d9

That identity across four regimes — disarmed, FORCE_KEEP, armed (defers 4), FORCE_LATE (defers 6,
re-admits 2) — is the piece's central claim: the cull rejects geometry and the picture does not move.

**What is needed:** a visual sign-off on the freshly-dumped BMPs. Then bless `vb_occ_mixed_off`
first (both legs) and verify the other three reproduce the same literal;
`the_pins_declared_byte_identical_actually_agree` keeps them from drifting afterwards. Until then
those four gates claim nothing — `PENDING == PENDING` is vacuous, and the guard's own doc now says so.

### 2. Piece 4 has no plan, and its scope is a VALUES call — **RESOLVED: planned @799db99, shipped P4-1…P4-7**

**Both halves are answered.** `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` went through four
architect × four critic rounds to APPROVED (@799db99) and landed as seven rungs, each committing
alone and green: `49e5630` · `28c3772` · `85b3313` · `c7465bf` · `58687d3` · `cf2d367` · this one.

- **The config field exists.** `boyko_render::OcclusionConfig { mode: OcclusionMode }` — two
  variants, `Off` (default) and `TwoPhase` — a Resource on `HzbConfig`'s surface, read live per
  frame. `BOYKO_VG_OCC_FORCE` and its boot panic left shipping code; the verdict overrides are now
  `boyko_app::OcclusionForce`, a test instrument. **`FORCE_KEEP` is no longer the disarm route;
  `OcclusionMode::Off` is**, and unlike `FORCE_KEEP` it suppresses the split predicate, the late
  passes and the extra descriptor-set bindings.
- **The number exists, and it says NOT RESOLVED.** Ten timestamp brackets in the shipping recorder,
  the piece-3 protocol re-run on that channel: `NetRun +10 240 ns` against a band of `49 152 ns`.
  Every contrast `NOT RESOLVED` — and that is a RESULT, not a failure: the instrument resolves
  per-pass costs, these fixtures do not separate the arms. **The default stays `Off` as a SCOPE
  statement**, not as an inconclusive measurement: default-ON would need `NetRun < −band` across ≥3
  sittings on ≥2 fixtures of differing occlusion density *and* a second consumer for the pyramid, so
  that `HzbBuild` is not charged to this feature alone. P4-6 is one campaign, two fixtures, one
  machine, no second consumer.
- **What is left for the owner is one VALUES call**, recorded in the next item: flipping the default.

The original record follows.

There is no `VG-R3-P4-*.md`. Piece 3's own text assigns piece 4 the **owner-facing config field** —
occlusion culling as a setting rather than an env var — and until then the supported disarm is
`BOYKO_VG_OCC_FORCE=keep`. That is a product-surface decision, not a perf/architecture fork, so it
is not mine to settle.

Piece 4's other inherited job is a number. The three-number measurement returned **NOT RESOLVED on
every contrast**, and the reason is structural rather than statistical: nothing brackets
`vb_batch_cull`, `vb_cull_late` or the late raster scope with timestamps, and `swapchain.rs` sets
`VK_PRESENT_MODE_FIFO_KHR` unconditionally, so wall clock is bounded below by the display refresh
(measured 6.893 ms/frame = 145.1 Hz). The zero control came in at 0.47 % against a resolution band
of 287.91 %. Adding the bracket touches the shipping recorder, which piece 3's boundary excludes —
so it is piece 4's first job if piece 4 wants a number.

---

## 2026-08-07 — VG R3 piece 4 is COMPLETE: two VALUES calls, and the dispositions that close the piece

Rungs P4-1…P4-7 all landed. **Nothing here blocks anything** — the piece ships with the default that
was designed for it. Two items are the owner's to decide, and the rest is recorded so no disposition
is left implied.

### VALUES 1 — should `OcclusionMode` default to `TwoPhase`?

It is one attribute, and a real behaviour change: with piece 4's host disjunct, any world carrying an
`OcclusionCulling` marker would then build a depth pyramid by default.

**My position: no, and it is not an inconclusive measurement.** The decision's failure mode is
DELETED GEOMETRY while its upside is bounded by the early raster's share of a frame — the same
asymmetry that makes the marker itself opt-in. On this corpus the benefit is provably zero (a
converged static scene's late scope correctly draws nothing) and the cost is not. The bar for
flipping it is written down and unmet: `NetRun < −band` across ≥3 sittings on ≥2 fixtures of
differing occlusion density, **and** a second consumer for the pyramid so `HzbBuild` is not charged
to occlusion alone. P4-6 is one campaign, two fixtures, one machine, no second consumer.

### VALUES 2 — present mode is a product surface, and nothing owns it

`present/swapchain.rs` creates the swapchain with `VK_PRESENT_MODE_FIFO_KHR` **unconditionally**, so
every wall-clock measurement in this repository is bounded below by the display refresh, and there is
no owner-facing way to ask for anything else.

Piece 4 deliberately did **not** fix it (disposition (c1)). The reason is not cost: the channel it
would have improved — host wall clock — was superseded by the timestamp brackets and is now labelled
`KNOWN-BLIND`, deciding exactly one thing (*did arming the instrument wreck the frame?*). Present
mode is vsync, tearing and power: an owner-facing `PresentConfig`, and a product decision, not an
occlusion piece's business. **Recorded here as a VALUES item rather than silently carried as a
perf TODO.**

### The dispositions, so none is implied

| item | disposition |
|---|---|
| **(c1)** unconditional FIFO present mode | **OUT** — superseded channel + product surface. VALUES 2 above |
| **(c2)** D8: `vb_indirect_late`'s provenance is covered by nothing | **OUT, BOOKED to framegraph core.** Piece 4 declared no new access, so it neither improved nor worsened it; P4-5 additionally asserts the shipping late chain is field-identical with and without the readback probe. The fix is P2-7's `is_write \|\| res_written \|\| res_seeded` change plus a 14-site audit, whose only gate is a replica this campaign has MEASURED blind to the class it would catch — so that rung needs an instrument before it needs code |
| **(c3)** PROBE-ON barrier-stream rows | **DONE at P4-5**, as a derived delta. Two findings: the plan's re-sourcing prediction is refuted by the tree, and the probe's perturbation is larger than any doc said — nine declared accesses over two passes on seven buffers, eight derived barriers, five pinned barriers moving |
| **(c4)** the intra-pass `TRANSFER → COMPUTE` edge on `VbCullUniform` | **DONE at P4-3, the record-order half only** — and it is a COMPILE-time red in both profiles, where the plan's own shape would have stayed green on the very defect it existed to catch. The DECLARATION half stays open (OQ 9): `FrameGraph::pass_access_count` is private and there is no per-pass accessor |
| **(c5)** the stale future-tense header in `vb_occ_split_gate.rs` | **DONE at P4-7.** It sat in FOUR places, not the two the plan named |
| **(c6)** `goldens/PINS.toml`'s UTF-8 BOM, which strict TOML rejects | **KNOWINGLY LEFT, and the reason is measured.** `golden.ps1 -Bless` writes the file back with `Set-Content -Encoding UTF8`, and the only PowerShell on this box is 5.1, whose `-Encoding UTF8` is BOM-**ful** — verified by round-tripping a BOM-less file through that exact call and getting `EF BB BF` back. Stripping the BOM alone would be silently undone by the next bless; the fix belongs at the WRITER and lands with a bless run. No impact today: `golden.ps1` parses with line regexes, and every strict-TOML check in this campaign strips the BOM explicitly |

### Two gaps piece 4 opened and could not close, recorded rather than absorbed

- **The pin-binary split gate runs PROBE-ON while the pins run PROBE-OFF.** The gap is small and
  named: `vb_probe_dump` is a host-side counter sink that records no commands and cannot enter the
  split predicate. It is still a gap.
- **The dual-read equality invariant is dev-profile only.** A release bench run does not execute it.
  If a release-only divergence between the two query readers is ever suspected, the check has to be
  re-run in the dev profile on the same scene; nothing in the ladder can detect it in release.

---

## 2026-08-07 — RESOLVED — the cull verdict divided, and a division cannot agree with a host oracle

**ANSWERED and implemented in the same session. Kept here in full because the reasoning is the
transferable part, and because the FIRST reading recorded below was wrong in a way worth preserving:
it named a direction from a sample of one.**

**Resolution.** The verdict no longer divides. `for all i: cz_i < occ * cw_i` replaced
`max_i(cz_i/cw_i) < occ` in the shader and in `boyko_render::hzb`'s oracle, `depth_near` moved under
`#ifdef VB_CULL_DEBUG_PROBE` so the shipping module no longer computes the quantity that used to
decide, and the boundary corpus was re-derived to plant against the new predicate. Measured after:

    DepthNearCensus { compared: 72, identical: 72, gpu_below: 0, gpu_above: 0, max_ulps: 0 }
    verdict disagreements: 0 of 72
    24 EXACT-TIE KEEP probes, 24 strict KEEP probes, 24 strict REJECT probes

The tie arm is what proves `<` is strict, and it is now reachable by construction rather than by
luck: the plant uses `z = near·2^k` and `occ = 2^-k`, both dyadic, so the tie is exact on both sides.

**One thing the fix cost, and it is the part worth remembering.** Re-pinning the artifact census
showed `op_ford_less_than` going DOWN by two at the exact step that ADDED a per-corner comparison —
because `!(cz < bound)` lowers to `OpFUnordGreaterThanEqual`, which the census had no field for. A
census that counts only the ordered compare would have read a verdict's *deletion* as a small
decrease and pinned it without comment. The field was added
(`op_funord_greater_than_equal: 4`, two of them the verdict, one per inlined copy).

---

### The finding as originally recorded

**Not a blocker. Recorded because it is a MEASURED correctness finding, and because the fork it
opened was mine to decide — the owner should be able to overrule it before it ships.**

VG R3 piece 3 step P3-4 (the occlusion leaf) is in the working tree, uncommitted. Its new gate
`crates/boyko_app/tests/hzb_verdict_oracle_gate.rs` runs four corpora. Three pass, including the
131,072-pair random corpus and the sentinel corpus. The fourth — exact tangency — fails on its first
probe:

    [64x48 boundary probe 0 (equal)] batch 0: the record's instanceCount is 0 but the oracle
    keeps 1 of 1 instances early. (deferred: gpu 1 / oracle 0)

The GPU **rejects** where the oracle **keeps**. The shader's comparison
(`vb_batch_cull.comp.hlsl:872`) is `return depth_near < occ;` — strict, and correct: equality must
keep. So the operator is right and the VALUE differs — the shader's `depth_near` lands below the
host's. The shader's own comment at `:766-767` named this in advance as *the geometry-deleting
direction*, and the fixture's comment at `:1309-1313` predicted the exact signature: a 1-ULP
disagreement "would show up as a failure on the exactly-equal arm and nowhere else."

**Both predictions were written before the run, and both came true.** The gate is working. This is
the campaign's eighth instance of the pattern — and the first where the instrument caught the defect
instead of being vacuous over it.

### The measurement, and what it overturned

A `-D VB_CULL_DEBUG_PROBE=1` variant now exports the shader's own `depth_near`, level and taps, so
the divergence is OBSERVED rather than inferred. The shipping module is untouched: the
macro-undefined source preprocesses character-identically and `vb_batch_cull_spv_byte_identical`
stays green, so the numbers describe the module that actually ships. Over 72 boundary probes:

    DepthNearCensus { compared: 72, identical: 66, gpu_below: 3, gpu_above: 3, max_ulps: 1, incomparable: 0 }
    verdict disagreements: 2 of 72   (one host=Early gpu=Late, one host=Late gpu=Early)

**This overturns the first reading above.** The divergence is NOT in the geometry-deleting
direction — `gpu_below` and `gpu_above` are 3 and 3, and the two verdict disagreements point
opposite ways. It is symmetric rounding at 1 ULP, not a bias. The first reading came from a single
probe, which is exactly the sample size at which a direction claim is worth nothing.

`level` and all four `taps` are IDENTICAL on every one of the 72 probes, so the window rect and the
level selection already agree exactly and only the depth differs.

### The cause, and why it closes option (A)

Under the corpus matrix, row2 = `[0,0,0,near]` and row3 = `[0,0,1,0]`, so `cz = near` and `cw = z`
are exact and bit-identical on both sides — corroborated by the identical taps. The only inexact
step left is the reciprocal. Vulkan's precision appendix specifies `OpFAdd`/`OpFSub`/`OpFMul` as
correctly rounded but allows **`OpFDiv` 2.5 ULP** at 32-bit; Rust's divide is the IEEE 0.5-ULP one.
`precise` emits `NoContraction`, which constrains contraction and reassociation and says NOTHING
about a division's ULP allowance.

So **(A) as originally posed is dead** — not expensive, *impossible*: no amount of tightening the
existing fold reaches bit-exactness, because the gap is a spec allowance, not a code shape. The
shader comment at `:909-910`, which claims `precise` "forbids substituting a reciprocal-estimate",
is a false claim and is being corrected.

**(B) is also wrong now** that the direction is known: rounding UP would trade one arbitrary
direction for another, and the 1-ULP bound it would lean on is measured on 72 probes on ONE device
while the spec permits 2.5.

### What is being done instead: remove the division from the DECISION

For `cw_i > 0` — already guarded by the behind-eye early-out —

    max_i (cz_i / cw_i) < occ    <=>    for all i:  cz_i  <  occ * cw_i

The right-hand form is one correctly-rounded multiply under `NoContraction`, so the shader and the
oracle agree **by construction** rather than within a tolerance, and it is *cheaper* than the divide.
That is an exact reformulation, not a relaxation and not a bias — which is why it is preferred over
every option originally listed. The window rect keeps its divide, which is measured to agree exactly.

**(C) — declare tangency untestable and relax the arm — stays rejected**, and is now unnecessary.

---

## 2026-08-07 — Profiling + logging, review round 3: both REJECTED, and the SEAM is incompatible

The two plans reached revision 3 and were reviewed a third time — separately, and for the first
time **against each other**. Verdicts: profiling `REJECTED (6 blockers)`, logging
`REJECTED (10 blockers)`, seam `INCOMPATIBLE AS WRITTEN (6 blockers)`. Revision 4 is in flight;
these three items are not the reviewers' to decide.

### The seam was never designed, and that is the round's main finding

Two prior rounds read one plan each. The first reader of the seam found the two documents
asserting **contradictory facts**: profiling justifies moving its ABI into `boyko_utils` because
that crate has zero dependencies; logging states flatly that `boyko_utils` depends on `boyko_log`.
Both cannot hold. Below that, each plan independently invented the same four primitives — a
per-thread lane index, an `rdtsc` calibration, a never-freeing lane allocator, and a loss
accounting — with incompatible semantics: one worker would be lane 5 to the profiler and lane 37
to the logger, and only one of the two clocks would know about a suspend/resume. That is precisely
the failure Principle 0 names: a capability two subsystems need, built twice as per-crate adapters
instead of once as a kernel feature.

**Decided by me, not the owner** (architecture, per the standing agreement): a new zero-dependency
bottom crate `boyko_diag` owns the clock, the lane registry, the loss vocabulary and the
never-freed storage policy; it is *diagnostically mute* (it emits no `boyko-####` code and prints
nothing, which is what keeps the graph acyclic); `profiling_abi` is hosted there rather than in
`boyko_utils`, which keeps its empty `[dependencies]`. Full design:
`docs/DIAGNOSTICS-SUBSTRATE-PLAN.md`.

### VALUES 1 — how much does a SHIPPED title pay for diagnostics?

Nobody had computed the joint number. Measured from the two plans' own tables:

| | profiling alone | logging alone | **jointly** |
|---|---|---|---|
| dev, `.bss` + reserved | 6.65 MiB | 3.46 MiB | **9.33 MiB** (10.11 naive; the shared crate saves 0.78) |
| **shipping** | 0.85 MiB | 1.16 MiB | **1.95 MiB** — **WRONG; corrected immediately below** |
| hot-path cache lines | 3-4 | ≤ 4 | **7-8** |

> **CORRECTED 2026-08-08 — the shipping figure above was already wrong on the day it was first put
> to you, and it is corrected in the open here rather than quietly re-based.**
>
> **1.95 MiB has never equalled the sum of its own operands, in any revision.**
>
> - As put to you above (rev 3): `0.85 + 1.16 = **2.01**`, printed as **1.95**.
> - At the corpus's first carved revision the operands moved and the total did not:
>   `0.89 | 1.15 | naive 1.95`, and `0.89 + 1.15 = **2.04**`.
> - **Then a second, independent error surfaced underneath the first.** The logger re-derived its
>   own `shipping` column term by term (`docs/diagnostics/logging/01-EMISSION-RING.md:130`:
>   512 + 32 + 16 + 16 + 4.25 + 0.008 + 256 + 64 + 320 = **1 220.26 KiB ≈ 1.19 MiB**) and showed
>   that **no subset of its rows sums to the 1 180 KiB** the seam was quoting — so 1.15 was not a
>   different configuration, it was wrong too.
>
> There is no third quantity 1.95 could have been. Both revisions state that the shared substrate
> saves **ZERO bytes in shipping** — the 0.78 MiB saving is dev-only — and with a zero saving the
> joint figure simply **is** the naive sum. **The corrected shipping figure is ≈ 2.08 MiB**
> (908 + 1 220.26 = 2 128.26 KiB), against ≈ 2.01 MiB on the numbers as they were handed to you.
> The error ran against you every time: the ask was understated by 0.06 MiB then and by 0.13 MiB
> now.
>
> 🔑 **The lesson that outlives the number, because it defeated a repair pass whose stated job was
> to catch exactly this.** After the first correction the seam table was *internally consistent* —
> `0.89 + 1.15` really is `2.04` — and that is precisely why the stale **operand** survived. **A
> total that checks out against its printed operands proves nothing about those operands.** The
> durable rule: with a zero shipping saving the joint figure is the sum of the two columns, and
> any edit to it must re-read the source rows it quotes rather than re-adding the numbers already
> printed beside it.
>
> **What the figure MEANS also changed, and that narrows what is being asked.** S13 —
> *free when not enabled*, folded in after this entry was written — moved every syscall, thread,
> hook and first write off the boot path and onto the enable path, so a shipped process that never
> enables diagnostics **never touches these tables at all**. An untouched all-zero `.bss` table is
> emitted by the linker with a virtual size and no raw data, so ≈ 2.08 MiB is **declared address
> space, not resident RAM** (`docs/diagnostics/SEAM.md` §S13, MEMORY row). Two limits on that,
> both stated by the corpus itself rather than smoothed away: the property holds **only if boot
> touches nothing** — one write to one lane buffer commits that page and it is lost for that page
> — and the corpus **explicitly refuses to claim** that the loader leaves an untouched page
> uncommitted (`substrate/section-report` proves the bytes are absent from the *image*, and no
> more; `docs/diagnostics/substrate/05-LADDER-GATES.md`, gate DG12).
>
> **So the question is narrower than this section's heading suggests.** Not *"what does a player's
> machine spend on diagnostics"*, but: **is ≈ 2.08 MiB of declared address space — resident only
> in the sessions where diagnostics are actually switched on — an acceptable price for a shipped
> title?** Still a VALUES call, and still not mine.

So the profiling plan's headline **"≤ 1 MiB retail" is false in the configuration that will
actually ship**, and the shared substrate saves **nothing** in shipping — its 0.78 MiB saving is
dev-only. It is bought for correctness (one lane number, one clock epoch, a loss report that
cannot itself be dropped), not for footprint, and neither plan may claim otherwise.

Cutting **≈ 2.08 MiB** means cutting one of: logging's 32 × 16 KiB lanes (512 KiB), `SINK_OUT`
(256 KiB), or the profiler's dynamic-zone arenas (96 KiB — *the current revision states this third
candidate as **40 KiB** in `shipping`, `docs/diagnostics/SEAM.md` §Open — needs the OWNER, item 1;
the divergence is recorded here, not resolved, because resolving it belongs to the profiling
plan*). **This is a VALUES call about what a player's machine spends on diagnostics, and it is not
mine.**

### SCOPE 1 — what does `shipping-min` actually mean?

Logging's `shipping-min` exists for a title that wants **no resident diagnostics thread**. But
profiling's `Always` tier still writes a telemetry window synchronously on the dispatcher, so such
a title pays a periodic `write_all` anyway. Either `shipping-min` also disables telemetry, or the
profile does not mean what its name says. **SCOPE call.**

### SCOPE 2 — the plans are growing faster than they are converging

Three review rounds, and the blocker count has not come down: **35 findings → 17 → 22**. The two
documents are now 3370 lines for two subsystems that do not exist as a single line of code, and
more than half of round 3's new blockers were introduced by what round 2 added — the game-facing
half — while the seam only became visible because both documents grew into full architectures.

That is a signal about **how much is being designed at once**, not about the reviewers. The
alternative is a narrowed first tranche — `boyko_diag` + CPU zones + log levels — built and
measured, with telemetry, retention and the game-facing API returning afterwards on a working
foundation. Stated here as an option; **the owner decides the scope, not me.** Work continues on
the full revision 4 unless told otherwise.

### SCOPE 3 — the workspace's `--cfg loom` leg has not compiled, and nothing said so

Found at rung D1, 2026-08-08, while checking that the new `boyko_threadpool -> boyko_diag` edge
did not disturb the loom build. It did not. The loom build was **already broken**:

```
RUSTFLAGS=--cfg loom cargo check -p boyko-threadpool --lib
error[E0599]: no method named `get_mut` found for struct `loom::sync::atomic::AtomicPtr<T>`
  --> crates/boyko_threadpool/src/scope.rs:185:39
```

loom 0.7.2 offers `with_mut`, not `get_mut`. `-p boyko-ecs` fails on the **same** error because it
reaches the same lib, so **both** crates that carry a `[target.'cfg(loom)'.dependencies]` block are
dead, not one. `rg loom .github/workflows` returns nothing — no CI leg passes `--cfg loom`, which
is why this has been invisible. Confirmed not to be a D1 regression: `git stash`-ing the D1 diff
reproduces the identical error at `93dbcf8`.

Why it needs a decision rather than a quiet fix:

1. The substrate plan cites these two crates as the working precedent `boyko_diag`'s `claim_lane`
   loom model will copy. The **manifest** shape is a valid precedent; the claim that a model *runs*
   beside it is not. The plan text is now qualified — the citation is not silently left standing.
2. The fix at `scope.rs:185` is one line inside an `unsafe` `Drop` on the scope-teardown path, and
   a green `cargo check` under `--cfg loom` is **not** a run model. Making the leg mean something
   means running the models, and this machine crashes loom **release** binaries at startup
   (recorded previously, unrelated).
3. So the real question is scope: (a) repair `scope.rs:185`, run the existing models in debug, and
   add a CI leg that keeps them compiling; (b) repair it and add no CI leg, accepting the same
   silent rot later; or (c) leave it, and land `boyko_diag`'s concurrency evidence as the
   proptest + Miri legs only, stating in the plan that no loom model backs `claim_lane`.

**Not repaired at D1** — it is outside D1's subject and the choice above is not mine. The lane
claim path's Miri and property legs are unaffected and still planned.

---

## KNOWN FRICTIONS — no decision needed, recorded so they are not rediscovered

- **`target/` grows without bound and silently breaks builds.** It reached 73 GB and hit zero free
  space mid-build this session. The failure presents as a *mingw linker error*; the real cause is on
  the last line (`no space on device`). `cargo clean` recovered 73.6 GiB.
- **The trybuild test fails under a concurrent full-suite run** (`compile_fail_frame_write_token`,
  3/3 fixtures) and passes standalone. trybuild spawns its own cargo into the same `target`. Not a
  flake — reproducible contention.
- **`cargo test` STOPS at the first failing binary, and the suite count silently shrinks.** A run
  that trips the flake above reports ~51 suites instead of ~445 — so "I ran the full suite" can mean
  "I ran a ninth of it" with nothing in the output saying so. Always pass `--no-fail-fast` when the
  claim being made is about coverage, and read the suite COUNT, not just the failure count.
- **graphify has been off-target for the render/VB path** for this entire session; every query
  returned `boyko_demo` internals. Grep/Read is the working path there.
- **The `graphify` binary is not installed in this environment at all** (not on `PATH`), while the
  `PreToolUse` hooks demand `graphify query` before every Read and Grep. `graphify-out/` exists but
  its newest subdirectory is from July, so the graph is also stale. The hooks only remind, never
  block — but they fire on every file access. Fix is either an install or a hook that checks the
  binary exists before demanding it.
- **The ECS's global query-type registry can exhaust under the full lib suite.**
  `MAX_QUERY_TYPES = 1024` is a process-global cap minted lazily, and `boyko-ecs --lib` runs 864
  tests in parallel. When scheduling happens to mint the 1025th distinct query shape, whichever test
  is unlucky dies with a TERMINAL panic naming the cap. Observed once, then 3 consecutive clean runs
  of the same binary. It is order-dependent, not a regression signal — check by re-running before
  bisecting anything.
- **`boyko-engine --test internal_docs_anchors` is RED on `master`'s content, and has been for a
  while.** 13 stale line anchors (10 in `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md`, 3 in
  `docs/SYSTEMS.md`) plus 8 over-waivers against a cap of 6 in the meshlet plan. **Proved
  pre-existing at L5** by stashing the whole rung and re-running: byte-identical failure. It makes
  `cargo test --workspace` red for everyone, so the next person to run it will spend the same
  fifteen minutes proving it is not theirs — which is the cost of leaving a standing red in place.
  Re-deriving the anchors is a self-contained chore and belongs in its own commit.
- **A line inserted into a widely-cited source silently invalidates every doc anchor below it.**
  L5 added `VmColumn::as_mut_slice` at ~line 355 and shifted `vm_column.rs:437-449` to `462-474` in
  two corpus files. The anchor test does not cover `docs/diagnostics/**`, so nothing would have
  said so. Grep `<file>.rs:[0-9]` across `docs/` after any insertion into a kernel primitive.
- **A gate can be RED for five commits because nobody runs it, and no gate can close that.**
  `G2a`'s file census (`tests/gpu_blocking_reader_census.rs`) went red the instant rung 5a landed
  `present/gpu_zone.rs`, whose module doc names `vkGetQueryPoolResults` while explaining what the
  BLOCK cost. It stayed red across `ee9196b6`, `cb54752d`, `8ca4e05b`, `cf8ffd20`, `7ae9162a` —
  three of which reported "workspace green". Found at 5c only because a full `--workspace` run
  finally happened. The mechanism worked perfectly; the *asking* was the gap. Practical rule:
  **a rung that adds a file must run the census gates, not only its own.**
- **The disk fills to zero and it looks like a mingw linker bug.** Second occurrence (see the
  2026-07-23 audit note). `target/` reached 72 GB with 8 KB free on `D:`, and the symptom was a wall
  of `linking with x86_64-w64-mingw32-gcc failed` across a dozen unrelated targets. Cure:
  `rm -rf target/debug/incremental` (12 GB here). Check `df -h /d` BEFORE reading a linker error.
- **G10's A/B runs as two processes and the corpus says one; the reason is in the docs but it is a
  deviation an auditor should see.** Leg A's `read_vb_bench_ns` waits with `VK_QUERY_RESULT_WAIT_BIT`,
  so a single process alternating legs would reach it on a frame leg B recorded — the hang class
  P4-1 removed. If a future rung wants the corpus's literal shape, it must first make leg A's
  readback non-blocking, which is rung 7's deletion anyway.
- **RESOLVED (owner: "whichever is more performant") — profiling rung 6's G10 fork went to the host
  arming knob.** `BOYKO_GBUF_BENCH` costs one boot-time `Option` that is `None` in every shipped run
  and expires with rung 7; the alternative would have forced `GpuZoneRecorder::open_frame` and
  `retire` to `&self`, deleting clause (c) of `FrameSlot`'s `Sync` argument permanently and pushing
  `set_mark` toward a locked RMW in a hot recorder. **The leg is armed and never read** — the witness
  clause needs no timings, and `read_query_pool_ns` would hang on a frame that skipped a pass.
  ⚠️ **Rung 8's reader must consult the witness masks before it waits on anything**: three of the
  four software-ray passes are bracketed inside their own `if let` arms, and neither old gbuffer
  collector has a totality epilogue.
- **RESOLVED (owner: "decide yourself what is optimal") — profiling rung 7's artifact format is
  decided and its writer/reader/gate have shipped.** See `profiling/artifact.rs`'s module doc for
  each decision and what it costs to get wrong. ⚠️ **One of the six answers turned out to be wrong in
  its REASONING and was corrected by insisting on a RED**: the fear that a wider file collapses
  `vg_occ_split_timing.rs:916`'s GCD is false — that consumer's own `(v * 10.0).round()` absorbs the
  extra digits, measured across seven values. One decimal is still right, for smaller reasons
  (direct comparability with the printed lines, which is what makes the next step's A/B possible).
  The six values, as decided:
  1. **Numeric precision in the artifact.** `vg_occ_split_timing.rs:916` reconstructs the GPU tick
     lattice by GCD over **tenths**, because that is the precision the summary prints. Full-precision
     `f64` collapses the GCD and sub-floors every band; the file's own doc measures the error at
     **32×** and says such a choice *"would satisfy every assertion here while under-stating the
     instrument's resolution by the whole lattice factor"*. **A silent false-win, not a red test.**
  2. **File path, per-run uniqueness, truncation.** `append_artifact(p, path)` takes a caller path
     and nothing else — no default, no rotation, no env knob. `vg_decidability_floor.rs` spawns 42
     sequential children; a fixed path is a stale-read generator.
  3. **One file = one sitting, one process, or many appended runs.** `G24`'s reverse RED is *defined*
     on staleness and cannot be written until this is chosen.
  4. **Who aggregates the 21 per-session artifacts into `docs/PROFILING-FLOOR.md`** — rung 7b
     depends on it and no line assigns it.
  5. **Whether `WorkloadTag` is an artifact field.** `resolve` checks it, 7b publishes it into
     markdown, nothing says the session file carries it.
  6. **What the artifact records when the device declines timestamps** — today an `eprintln!` that
     three consumers key their third outcome on.
  ⚠️ **`G24`'s reverse RED named two fields that cannot do the job, and one of them does not exist.**
  `crates/boyko_diag/` has **no `build.rs`** and `BUILD_HASH` appears nowhere in the workspace — a
  planned rung-0 artifact that never landed. `SessionId` exists but is minted INSIDE the child, so a
  parent cannot predict it. The discriminator is therefore a **parent-supplied run token**, the only
  field that can catch staleness within one run.
  Rung 7's remaining halves, in order: the reducer that fills the artifact, the producer wiring
  (verified by A/B against the still-printing channel while BOTH are live), 7b's floor
  re-measurement, then the deletions — **713** lines from `gpu_timing.rs`, **1381** from `runner.rs`
  (31 % of the file), **465** from `gpu_scene/mod.rs`, plus five consumer migrations.
- **RESOLVED (owner: the STRICT option of three) — the workload tag is two halves, and an
  undeclared one is not a floor.** Measured while opening rung 7's consumer migrations. The tag is
  `format!("{path:?}_{legs:?}")` over `ResolvedRenderPath` — `deferred_both`, `visibilitybuffer_mesh`
  and so on. `vg_decidability_floor.rs` runs its NULL experiment twice per repetition, once with
  `BOYKO_VB_FROXEL_FORCE_OFF` and once without, and **neither `path` nor `legs` changes between
  them**: `froxel_light_cull` is a separate field of the same struct and the tag does not read it.
  `BOYKO_VB_BENCH_LIGHTS` (`N_ps`) is invisible to the engine entirely.
  The migration itself is not blocked — the floor test writes each leg to its own file path and
  never reads the tag to separate them. What the hole costs is **downstream**: `resolve` refuses a
  `Floor` whose `workload` differs, and that refusal is the ONLY mechanism keeping a floor measured
  on one configuration from bounding a delta measured on another. With this tag, a flat-leg floor
  silently bounds a froxel-leg claim — which `vg_decidability_floor.rs`'s own "What this does NOT
  decide" forbids in words (*"It is one CONFIGURATION"*, *"a rung that measures at a different scale
  must re-measure its own floor rather than cite this one"*).
  ⚠️ **And a correction to this entry's own first sentence about the mechanism.** It said `resolve`
  refuses a mismatched `Floor` as if that were shipped code. MEASURED: `Floor`, `resolve`,
  `FloorWorkloadMismatch` and `NotResolved` exist **only in the corpus documents** — `rg` over
  `crates/` returns nothing. They are rung 8's content and are unwritten. So nothing was silently
  wrong; the tag is the INPUT to a comparator that does not exist yet, which is why what it names
  had to be settled before 7b publishes a floor that later rungs cite.
  **Shipped as decided:**
  * **Derived** — [`config_tag`] over the WHOLE `ResolvedRenderPath` (readable `path_legs` prefix +
    8 hex of FNV-1a over every field), not a hand-picked subset: the bug was not that the wrong
    field was chosen, it was that fields were chosen. A field added to that struct invalidates prior
    floors deliberately — floors on this box drift faster than that anyway.
  * **Declared** — `content_tag`, from `BOYKO_PROFILE_WORKLOAD`, set by the measuring test in its
    own spawner code where the value already lives, not in an operator's shell.
  * **The refusal is enforced NOW, not promised to rung 8**: `Artifact::floor_source` returns
    `UndeclaredContent` on an empty or blank content tag, because a clause whose subject does not
    exist yet is a promise rather than a gate. A missing KEY is a malformed header, kept distinct
    from a present-but-empty one — the whole of the refusal is that distinction.
  RED run: revert the derivation to `path × legs` ⇒ *"the flat and froxel legs produced the SAME
  workload tag ... flat: deferred_both, froxel: deferred_both"*. Measured on a live run:
  `workload_tag = "deferred_both#99f4482e"`.
- **`boyko-ecs --lib`'s profiling tests are ORDER-DEPENDENT FLAKES — now TWO of them.**
  `every_dispatching_round_records_one_span_and_one_width` joined
  `one_zone_taking_a_hundred_thousand_samples_keeps_count_exact` on 2026-08-10: observed failing
  once in a full `--workspace --all-targets` run, then passing in isolation and in two consecutive
  full lib runs (915/915 each). **A second name in the same class raises the priority**: this is
  no longer one unlucky test but a property of the module, and a real regression here would be
  indistinguishable from the flake. Original entry follows. Observed failing once in a full `--workspace --all-targets` run, then
  passing in isolation and in two consecutive full lib runs (915/915). Same class as the
  `MAX_QUERY_TYPES` note above: process-global profiling state (lanes, zone slots) against 915
  tests in parallel. Re-run before bisecting.
- **`.claude/settings.local.json` is dirty** from earlier sessions and is deliberately never staged.
