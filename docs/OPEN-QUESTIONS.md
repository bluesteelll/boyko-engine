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

---

---

## 2026-08-30 — what the adversarial pass found in the rulings above, and the kernel finding that outranks all of them

**Recorded, not repaired.** The rulings in this file landed and their gates are green; two independent
reviewers then found the items below. They are written here rather than fixed because a repair taken
on incomplete information generates a second repair — and one of these findings changes the axis
that F2 was decided on.

### ⚠⚠ KERNEL — every `par_iter` inside a system body runs on ONE thread

Measured 2026-08-30, three independent instruments, release, 16 workers, 16384 rows, ~50 ns/row body:

| arm | ns/row | speedup | max in flight |
|---|---|---|---|
| `for_each_chunk`, no pool (reference) | 50.10 | 1.00× | — |
| `par_for_each_chunk` under `pool.install` | **6.51** | **7.69×** | 16 |
| `par_iter()` **in a system body** | 49.36 | **1.01×** | — |
| `par_for_each_chunk` **in a system body** | 48.69 | **1.03×** | **1** |

**Mechanism, one line.** `crates/boyko_threadpool/src/worker.rs:370-371` (`push_task`) sends a task
spawned *by a worker* into `injector_local[wid]`. Sibling stealing iterates `inner.stealers`, which
holds the worker **deques** only — **no thread ever polls another thread's local injector.** Work
spawned from inside a worker is reachable by that worker alone, serial by construction. A system
body always runs on a worker.

**The query drivers are innocent**: a raw `pool.scope` with zero ECS code, opened from a worker,
serialises identically.

**Second, independent occupancy defect on the same principle.** Even on the healthy path only
**4–5 of 16** tasks are ever simultaneously live: `Scope::drop` steals roughly half the wave into a
private `scratch` deque that has **no registered `Stealer`**, and runs it inline, one at a time.
Same shape — runnable work parked where nobody can steal it.

**Blast radius.** All four parallel physics sites are on the defect path — `solver/colored.rs:2667`,
`soft/colored.rs:1031`, `resources.rs:1688`, `:1760` — plus 7 `par_iter_mut` call sites, all in
system bodies. Render, UI and app have **zero** exposure. ⚠ **The O-series colored-solve speedups
were measured from a bench thread (dispatcher = the healthy path); production takes the serial one.
Those numbers must be RE-TAKEN, not re-read.**

**Why three layers of defence all missed it.** `tests/scheduler_par_iter_concurrent_systems.rs`
asserts no-deadlock and full row coverage and **nothing about threads** (two matches for
`num_threads`, both in a builder). The one bench on the path, `g3_boyko_par_iter_10k`, has an
absolute ≤41 µs ceiling with **no single-thread baseline** and a body that is a `fetch_add` on one
shared static — which runs *faster* serialised. And two doc comments (`thread_pool.rs:128-129`,
`worker.rs:356`) assert siblings *can* see these tasks "via stage 1.5 of `worker_main`" — **there is
no stage 1.5**; `colored.rs:2629-2631` likewise assumes a lane pool of `num_threads + 1` where it is
**1**.

**Inter-system parallelism is unaffected and measured healthy**: four conflict-free systems under
`Schedule::run` give `max_inflight=4`, `lanes_busy=4`, `top_lane=25.1%`. The two kinds of
parallelism this engine plans for are both implemented; the between-systems kind works, the
within-a-system kind does not.

### F2 — the ruling stands but its decisive number was taken at the size that flatters it

`constants.rs:55`: *"Resident floor: one `POOL_MIN_SLAB` (64 KiB) commit per NON-EMPTY column."* A
table archetype has four non-empty columns ⇒ **256 KiB per table, independent of row count.** F3
(owner-ruled) admits N table files through one schema, so twenty small tables of 50 rows × 40 B cost
**~5 MiB resident for 40 KB of payload (~128× overhead)** plus twenty extra archetype mask tests on
every query, every frame. The rejected resource region loses by ~25 % at 4000 rows and **wins by two
orders of magnitude at 50**. The ruling never says the choice is size-dependent, and the mechanical
rule it gives keys on a different property. Marginal cost also mixes units: honest figure is
**511–704 KiB**, not a flat 511 KiB.

⚠ **And the parallelism finding above removes the axis F2 was partly argued on** — "dense does not
parallelise" no longer discriminates between the options, because *nothing* parallelises inside a
system today. F2 must be re-argued on resident memory and access cost.

Internal tension to resolve with it: F2(ii) rejects laziness because *"row constants exist to delete
a lookup"*, and F2(iii) then makes the runtime handle an `Entity` captured at load — reading a row
from an `Entity` **is** a lookup (inland resolve + archetype pool). Access cost is priced on neither
option.

### GK-1 as specified violates Principle 0

`LoadEntityMap` is legitimate **today** because it is a stack local of `load_world` — the named
"truly transient function-local scratch" exception. GK-1 requirement 4 makes it world-owned in a
`Resource` with a lifetime spanning every loaded cell, keeping the `Vec` — which converts the
exception into exactly the durable parallel data system Principle 0 forbids. **The in-tree answer is
one line away and the ruling already cites the file**: `PathIndex { entries: VmColumn<PathEntry>,
sorted_len, … }` — VM-native, sorted prefix + unsorted tail, HashMap-free. `(u64, Entity)` is 16 B
and divides the granule.

### GB-3's dangle check is structurally unreachable downstream of the sigil pass

A dangling asset reference produces **one error-level log line and an otherwise fully-formed,
unvalidated, refcount-holding entity**: `AssetServer::load` logs `E0801` and returns a live handle in
the `Failed` state, which it inserts into the path index. `validate_asset_refs` early-returns unless
`free_epoch` advanced — a never-loaded asset frees nothing, the epoch never moves, and the row is
**never examined**. A reference kind whose only validator is owed is a silent-wrong-answer generator.

### The shape four of the five blocking findings share

> *A mechanism was verified to EXIST and was not verified to be REACHABLE, or to behave, on the path
> being ruled about.*

`Construct` exists — for pooled ids. `remap_loaded_entities` exists — for fresh worlds.
`relationship_on_insert` exists — with a destructive guard that removes the relationship component
rather than skipping. `apply_attach_flags_*` is absent from the loader and reachable from its drain.
**The rulings' citations are accurate; what they under-tested is the second question.** This is the
same question the kernel finding above answers for `par_iter`, from the other side.

### Owner ballots still open

**F8** (name-vs-id) and **F10** (bundle expand-or-refuse) — untouched, and no ruling above settles
either. **GB-5** and **GB-6** carry analyses, deliberately not rulings: the owner asked for
trade-offs. **AB-12** remains, and blocks nothing.

## 2026-08-30 — the owner's rulings, and the Aether ladder closes

Seven ballots answered by the owner in one session, plus eleven decided by the orchestrator under
the standing rule that performance and architecture forks are settled with numbers. **Every `AB`
ballot is now closed except AB-12**, which blocks nothing.

Recorded here as the single index; each ruling's full ground and rejected alternative live at the
site named in its row.

| ballot | ruling | where the ground lives |
|---|---|---|
| **AB-1** | **RATIFIED as specified** — event auto-registration adopted on ergonomic grounds; the STAGE-under-D4 alternative rejected. ⚠ The reopen was licensed because C3's recorded ground — *"the unregistered case fails silently on both ends"* — was **refuted by measurement**: both generated ends are a loud init-time panic. The grant now stands on a stated ground (ergonomics), not on the refuted one | [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) C3 |
| **AB-6** | **Option (b), and the class split in two.** The ballot's premise was refuted first: hand-written Rust could not do it either, so there was no ratified surface to narrow — only a promise the kernel did not keep. **Dense**: build the construct-and-commit route. **Bitset/flag**: refuse in the **derive**, since a flag has no bytes and `FLAGS_DIRECT` is the mechanism. Refusing in Aether alone was ruled out because it would make Aether reject what the derive accepts | [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) AB-6 + **KE14** |
| **AB-7** | **R-DENSE LIFTED, and the ballot's framing rejected.** Owner: *"that there is no parallelism is just wrong."* The candidate driver-independent ground had already been measured and **refuted on both conjuncts**; the owner then declined to treat the driver limitation as a constraint at all. Measured: the refusal is a `const assert` whose own comment says the chunk runner *"has no world cell"* — unwired plumbing, not a design limit, and the same gap blocks `Related` joins | [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) **KE15** |
| **AB-10** | **(a) the measurement script IS required** when the audit branch is taken. **(b) the widening to the joint Aether+Gaia vocabulary is RATIFIED** — if the two languages are one body of work, the vocabulary is one vocabulary, and a word meaning different things across them is a false friend between the author's own languages. Price accepted: the risk list grows with Gaia's vocabulary, and a collision there may force a rename in Aether | [`aether-v2/AI-ORIENTATION.md`](aether-v2/AI-ORIENTATION.md) AIR-10 / AB-10 |
| **AB-11** | **The recommended option — a parse refusal** with a did-you-mean pointing at `enabled` / `disabled`. ⚠ Adds a refusal where v1 documents non-refusal, which is what made it the owner's. Ground, measured with R0's fix already in the tree: over a `flag`, `with F` matches nothing and `without F` excludes nothing — two different silent wrong answers — and the defect is in the **leaves**, so R0's `Or` fix does not reach it | [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) AB-11 row; red test `ab11_flag_filter_polarity.rs` |
| **AB-13** | **`true` / `false`.** Settles all four interacting parts at once: the values are Rust keywords already, so nothing new is reserved; the three-way `on` collision (`machine … on entity`, `on E => T`, `flags (X = on)`) **does not arise**, so neither a reader nor a generator needs lookahead; and the group keeps the name `flags`, PENDING Tier 3's withdrawal of the `flags → initial` rename standing on its own sound ground. Spelling: `flags (Stunned = false, Burning = true)` | [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) AB-13 row |
| **GB-2** | **`#RRGGBB` is `#RRGGBBAA` with maximal `AA`** — a defaulted field, not a second literal kind, which dissolves the reopen question rather than answering it. **And the arity direction the clause did not cover: REFUSE AT BAKE** — an 8-digit literal at a 3-component field (`PointLight.color` is `[f32; 3]`) is a coded refusal naming the field, never a silent alpha drop | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Colour |

**Still open on the Aether side: AB-12 only**, and it blocks nothing. Evidence gathered 2026-08-30
points at option (b): every `G1`/`G2` occurrence in the 2026-08-29 plan session is a **Gaia rung**,
not a defect body, and the surviving `AD1..AD4` all have subjects. That is evidence, not proof — the
search was a grep over a 7 MB transcript, an instrument this campaign has had lie to it three times
in one day.

**Not ballots, and not the owner's: K8 and K9** — the bundle-with-`link Entity` spawn spelling and
the relationship macro's private-field demand — are architecture gaps routed to R3's own design
pass. R3's `bundle` and `relation` constructs may not be declared done while they stand.

### The Gaia side, same day — fourteen ballots put, TWELVE answered

Six the owner settled outright, four he **delegated** (ruled below and in
[`gaia/DECISIONS.md`](gaia/DECISIONS.md)), and two he asked to be **analysed, not decided** — those
two stay **OPEN**, with the analysis attached to their bodies in §2026-08-29 so he can rule from one
reading. **A ruling written for either would be a defect.** Unanswered: **F8** and **F10**, plus
**F9's residual VALUES question**, which the delegated ruling deliberately did not settle.

| ballot | who | ruling | where the ground lives |
|---|---|---|---|
| **F1** | owner | **Macro-time GK-4, NOW** — not sequenced behind EG2. *"Do it properly right away. But bear in mind the world must support streaming."* Both halves bind. ⚠ The ballot's own `RequiredCtor`-is-unbakeable inference is **refuted**: a GK-4 baker is a Rust program linked against the derive tables and calls the fn pointer trivially (measured through today's public `required_ctor_in_set`); only the *evaluator over text* cannot | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline |
| **F3** | owner | **ONE schema for both file shapes.** The table file is a spelling, not a second type system. Rejected: a table dialect | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables |
| **F5** | owner | **EVERYTHING FROM THE START — option (b), the one nobody had written down.** *"Все сразу грамотно по списку с самого начала."* `load_cell`/`unload_cell`, GK-1's cross-load map with a declared lifetime, and cross-cell reference resolution ship **with** the scene profile. ⚠ The single largest change to the campaign's ground: **G6 absorbs the hardest half**, because a per-load `LoadEntityMap` cannot express references BETWEEN chunks | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Streaming scope; [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) rows G6 and G8 |
| **F6** | owner | **Migrate `.ui` to the Gaia `ui` profile and DELETE the old format in the same campaign.** Price accepted and recorded at G7: the migration is done **blind** | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape |
| **F7** | owner | **Mods are NOT supported for now**, ratified explicitly — the ballot existed because silence was itself a decision. The reflection constraint is recorded in its **narrow** form (no reflection in the game binary), so a later mod campaign is not blocked by a sentence that never meant to block it | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Refusals |
| **GB-3** | owner (adoption) + delegated (the rewrite) | **The third reference kind is ADOPTED**, and the four-part rewrite is ruled: sigil / styles-are-kind-1 / `$hole` stays / "declared node ⇒ `@`" withdrawn. ⚠ The glyph itself is **not** minted — it cannot be chosen before Gaia's arithmetic operator vocabulary is closed | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Identity and references |
| **F4** | delegated | **(a), as SUPPRESS-THEN-FIXUP** — four ordered sub-passes on the clone path's own precedent. (b)'s mechanism adopted as sub-pass 4; its timing refuted at a measured 100% mis-link rate. Carries the stable-asset-carrier addendum | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Load semantics |
| **F2** | delegated | **(a) rows are entities — AMENDED**: `StorageKind::Table` (not dense), eager at load, own explicit load and pinned. (b) saves 0.13 MB and costs the entire per-`ResourceId` serialize seam from scratch | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables |
| **F9** | delegated, **partly** | **Three eliminations RULED** — do not fire `FLAGS_DIRECT` on load; the refusal option contradicts ratified **AB-6**; the spelling, if a carrier lands, is `flags (X = true)` shared verbatim with Aether. ⚠ **The residual is a VALUES call and is escalated back:** may a document set a flag on an *individual authored object*? | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Flags in the byte format |
| **GB-4** | delegated | **(a) linkage-in-slot, and the linkage word is MANDATORY**: `instance <name> extends|copy <base-ref>`. `from` deleted (head-position occurrences measured: **zero**) | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The instance spelling |
| **GB-5** | **owner asked for ANALYSIS** | ⚠ **STAYS OPEN.** The analysis is attached to the ballot body below. It settles one thing only, and it is a refutation of the ballot's own mechanism: **all 12 measured engine-derived field members are *conditionally* derived**, and both dispositions are pinned by green committed tests today | §2026-08-29, the GB-5 body |
| **GB-6** | **owner asked for ANALYSIS** | ⚠ **STAYS OPEN.** Analysis attached below: (a) the disposition is already declarative in four documents with four downstream reassignments filed against it, and its rejected alternative is **its own only witness**; (b) the valve's runtime already ships three times over and the whole gap is **one Aether construct** | §2026-08-29, the GB-6 body |

## 2026-08-29 — Corpus audit of the aether-v2 + gaia plans: THIRTY-TWO open ballots, listed here because a plan that settles a fork silently is the defect

A multi-lens review of the two plan corpora ([`aether-v2/`](aether-v2/CAMPAIGN.md),
[`gaia/`](gaia/CAMPAIGN.md)) returned 57 adjudicated edits. The determinate ones — false engine
claims, phantom artifacts, gates that cannot fail — are written into the documents themselves. What
an edit cannot do is decide a fork, and the review found the corpus quietly deciding them: a
recommendation printed as if ratified, an alternative never named, a refusal re-grounded in passing.
Every such fork is below, with what it blocks.

**Nine of them REOPEN something already ratified** — F7, F8, GB-3, AB-1, AB-6, AB-7, AB-10 (the
PENDING Tier 1 AIR-10 widening), AB-11, and AB-13's rename option. Those nine carry ⚠ and name what
they touch; no other item on this list does, so the marks and the count check each other. None may
be settled by an edit; where a reopen is licensed it is because a measurement refuted the original
premise, and that is said at the item.

**Nothing here blocks R0, R1 or R2 on the Aether ladder** — those are buildable immediately (KERNEL-BACKLOG **KE11**'s
red tests land regardless of its disposition). Everything above them waits on a line from this list.

> ✅ **STATUS, 2026-08-30 — read this before any ballot body below.** The bodies in this section are
> kept verbatim as the record of what was open and why; **they are not a to-do list any more.**
> **Every `AB` ballot except AB-12 is now closed**, along with `GB-1`, `GB-2`, `GB-7`, `GB-8` and
> `GB-9`. Seven were answered by the owner and eleven by the orchestrator under the standing
> perf/architecture rule. The index with each ruling and where its ground lives is
> §*2026-08-30 — the owner's rulings, and the Aether ladder closes*, above.
>
> ✅ **UPDATED LATER THE SAME DAY — the Gaia fourteen were put to the owner and TWELVE are
> answered.** Settled by him: **F1, F3, F5, F6, F7** and **GB-3**'s adoption. Delegated and now
> ruled: **F2, F4, GB-4**, and **F9 in part**. Sent back for **analysis, not decision**: **GB-5**
> and **GB-6** — both stay OPEN, and the analysis is attached to each body below.
> **Still open and genuinely awaiting an answer: `F8`, `F10`, `GB-5`, `GB-6`, and F9's residual
> VALUES question** (may a document set a flag on an individual authored object). Plus **AB-12**,
> which blocks nothing. The per-ballot index is the table in
> §*2026-08-30* above, subsection *The Gaia side, same day*.

### Gaia — the `F` series (F1..F7 raised 2026-08-28, full bodies in the entry below; F8..F10 born in this review)

- **F1 — the bake route.** EG2 reflection seam (already rejected by the 2026-08-27 audit ballot) vs
  macro-time GK-4 now with EG2 as a later upgrade. The real question is SEQUENCING. Blocks **G1**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — macro-time GK-4, NOW, with streaming not foreclosed.**
  Ground: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline.
- **F2 — where a DataAsset's rows live at runtime.** Entity-shaped dense columns with generated row
  constants, vs a new resource region in the byte format. Blocks **G5**.
  ✅ **RESOLVED 2026-08-30 [delegated] — (a) rows are entities, AMENDED**: `StorageKind::Table` not
  dense, eager at load, own explicit load and pinned. Ground:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables.
- **F3 — the shape of a table file.** Single-file-per-asset only, vs also a table file baking N rows
  into one dense column — over ONE schema either way. Blocks the **grammar** (G5's authoring
  surface).
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — both shapes, ONE schema.** Ground:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables.
- **F4 — what "loaded" means, WIDENED** to every insert-path mechanism the load path misses (hooks,
  the `#[require]` closure, flag initial state, relation reverse indexes, asset refcounts). Blocks
  **G6** and the engine's load semantics generally; carries the G6 stable-asset-carrier addendum.
  ✅ **RESOLVED 2026-08-30 [delegated] — (a), as SUPPRESS-THEN-FIXUP: four ordered sub-passes.**
  Ground: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Load semantics.
- **F5 — streaming scope.** Catalog now / loader later, vs the whole streaming half inside G6. The
  second option had never been written down. Blocks **G6**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — option (b), everything from the start.** Ground:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Streaming scope; G6's scope is rewritten accordingly.
- **F6 — the fate of `.ui`.** Migrate and delete in the same campaign, vs freeze until an owner-eval
  of a real Gaia HUD. Blocks **G7**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a), migrate and delete in the same campaign.** Ground:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape.
- **F7 — mods.** ⚠ Borders the ratified reflection-only-at-bake refusal; must be framed against it
  rather than asked fresh. Blocks nothing today; silence is itself a decision.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — mods are NOT supported for now, ratified explicitly**, and
  the reflection constraint is recorded in its narrow form. Ground:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Refusals.
- **F8 — name-vs-id for objects.** Does an authored object carry a human NAME that bakes to an id,
  or a minted ID with the name as commentary? ⚠ **REOPENS the ratified `gaia fmt --assign-ids`
  identity ruling** — which was written before the third reference kind (assets, GB-3) surfaced.
  AIR-10's bar applies: a reopen states its TRIGGER and its MEASUREMENT, it does not re-argue taste.
  Blocks: **TBD — owner scoping** (G3 is the candidate).
  ⚠ **STILL OPEN, and deliberately untouched by every ruling of 2026-08-30.** GB-3's Part 4 and
  F4's carrier addendum are both **F8-neutral by construction** and say so at their own sites: a
  *carrier form* (a stable name used as a key) is not a *name-vs-id ruling*.
- **F9 — the flags carrier.** Where a document's `flag` lands in the byte format: a carrier of its
  own, or a bake-time refusal with a `GA####` code telling the author to spell it as a component.
  Blocks **G2/G6**.
  ⚠ **PARTLY RESOLVED 2026-08-30 [delegated] — three eliminations ruled, the residual escalated
  back.** RULED: (1) `FLAGS_DIRECT` must **not** be fired on the load path — it would overwrite
  every saved bit with its attach-time value; (2) the refusal option **may not be chosen**, because
  it contradicts ratified **AB-6** (*a language may not refuse what the derive accepts*); (3) the
  spelling, if a carrier lands, is `flags (X = true)` shared verbatim with Aether, keyed on GK-1's
  global object id, riding F4's fixup pass. ⚠ **STILL THE OWNER'S:** may a document set a flag on
  an **individual authored object**? That is a VALUES call about the authoring surface; it blocks
  **G2** and nothing else. Ground and the full price of a carrier:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Flags in the byte format.
- **F10 — bundles: expand or refuse.** Does the baker expand a bundle into its components at bake
  time, or refuse a bundle in a document and demand the components? **Pre-question: is this a ballot
  at all** — PENDING's Tier 3 and its Part D contradict each other on whether it was already
  decided. Blocks: **TBD — owner scoping**.
  ⚠ **STILL OPEN, and deliberately untouched.** Every ruling of 2026-08-30 that could have brushed
  it — GB-1's depth rule, GB-3's Part 4, GB-4's instance form — states its F10-neutrality at its own
  site rather than settling it by proximity.

### Gaia — the `GB` series (born in this review)

- **GB-1** — which layer of the priority ladder a template EXPANSION and an UNLABELED write occupy.
  Blocks **G4**.

  > **RESOLVED 2026-08-30 — decided by standing rule** (`CLAUDE.md`: perf and architecture forks are
  > decided without asking; only VALUES and SCOPE go to the owner). This ballot was mis-routed here;
  > the owner re-routed it back. **Neither case gets a ladder layer, because the ladder is the wrong
  > axis for one of them.**
  >
  > **The ratified ordering, quoted exactly** ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The logic
  > line): *"defaults + a **closed priority ladder** `base < variant < tuning < debug` (named layers,
  > never free numbers — the `!important` race; two writes of one field on one layer = error with
  > both files; bake is order-independent and byte-identical in any file order)"*. Nothing below
  > disturbs it: no rung is added, removed, renamed or reordered.
  >
  > **(a) A template EXPANSION occupies no ladder layer. It is one step on the INHERITANCE-DEPTH
  > axis, which is already ratified and already separate.** The corpus carries depth as its own
  > mechanism in three places — *"Single parent, no diamonds"*, *"Variant chains: a patch target
  > resolves against the **immediate** base — one sentence plus one pinned test"*, and the bake
  > budget over *"expansion steps, spawned fields, **template depth**, output bytes"*. A template
  > application ranks exactly where `extends` ranks: below the applying node's own writes, at the
  > same layer. Resolution order is **depth first, then ladder**: within one layer a node's own write
  > beats what it expanded; across layers the flattened node from the lower layer loses field-wise to
  > the higher one.
  >
  > **The decisive ground is that the alternative is not expressible.** The rejected option is to
  > mint a rung for expansion (`template < base < …`). Its price is not taste: **`template depth` is
  > a BUDGETED quantity in the ratified text, and a budget bounds a number that is otherwise
  > unbounded.** A template that applies a template needs one rung per level; a closed four-name
  > enum cannot carry an unbounded count. The rung would also break the ladder's own closure, which
  > AIR-10's bar protects. So the option fails twice — on the ratified text and on arithmetic — and
  > this is why the ruling routes expansion to the axis that is already unbounded.
  >
  > **This dissolves K2 without new machinery.** K2 is *"template application double-writes a field
  > on one unnamed ladder rung"*: `apply Torch` plus a local `power=2.5` in the same node looked like
  > the ratified *two-writes-one-layer = error*, which would have made the commonest authoring act in
  > the language — apply a template, override one field — a bake error. Under the ruling they sit at
  > different DEPTHS, so it is an override, not a collision. The error keeps a real referent and
  > narrows to what is genuinely ambiguous: **same depth AND same layer.**
  >
  > **(b) An UNLABELED write occupies `base`.** Ground: `base` is the layer whose name already means
  > "the thing itself", and it is the ladder's bottom, so authored content stays overridable by every
  > pack above it. The rejected alternative is an implicit fifth "unlabeled" rung, and its price is
  > the exact race the ladder was ratified to prevent — an unranked write makes *"which write won"* a
  > computation over file order, which would make G4's own gate (`bake(files) == bake(shuffle(files))`
  > byte-identical) either red or, worse, vacuously green on a fixture that happens not to collide.
  >
  > **Recorded, NOT decided here — the consequence (b) buys.** In a non-`base` pack every line must
  > repeat `layer=tuning`, and one forgotten `layer=` lands silently at `base`: a silent wrong-layer
  > write, which is this repo's standing defect class. A file-level layer default is the obvious cure
  > and is a **grammar** question belonging to **G4**, not to this ballot; naming it here so it is not
  > lost, and deliberately not ruling it.
  >
  > **Spelling-independent, and deliberately so.** The ruling is about precedence, not words. It
  > holds under either option of **GB-4** (`instance … extends|copy` in slot vs in head) and under
  > whatever anchor the template-application fix lands on (PENDING Tier 1 proposes `apply Torch:`).
  > It is scoped to **template** expansion, the construct GB-1 names. Whether a **bundle** expands at
  > bake at all is **F10**, which is the owner's and is untouched; if F10 rules "expand", the depth
  > rule above applies to it unchanged, and that is a consequence, not a pre-decision.
  >
  > **Riding lines moved in the same edit:** [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The logic line
  > and §Inheritance / variants, [`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md)
  > Tier 1 (template use), Part D and Part E (K2),
  > [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) §Work order,
  > and [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md).

- **GB-2** — the colour transfer function: `#RRGGBBAA` sRGB-decoded at bake, vs raw bytes carried
  into a LINEAR field. Blocks **G2/G5**.

  > **RESOLVED 2026-08-30 — decided by standing rule.** This is a correctness question with a
  > measured answer, and the answer is **neither of the two options as posed**: both assume ONE rule
  > for the literal. **The transfer function is a property of the DESTINATION FIELD, not of the
  > literal**, and it is declared in the GK-4 field table.
  >
  > **What is true at source.** Every colour-typed field in the workspace was read, not inferred.
  > There are three carrier classes and they disagree with each other:
  >
  > | carrier | fields | what the source says |
  > |---|---|---|
  > | LINEAR float | `PointLight::color` (`boyko_render/src/light.rs:309-310`), `DirectionalLight::color` (`:289-290`), `SpotLight::color` (`:336-337`), `SkyLight::sky_color`/`ground_color` (`:357-360`), `MaterialGpu::base_color`/`emissive` (`material.rs:51,54,64-69`) | `/// LINEAR rgb color`; `light.rs:114` *"All radiometric values are LINEAR"*; `material.rs:56` *"All values are LINEAR"* |
  > | 8-bit encoded RGBA | `UiBackground::color`/`border_color` (`boyko_ui/src/components.rs:211,220-223`), `UiText::color` (`text/components.rs:39-47`) | `u32`, *"authored STRAIGHT RGBA8 (`byte0=R .. byte3=A`)"* — **"straight" here means NON-PREMULTIPLIED, not "not decoded"**: `components.rs:211` continues *"the pack system premultiplies them"*, and `premultiply_rgba8` (`boyko_render/src/ui/instance.rs:196`) is the operation named. No sRGB decode exists anywhere on the UI path — `ui_rect.fs.hlsl:130` unpacks the byte word and composites it directly |
  > | device-encoded packed | `ParticleEffect::color_keys: [u32; 4]` (`boyko_render/src/particle_effect.rs:120-143`) | byte order is **`0xAABBGGRR`**, *"the opposite of the `0xRRGGBBAA` an author reaches for by habit"* — and its own doc records that both in-tree presets were authored wrong exactly that way, *"and neither was caught by anything"* |
  >
  > That table settles the ratified-text worry before it starts: the ratified *"`#RRGGBBAA` →
  > straight-RGBA8"* line ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape) is a statement
  > about **alpha association**, not about a transfer function. It is not being reopened; it is being
  > read against the field whose doc comment defines the word.
  >
  > **The ruling.**
  > 1. **Destination is a LINEAR float colour field → the sRGB EOTF is applied at bake.** The
  >    authored byte is display-referred; the field is scene-referred; a converter that skips the
  >    conversion is simply wrong.
  > 2. **Destination is an 8-bit encoded RGBA carrier → the transfer function is the IDENTITY.** Not
  >    a special case bolted on: source space and destination space are the same space, so the
  >    general rule yields the identity here. **Measured:** decode-then-re-encode over the whole
  >    domain drifts **0 of 256 bytes**, so stating the rule uniformly changes no existing UI colour.
  > 3. **Destination is a device-encoded packed carrier → a coded `GA####` bake refusal**, naming the
  >    field's own byte order. Gaia must not be the fourth way to author `0xAABBGGRR` by habit.
  >
  > **Rejected alternative, and its price — measured, not asserted.** Carrying raw bytes into a
  > LINEAR field (the ballot's option (b), and what
  > [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md) does today) is wrong by up to **12.92×** multiplicatively
  > (byte 3: `0.011765` raw vs `0.000911` decoded) and **0.287** absolutely (byte 136). On the
  > corpus's own literal `#FFB35CFF` into `PointLight::color` it is **1.557× on green** (`0.702` vs
  > `0.451`) and **3.371× on blue** (`0.361` vs `0.107`) — a large hue shift toward desaturation,
  > not a brightness nudge, and nothing downstream attributes it to the baker. The mirror-image
  > alternative — one global "always decode" rule — is wrong in the other direction: applied to a
  > `u32` field by storing the decoded value it corrupts every UI colour, which is why the rule is
  > keyed on the destination rather than on the literal.
  >
  > **⚠ A gate constraint that falls out of the same measurement, and must be obeyed by G2's
  > fixtures: the two routes have exactly TWO fixed points, byte 0 and byte 255.** A red fixture
  > written with `#FFFFFFFF` or `#000000FF` is satisfied identically by both routes — **a gate that
  > cannot fail**, the class this corpus exists to repair. Every colour fixture must use a
  > mid-domain channel; `128` (raw `0.501961` vs decoded `0.215861`) is the recommended witness.
  >
  > **The ARITY half — settled here, and it needed settling because nothing carried it.**
  > `LANGUAGE.md:71-74` flags a 4-component `#RRGGBBAA` written into a 3-component `[f32; 3]` and
  > records that *"the arity half is carried by nothing"*. **The precedent is already shipped in this
  > repo**: Aether's `ColorLit` (`crates/aether_lang/src/parse.rs:1177-1213`) checks arity against
  > the TARGET's arity — 3 or 4 for `base`, exactly 3 for `emissive` — synthesizes the missing alpha
  > as `1.0` when widening (`expand.rs:891-898`), refuses when narrowing with a message naming the
  > target type (*"`Material::new` takes `emissive: [f32; 3]`, emitted radiance has no alpha"*), and
  > lands the blame on the **tuple's own span**, *"because neither the key nor any single component is
  > the thing that is wrong"*. Gaia mirrors that rule verbatim: **widening 3 → 4 supplies alpha
  > `1.0`; narrowing 4 → 3 is a coded refusal, never a silent alpha drop** (a silent drop is Unity's
  > class, which §Inheritance bans by name). Consequence: the 6-digit **`#RRGGBB`** form is admitted
  > as the 3-component spelling, and `LANGUAGE.md`'s `color=#FFB35CFF` on a `PointLight` is a bake
  > error whose correct form is `#FFB35C`.
  >
  > **Why admitting `#RRGGBB` is not a reopen.** §Language shape's literal list is introduced as
  > *"Literals are engine-native (…)"* — an illustrative list. The one list in this corpus that is
  > closed says so in as many words (§The logic line: *"the list is exhaustive"*), and this is not
  > that list. The alternative to admitting the 6-digit form is to make every light colour a float
  > tuple, which prices the commonest scene edit in the least readable form for no correctness gain.
  >
  > **Not touched:** whether `PointLight::position` is declarable at all is **GB-5**, the owner's,
  > and nothing above bears on it.
  >
  > **Riding lines moved in the same edit:** [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language
  > shape, [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md), [`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md)
  > Tier 1 (`PointLight` field names), Tier 4, Part D and Part E (K12),
  > [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) §Work order,
  > and [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md).
- **GB-3** — the reference taxonomy needs a THIRD kind. Ratified §Identity has two (lexical/copy,
  entity identity via `@` with remap); asset references are neither. On one ballot because R4 should
  be rewritten once: the asset-ref spelling (sigil / typed head / bare strings — the bare-string
  option needs its own dangle-check answer), the style-reference kind, the `$hole` sigil
  disposition, and withdrawal of the "declared node ⇒ `@`" rule. ⚠ **amends ratified §Identity** —
  an extension consistent with its own one-spelling-one-behaviour rationale, not a reversal. Blocks
  **G2/G3** and PENDING Tiers 1-2.

  > ✅ **RESOLVED 2026-08-30 — the THIRD KIND IS ADOPTED BY THE OWNER; the four-part rewrite ruled
  > the same day [delegated].** Full ruling: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Identity and
  > references. Summary, with the measured ground and the rejected alternative at each part:
  >
  > **The kind is real, and the deciding column is one the other two do not have.** An asset
  > reference is the only kind whose referent can **stop being valid after load**, because streaming
  > retires slots — and under the owner's F5 ruling that is the normal case. At source:
  > `PathIndex::lookup(hash: u64) -> Option<(u32, u32)>` = `(slot, generation)`
  > (`crates/boyko_ecs/src/ecs/core/asset/path_index.rs:112`), then refcounted and revalidated every
  > frame by `validate_asset_refs` against the `MeshRefGen`/`MaterialRefGen` lanes. Both collapses
  > are refuted by that: there is no `Entity` and no load-map row (and `PathIndex` is globally
  > interned, first-insert-wins, **not** per-load), and a lexical reference does not survive into the
  > binary at all while this one must. ⚠ It is also the **only reference kind that already works
  > across a cell boundary** — two cells referencing one path share `(slot, generation)` and the
  > refcount keeps it alive while either holds it — so folding assets into `@` would give the
  > working kind the lifetime of the one F5 just made hard.
  >
  > **Part 1 — spelling: (a) A SIGIL**, uniform and position-independent, buying the invariant *a
  > bare word is never a reference*. **Rejected (c) bare strings**, on this ballot's own ratified
  > rationale: a string literal is already a **non-reference value** in this language, so (c)
  > collides kind 3 with plain values, not merely with another reference kind — and its dangle check
  > needs **three** mechanisms (field table, head grammar, closed valve signature) because asset
  > references occur at positions with no destination field, which is three places the check can be
  > structurally unreachable. **Rejected (b) a typed head**, on the measured ground that already
  > killed widget sugar (*the vocabulary is unclosable*) and on R7's requirement that the operation
  > list be closed and generated. ⚠ **The glyph is NOT minted here, and that is a finding.** Measured
  > over Gaia code fences (44 fenced lines, 4 files): `/` 21, `@` 10, `:` 4, `#` 3, `|` 2, `-` 2,
  > `$` 1, `;` 1, `+` 1, `>` 1; absent `!  %  &  *  <  ?  \  ^  ` ~`. Excluded: `@`/`#`/`$` (taken),
  > `&` (ratified-rejected, and reads as *borrow* for an owned refcounted handle), `?` (reads as
  > fallible, which contradicts §Refusals' *no construct with a fallback*), backtick
  > (measured-hostile in this toolchain), **and any glyph that can open a binary operator** — R3
  > makes whitespace insignificant, so `(a %b)` and `(a % b)` must parse identically and `%` is what
  > the ratified `for … if` guard wants. **Therefore the sigil cannot be chosen before Gaia's
  > arithmetic operator vocabulary is closed; the two are one question and G2 closes them together.**
  > Recommendation `~`. The dangle check is named in the ruling: one lexical pass over sigil-marked
  > tokens, a coded `GA####` refusal naming pack and path, blamed on the reference's own span.
  >
  > **Part 2 — style references are the FIRST kind (lexical/copy).** §Refusals already ratifies *no
  > cascade — styles are flattened by bake*, and a thing flattened by bake leaves no reference in the
  > binary. ⚠ **A contradiction inside the ballot's own text is resolved rather than carried
  > forward:** it listed *"a style record"* among the asset kind's examples, against §Refusals on the
  > same page. §Refusals is ratified and the parenthetical was not; the example list **drops** it.
  >
  > **Part 3 — `$hole` STAYS, and the ballot's premise is false.** It is not a third *kind*: §Identity
  > already names templates and `let` as the lexical kind, so `$power` was ratified all along. What
  > `$` does is stop a real ambiguity: R6's case-gate was refuted (M12), and at value position the
  > grammar admits bare lowercase words that are **not** references (RON-style enums, the ladder
  > names, `rarity=common`), so a template parameter named `common` would silently flip
  > `rarity=common` from an enum value into a substitution. Ruled: `$` marks the lexical kind **in
  > every position**, including `extends $sword_base` and `apply $Torch`, which is what lets a
  > generator pick the sigil from the kind alone. Price: one character per reference.
  >
  > **Part 4 — "declared node ⇒ `@`" is WITHDRAWN**, replaced by *what does bake do with this
  > reference?* — substitutes it (`$`), records an object id for remap (`@`), or records a path hash
  > for lookup + refcount (the new glyph). The corpus already shows the withdrawn rule's damage:
  > `LANGUAGE.md` spells a **lexical** inheritance base as `extends @iron_sword`.
  >
  > **The ballot's second item is REFUTED, not answered.** It asserted *"GN1 resolves every reference
  > form … WITHOUT remap. That is the asset kind's behaviour."* GN1's own four resolutions open with
  > *"object id → `Entity` (**load remap**)"*, so GN1 spans a remapping resolution. GN1 is the uniform
  > law over all three kinds; **only kind 3 is lookup-without-remap.**
  >
  > ⚠ **Two findings recorded, neither ruled here** (each is a different ballot's or rung's):
  > (1) **`@asset/object` is ambiguous under the `/` separator** — every asset path in a Gaia fence
  > has ≥2 slash-separated segments (**8 of 8**), so `@a/b/c` cannot be split without an out-of-band
  > rule; the one ratified example reads unambiguously only because it uses a 1-segment asset name, a
  > shape occurring nowhere else. R4's amendment already flags the `/`-vs-`#` drift; the measurement
  > says only `#` survives contact. (2) **The asset-ref target registry does not exist** —
  > `grep -rnE "get_by_name|by_path|load_path|handle_for_name|name_to_handle"` over the asset and
  > render crates returns **0**. That is a build item for **G3**, not a spelling item.
  >
  > **Riding lines this ruling OWES and does not make:** PENDING R4/R5/R7 and Tiers 1–2 are
  > regenerated by it, and `LANGUAGE.md` still carries `extends @iron_sword` and bare-string asset
  > refs. Both files are already marked — PENDING as owner-gated, `LANGUAGE.md` as `ratified-stale`
  > in its own head — so the divergence is *marked*, not silent. The edits belong to **G2**.
- **GB-4** — instance re-spelling: (a) linkage-in-slot (`instance wall_east extends|copy "…"`) vs
  (b) linkage-in-head (`instance` ≡ live link, `copy` its own head). `from` is deleted either way —
  it was minted by that one line and appears in no ratified vocabulary. Blocks **G4** / Tier 2.

  > ✅ **RESOLVED 2026-08-30 [delegated] — (a) linkage-in-slot, with the linkage word MANDATORY.**
  > Full ruling: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The instance spelling.
  > `instance <name> extends|copy <base-ref>`, no default linkage, `from` deleted.
  >
  > ⚠ **How the owner's word was read, said plainly rather than assumed.** He answered *"давай"* —
  > **delegation, not a selection**: he named no option, and the ballot text carries no recommendation
  > for the word to point at. Nothing in the evidence makes one option obviously his intent either,
  > so it is decided under the standing perf/architecture rule and recorded as the orchestrator's
  > call, not attributed to him.
  >
  > **Four grounds.** (1) **(b) mints two grammars for one ratified construct** — this corpus already
  > ratifies *"Instance = variant = derived asset = one construct"*, and **AIR-15** rejects
  > *"two grammars per construct — the `at` lesson"* by name, on the AI-orientation axis this ballot
  > was to be judged on. (2) **(b) turns a modifier change into an anchor mutation**: under (a),
  > "make `wall_east` a snapshot" rewrites one token in a fixed slot and every patch/diff/tool edit
  > keyed on *the `instance` node named `wall_east`* still resolves; under (b) it rewrites the
  > **head**, which is neither a key nor a value and so lies outside **AIR-09**'s ratified
  > `{node_id, key, value}` patch model. (3) **(b) has no refusal for the omitted case; (a) does** —
  > `instance …` alone is legal under (b) and means *live*, so forgetting the word yields a silent
  > wrong-linkage node, the identical consequence the ladder ruling already bought once for a
  > forgotten `layer=`; and defaulting is unavailable on the section's own ground, since RimWorld and
  > Factorio each shipped one form and their ecosystems invented the other. (4) **The two axes stay
  > orthogonal**: the data profile already spells a linkage word in a slot on `row`, so under (b)
  > `copy` is a *head* in one profile while `extends` is a *slot word* in the other — the "one word,
  > N positions" class the syntax plan catalogues against Aether — and R7's closed operation list
  > grows per construct × linkage under (b) versus by two entries under (a).
  >
  > **Rejected, with its price stated rather than dismissed.** (b) is genuinely cheaper to read in the
  > common case and maximally loud on scan; **(a)'s price is one mandatory word on every instance
  > line.** The trade is taken because on the generator axis (a) has **no silent failure** — both
  > wrong answers are visible, the token is there and it is the other one — while (b) has exactly
  > one, and it is the omission case, the commonest generator error.
  >
  > **`from` — measured, and the qualification strengthens the deletion.** A raw grep counts prose, so
  > the measurement is over **code fences only**: extract fenced blocks from `docs/gaia/*.md` and
  > match `\bfrom\b` → **1 hit in 44 fenced lines across 4 files**, and it is
  > `bind value from=@player/unit …` — `from=` as a **field key**, not the head-position `from` being
  > deleted. Head-`from`'s count is therefore **zero**, and `grep -rn '"from"' crates/aether_lang/src/`
  > exits 1, so it is not an Aether keyword either. ⚠ **Rider:** PENDING's own R1 amendment spells
  > that construct `bind: value source=…`, so the corpus contains an undeclared `from=` → `source=`
  > rename. GB-4 deletes head-`from`; the surviving role must not be left half-renamed. That edit is
  > **owed to G2**.
- **GB-5** — may a scene document declare an ENGINE-DERIVED field (`PointLight.position`)? The `ui`
  profile already refuses this; the ballot is extending the rule to `scene` as a DECISIONS line
  BEFORE G1 freezes the GK-4 field table, which then marks such fields. Blocks the **G1 table
  freeze**.

  > ⚠ **STILL OPEN — 2026-08-30. The owner asked for the TRADE-OFFS, not a ruling, and a ruling
  > written here would be a defect.** What follows is the analysis, written so he can rule from one
  > reading. Nothing in it is a decision.
  >
  > **1. The ground at source.** `light_reconcile`
  > (`crates/boyko_render/src/light_reconcile.rs:105-125`) takes
  > `Query<(&GlobalTransform, Mut<PointLight>), Changed<GlobalTransform>>` and writes
  > `l.position = g.translation()` behind a bit-gate. `PointLight` carries `position`, `color`,
  > `power`, `range` and is `#[require(Transform, GlobalTransform)]`; its own doc says
  > `light_reconcile` *"derives `position` from the `GlobalTransform` translation when one is
  > present."*
  >
  > **⚠ What happens today is not one answer but TWO, and both are pinned by green committed tests.**
  > Run live (`cargo test -p boyko-render --test render_upload_s4`, `running 2 tests`, `EXIT=0`):
  >
  > | test (`crates/boyko_render/tests/render_upload_s4.rs`) | result | what it pins |
  > |---|---|---|
  > | `point_light_position_tracks_global_translation` (`:261`) | ok | authored `position` is **overwritten** by the transform translation — **the overwrite is a contract** |
  > | `light_without_global_transform_is_untouched` (`:336`) | ok | *"the authored pose is exactly preserved"* — **non-overwrite is equally a contract** |
  >
  > The concrete failure the ballot is about: an author writes `PointLight.position = (3,5,2)` and
  > omits `Transform`. `#[require]` supplies the defaults, the first frame's `Changed<GlobalTransform>`
  > matches, and the light silently renders **at the origin**. Nothing logs; the authored value
  > survives zero frames. ⚠ **One seam decides whether that reproduces for a BAKED scene, and it is
  > an engineering fork, not the owner's:** F4(ii) measured that the load path adds no component the
  > file omitted, so whether a baked `PointLight` is overwritten depends on whether the **baker's**
  > world construction fires `#[require]` — which is G1's own unwritten design. Flagged so it is not
  > settled by accident.
  >
  > **2. The `ui` profile does NOT already refuse this — measured, and this weakens the ballot's
  > premise three ways.** The rule exists at exactly one site,
  > [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md): *"Engine outputs (`ComputedRect`, `Interaction`, bitset
  > tags) are undeclarable — bake error."* (i) **It is not in the decision log** —
  > `grep -rn "ComputedRect\|Interaction" docs/gaia/DECISIONS.md docs/gaia/CAMPAIGN.md
  > docs/gaia/PENDING-SYNTAX-PLAN.md` → **zero hits** — and `LANGUAGE.md` is the corpus's standing
  > example of a file that resolves but is stale. (ii) **It gives no reasoning**: one sentence, three
  > examples, no ground, so there is nothing to extend *by argument*, only by repetition. (iii)
  > **All three named examples are WHOLE components; `PointLight.position` is a FIELD inside an
  > author-owned one** — refusing a component the author never wanted costs nothing, refusing one
  > field of a component whose other three they must write is a different trade. ⚠ And a name-shaped
  > reading of that rule is already wrong in-tree: `ComputedClip` is documented *"AUTHOR-OWNED in P1
  > (not computed)"* and `StackIndex` *"AUTHOR-OWNED in P1"* (`crates/boyko_ui/src/components.rs`),
  > so a rule keyed on the word "Computed" forbids two author-owned components on day one.
  >
  > **3. The class, enumerated — and it is not one field.** Two comment-proof commands over the four
  > scene-relevant crates (`boyko_scene`, `boyko_render`, `boyko_ui`, `boyko_physics`), re-run
  > 2026-08-30:
  > `grep -rEn "[ (,]Mut<[A-Z]|Query<[^>]*&mut [A-Z]" … | grep -vE ":[0-9]+:[[:space:]]*(//|/\*|\*)"`
  > → **17**; `grep -rEn "get_component_mut::<[A-Z]" … | grep -vE …` → **12**.
  > ⚠ **These two are a FLOOR, not a census**: they are different mechanisms (`ComputedRect` and
  > `Interaction` are written only through `get_component_mut`, so the first command misses them
  > entirely), and reading turned up a **third** — a `Command::apply` body, `SyncRefGenCommand`. No
  > single grep produces this class.
  >
  > **Shape A — whole engine-owned components (what the `ui` clause actually names): 14.**
  > `GlobalTransform` · `ComputedRect` · `Interaction` · `RelativeCursorPosition` ·
  > `UiWorldProjection` · `UiWorldCulled` / `UiWorldHidden` / `UiWorldOccluded` (three separate
  > owning authorities) · `InstanceModelCol` · `PrevInstanceModelCol` · `GpuTransform3D` ·
  > `Gpu3dInstance` · `MeshRefGen` · `MaterialRefGen`.
  >
  > **Shape B — engine-derived FIELDS inside author-owned components. This is GB-5's actual subject:
  > 12 members over 8 components, and EVERY ONE IS CONDITIONAL.**
  >
  > | field | overwritten iff | writer |
  > |---|---|---|
  > | `PointLight.position` | the entity has `GlobalTransform` | `light_reconcile` |
  > | `SpotLight.position`, `.direction` | same | `light_reconcile` |
  > | `DirectionalLight.direction` | same | `light_reconcile` |
  > | `Transform.translation`, `.rotation` (`.scale` **never**) | `Simulated` ON ∧ `inv_mass != 0` ∧ no `ChildOf` | `boyko_physics` `scene_sync.rs` |
  > | `Transform` (whole) | the entity has `OrbitCamera` | `boyko_scene` `camera.rs` |
  > | `Transform` (whole) | the entity has `FlyCamera` | `boyko_scene` `camera.rs` |
  > | `RigidBody.position`, `.rotation` | **opposite polarity**: `Simulated` OFF ∧ `inv_mass == 0` | `scene_sync.rs` |
  > | `ContentSize.width` / `.height` | the node has `UiText`+`UiTextBuffer` ∧ a font is loaded | `boyko_ui` `text/measure.rs` |
  > | `UiLayout.width` **or** `.height` (axis picked at runtime from the track's `LayoutType`) | the node is a `Bar`'s fill child | `boyko_ui` `widgets.rs` |
  > | `UiValue.0` | a `BindValue` record targets it | `binding/bind_system.rs` |
  > | `UiTextBuffer` | a `BindText` record targets it | `binding/bind_system.rs` |
  >
  > **The finding that governs any ruling: "engine-derived" is not a property of a FIELD.** It is a
  > property of *(field × the entity's other components, and for physics and `Bar` × their runtime
  > values)*. Three consequences: **`Transform` is in the class**, so a blanket per-field rule applied
  > honestly refuses `Transform.translation` on every entity; **`Transform` and `RigidBody` are a
  > polarity pair** whose author-owned side flips on `Simulated`, a bitset bit gameplay toggles at
  > runtime, which bake cannot decide; and **both green tests above are correct**, so any rule saying
  > "`PointLight.position` is engine-derived, full stop" contradicts a test that passes today.
  >
  > **4. Why the timing matters, and the honest price is not the ballot's.**
  > `grep -rn "field_table\|FieldTable\|field_by_name" crates/boyko_macros/src crates/boyko_ecs/src`
  > → **empty**: G1 is unstarted, so "before the freeze" means *before the table's shape is designed*.
  > **Bytes are indifferent** — G1's gate is `bake_one(...) == save_world(...)` byte-for-byte, and a
  > refusal is a bake-time diagnostic that changes no bytes, so **lateness does not turn a gate red**.
  > The real prices: **cheap before** — one attribute at the component definition site, one column in
  > the derive-emitted table, zero consumers to update; **expensive after** — the disposition has to
  > live in a hand-maintained side list keyed by (component, field), an object this corpus has already
  > priced (*four hand-maintained lists of one vocabulary* as the single cause of `.ui`'s measured
  > defects, and a printer losing 10 of 19 components under a gate structurally blind to the loss),
  > plus reopening a landed G1 under AIR-10's bar.
  > ⚠ **The same measurement cuts the other way, and this is the strongest argument for deciding
  > CAREFULLY rather than EARLY:** a per-field marker in the frozen table is the shape the ballot
  > assumes, and it is **wrong for 12 of 12 measured members**, because not one is unconditional.
  > Freezing a per-field boolean early locks a predicate that is false for every case it covers —
  > worse than deciding late.
  >
  > **5. The options, each in its strongest form.**
  > - **(a) REFUSE — a bake error on any write to a marked field.** *Strongest case:* it kills the
  >   silent wrong answer at authoring time, this repo's standing preference, reaffirmed by the owner
  >   on this very campaign — GB-2's alpha-drop ruling, whose recorded ground is *a refusal that
  >   dissolves a class beats a documented sharp edge*. *Price:* it needs a decidable per-field
  >   predicate and the measurement says none exists; applied to `Transform.translation` it refuses
  >   the commonest write in the language, and applied only to lights it is a rule for four fields
  >   pretending to be a class.
  > - **(b) PERMIT — the field is writable; the engine discards it.** *Strongest case:* no new
  >   machinery, no over-refusal, and no wrong predicate frozen into the table; it also keeps Aether's
  >   own emitted authored scenes legal, since the engine already writes a light pose and lets
  >   `light_reconcile` re-derive it. *Price:* the light-at-the-origin failure, silent, in the language
  >   whose whole §Refusals section exists to make such things inexpressible.
  > - **(c) PERMIT-AS-SEED — authorable, documented as a seed, warned about at bake.** *Strongest
  >   case:* it says what is true, and it is the engine's own vocabulary already (*"`SpotLight::new`'s
  >   `direction` is only a SEED"*). *Price:* a warning is not a gate, and Gaia's ratified evaluator
  >   is **closed** — *anything bake cannot fully resolve is a bake error, never a fallback* — so a
  >   warning-only disposition is the first fallback in a language that has none.
  > - **(d) REFUSE CONDITIONALLY — refuse iff the deriving sibling is on the same flattened entity.**
  >   *Strongest case:* it is **the only option consistent with both green tests**, and it is decidable
  >   at bake for every presence-conditioned member (**7 of 12** — the four light fields, the two
  >   camera cases, `ContentSize`), because the baker knows the entity's final column set. *Price:*
  >   the predicate runs over the *flattened* entity, so it is a **G4**-time check while the write is a
  >   G2/G3-time act — the blame span is a write in file A while the disqualifying sibling arrives from
  >   a template in file B; and it is **undecidable for the 5 value-conditioned members** (physics
  >   `Simulated`/`inv_mass`, `UiLayout` under `Bar`), which must then take (b) or (c) anyway, giving
  >   the language two rules for one class.
  >
  > **Where the evidence points, and where it stops.** It points hard at **rejecting the
  > per-field-flag mechanism the ballot assumes** — that is a measurement, not a preference: 12 of 12
  > members are conditional. It does **not** pick between (b), (c) and (d); that is a values call
  > about how much authoring expressiveness a refusal may cost.
  >
  > **⇒ The question, in one sentence.** Given that every engine-derived field measured is
  > *conditionally* derived — including `Transform.translation`, which is engine-owned only for a
  > simulated dynamic body — should the scene profile **refuse** such a write when the deriving
  > sibling is present on the same baked entity (accepting a post-composition blame span and a second
  > rule for the five cases bake cannot decide), or **permit** it and document the discard?
- **GB-6** — §Relations carries two unclassified "ratify" imperatives: the scene-form fork (blocks
  **G6** if it is a ballot; if delegated, it must be relabelled "decided, owner may veto"), and the
  EnableTag-toggling visibility valve, which carries a DEADLINE ("before the first designer asks")
  and therefore needs an F-id and a rung on the Aether ladder — **a deadline with no rung can never
  come due**. Recorded beside it: the two authored-scene emission fixes ride no rung at all.

  > ⚠ **STILL OPEN — 2026-08-30. The owner asked for the ANALYSIS, not a ruling.** Below is what was
  > measured; nothing in it is a decision.
  >
  > **(a) The scene-form fork — the evidence says it was already decided, and already acted on.**
  > The imperative sits in [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) §Relations. The disposition appears
  > in **four other documents, all in the declarative voice, none interrogative**:
  >
  > | site | text |
  > |---|---|
  > | [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) | in the **landed-changes** table: *"Aether's `scene` narrows to dev-bootstrap and the shipped world form moves to Gaia"*, with carriers cited |
  > | [`AETHER-LANG-PLAN.md`](AETHER-LANG-PLAN.md) §3.7 | *"`scene` keeps only a **dev-bootstrap** role"* |
  > | [`AETHER-V1-SURFACE-REVIEW.md`](AETHER-V1-SURFACE-REVIEW.md) | *"`scene` narrows to a dev-bootstrap role; the shipped world form moves to Gaia"* |
  > | [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md) §`scene` | *"`scene` keeps its narrowed authored-scene role, with the world moving to the baked asset format"* |
  >
  > **And downstream work has already been reassigned on its basis.**
  > [`AETHER-V1-SURFACE-REVIEW.md`](AETHER-V1-SURFACE-REVIEW.md) declares four v1 absences —
  > `N-14` (no node handle), `N-15` (no scene unload), `N-16` (mesh sources), `N-17` (no material
  > asset handle) — to be *"Gaia's ground now"*, each with a named carrier (AIR-08; ballot F5 / rung
  > G6; §Identity + the GN1 lint). **A question still open cannot have had four consequences filed
  > against it.**
  >
  > ⚠ **The one thing missing is the price of the rejected alternative.** Measured:
  > `grep -rn "SceneModel" docs/ crates/` returns **exactly one line in the whole repository** — the
  > very line asserting *"the alternative, a shared SceneModel, is priced higher"*. **The claim is its
  > own only witness.**
  >
  > **What changes under each classification.** *As a ballot:* G6 is formally blocked — but G6 is
  > already held by F4's unimplemented ruling and by F5's newly widened streaming scope, so **no
  > schedule moves**; what does change is that the four `N-14`…`N-17` reassignments become
  > provisional and two documents are left asserting a decision the owner has not made — the
  > diverged-pair cost this corpus already prices. *As delegated (relabel "decided, owner may
  > veto"):* one phrase changes on one line, the reassignments stand, and the veto becomes explicit
  > instead of implicit — `CAMPAIGN.md` already carries exactly this shape for first-class `remove`.
  > ⚠ *The cost of the in-between state is visible in the tree right now*: G7's row records the
  > symptom itself — *"it lists **GB-6** against G7, while GB-6's own body names **G6**; not resolved
  > here."* A line that says "ratify" with no register reads as owed; a rung whose row names no ballot
  > reads as unblocked. Both readings are live.
  >
  > **Rider that must travel with a relabel, either way:** a veto point presumes the owner can see
  > what he is declining to veto, and today he cannot, because "priced higher" cites no price.
  > Writing that price is one paragraph and is the only thing that makes the veto meaningful.
  >
  > **⇒ Question (GB-6a), one sentence.** The scene-form narrowing is stated declaratively in four
  > documents and has already had four downstream reassignments filed against it — do you want the
  > §Relations line relabelled **"decided, owner may veto"** (the `remove` shape), or is this
  > genuinely a ballot you want to answer, in which case those four reassignments revert to
  > provisional?
  >
  > **(b) The conditional-visibility valve — the mechanism exists; ONE construct does not.**
  > What the valve is for: a designer wants *"show this element when health < 30%"*, which Gaia
  > refuses by ratified rule (§UI bindings item 5, *"Structure bakes; only VALUES react … a
  > structural change is an explicit document/subtree swap via an action"*; §Refusals, *no structural
  > reactivity*, *no out-of-document conditions*). The sanctioned escape: the document names an
  > action; the action toggles an EnableTag; the layout pass skips the node.
  >
  > **Three of the four pieces already ship, measured.** (1) `flag` is a ratified Aether construct —
  > [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md), *"enable bit: O(1) toggle, takes
  > NOTHING"* — with `enabled`/`disabled` filter terms in the shipped v1 grammar. (2) **The engine
  > already runs this exact idiom in production, three times over**: `UiWorldCulled`,
  > `UiWorldHidden`, `UiWorldOccluded` (`crates/boyko_ui/src/world/components.rs`) are bitset tags
  > owned by three separate authorities, deliberately split so *"the three never race a shared
  > bit"*, and *"the layout pass skips a root with this bit set."* The valve's runtime is not
  > speculative; it is shipped, with a documented ownership discipline to copy. (3) Gaia's document
  > side is ratified already: *"A condition in a Gaia file is a **token reference, never an
  > expression** — that is the whole answer to how data references logic without becoming code"*.
  >
  > **The missing piece is exactly one thing: Aether has no construct that TOGGLES a flag.**
  > `CONSTRUCTS.md` has 16 construct sections; `flag` declares the bit, the filter grammar reads it,
  > and there is **no `action` construct at all** — the word "action" occurs once in that file, inside
  > a machine's drain-loop prose. A machine action block can write one, but `machine … on entity` is
  > **R5**, and R5 is blocked on owner ballot **AB-7**.
  >
  > **What rung could carry it.** **Aether R3 — the only honest candidate today**: it owns
  > `tag`/`flag`, it owns `system` and `set`, it is the rung CG-1/CG-2 are already nominated to, and
  > it is where the deadline's own failure mode — *"the surface hardens without it"* — actually
  > occurs. **R5** if the valve is spelled as a two-state machine, at the price of inheriting AB-7, a
  > blocker the valve does not need. **Gaia G7** can carry the document-side spelling but **not** the
  > action, which is an exported Aether symbol by ratified rule — and splitting it G7/R3 gives the
  > valve two homes, precisely the CG-1/CG-2 disease this section is about.
  >
  > **What it costs on R3:** one construct or one modifier that writes a named `flag` bit, its refusal
  > wording, and a trybuild golden. **No new storage kind, no new tag, no new runtime — all three
  > exist and ship.**
  >
  > ⚠ **What NOT scheduling it costs, and the asymmetry is the finding.** The corpus states the
  > mechanism itself: *"Logic creep has a documented trajectory (Paradox: literals → … → 'calculated
  > on every frame, massive lag'); what stops it is refusal plus a pressure valve (curves), not
  > discipline."* **That one sentence promises two valves, and only one was built.** The data
  > profile's valve (`curves` + `scalable`) is written into the *exhaustive* allowed list of §The
  > logic line; the document profile's valve got a deadline and no rung — and the corpus's own words
  > for that are *"a deadline with no rung can never come due, and is therefore not a schedule."*
  >
  > **⇒ Question (GB-6b), one sentence.** The valve's runtime already ships three times over in
  > `boyko_ui` and its Gaia-side spelling is already ratified, so the whole gap is one Aether
  > construct that toggles a named `flag` — do you want it attached to **R3** (where `tag`/`flag` and
  > the `scene` surface already live, and where the deadline's "surface hardens" failure actually
  > occurs), which is what converts the deadline into a schedule?
  >
  > **The carrier gap — CG-1 / CG-2, analysed with the same pass.** Both are assertions about
  > **Aether's emitted authored scenes**. The natural carrier is Aether **R3**'s `scene` surface,
  > which is also where the two Aether-side twins sit; the alternative is Gaia **G3**, beside PENDING
  > M3/M10. **The asymmetry decides it:** the Gaia side is already gated — **M3** owns the cross-check
  > (bake refuses a `link` mismatch *in either direction*, two red fixtures at G3) and **M10** owns
  > the bake refusal for an `Entity` field authored without `link` — and what is unowned is only the
  > **Aether emitter's** half. ⚠ **Which produces a concrete, checkable consequence: M3's fixture
  > cannot be authored honestly until CG-1 has a carrier**, because "in either direction"
  > presupposes the Aether side emits `link` at all; written today it is either red by construction or
  > vacuous — the "gate that cannot fail" class, arriving through an unassigned rung rather than
  > through bad wording. The precedent for what happens if nothing carries them is in the same
  > document: the `stable_name` ruling is ratified and its resolution axis specified at M8, while its
  > *application to Aether-emitted scenes* rides nowhere — so the ratified rule and the emitter can
  > diverge silently, and the only thing that would notice is a fixture that does not exist.
- **GB-7** — the wall-clock companion beside G7's count gate: kept or dropped, and if kept, its
  tolerance, run count and noise floor. **G7**, minor.

  > **RESOLVED 2026-08-30 — decided by standing rule. DROPPED.** No wall-clock companion. G7's gate
  > is the count gate alone, exactly as re-axed. The ground is measured, and it is **not** "a clock
  > is noisy" — it is that a clock does not measure the quantity this gate is about.
  >
  > **What is true at source** (read, not inferred — the shapes decide the ruling):
  > - `ui_bind_discovery` (`crates/boyko_ui/src/binding/bind_system.rs:75-89`) makes **one** call:
  >   `world.any_changed_since(&scratch.dynamic_bound_ids, …)`.
  > - `dynamic_bound_ids` is a **deduplicated set of component TYPES** — `register_bound_id`
  >   (`:56-60`) pushes only `if !self.dynamic_bound_ids.contains(&id)`.
  > - `EcsMaster::any_changed_since` (`crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs:403-423`)
  >   is `for archetype in …iter_archetypes()` over **every archetype in the world**, then `for &id in
  >   ids`, then `for row in 0..pool.count()`.
  > - `ui_bind_apply` (`:98-105`) returns immediately when `!dirty`, so a still frame is **discovery
  >   only**.
  >
  > **Therefore the still frame's wall clock is a function of archetype count, bound-TYPE count and
  > live-row count — and of the binding count not at all.** "200 bindings" is the number in the
  > gate's own name and it is **not an input to the loop being timed**: 200 or 2000 bindings over the
  > same three component types produce the identical id set and the identical scan.
  >
  > **Measured on this machine** (`windows-gnu`, rustc 1.97.1, `-O`; the discovery loop modelled at
  > HUD scale — 40 archetypes, 3 bound types, 200 rows; medians of 60 × 2000-call batches, two runs
  > agreeing to <1%):
  >
  > | what changed | still-frame cost | vs the fixture |
  > |---|---|---|
  > | the fixture as written | **332 ns** | — |
  > | bindings 200 → 2000, same 3 types | **332 ns** | **+0%** — the gate's own number moves nothing |
  > | 2× archetypes (an unrelated feature adds types) | 360 ns | **+8%** |
  > | 5× archetypes | 484 ns | **+46%** |
  > | 2× live rows (an unrelated feature adds entities) | 604 ns | **+82%** |
  > | 2× bound component types | 670 ns | **+101%** |
  >
  > A threshold pinned on that fixture is pinned to **none** of what its title names and to **all** of
  > what the fixture does not pin. Spawning one more entity in an unrelated part of the fixture world
  > moves it by tens of percent, so the companion would fail for reasons that have nothing to do with
  > binding cost — and the standing repair for that is to loosen the tolerance until it cannot fail,
  > which is where every gate in this repo's failure ledger ended up.
  >
  > **Resolution, second and independent ground.** `Instant::now()`'s median non-zero step here is
  > **100 ns**, so the whole still frame is **~3.3 timer ticks**. Single-call jitter on an idle
  > machine: median 200-300 ns against a max of 4600-5800 ns — a **15-25× tail** over the entire
  > measured quantity. The only form that reaches a usable ±1.7% process-to-process is a batch of
  > **200 000** frames (25 processes, 222 700-226 400 ns per 1000-call batch) — and 200 000 frames is
  > not a still frame; it is a microbenchmark of `any_changed_since`, which belongs to **GK-2**, not
  > to G7. Even that form's in-run spread ranged **36%-83%** across six runs, reproducing on the CPU
  > the lesson this repo already recorded for GPU timing: **the noise floor is not a constant.**
  >
  > **The tolerance the ballot asked for cannot be honestly quoted.** A tolerance must come from a
  > measured noise floor; the measured floor here is a range, not a number, and the signal it would
  > have to resolve — G7's red-first is *"dirtying exactly one source"*, i.e. **0 → 1 sink write** —
  > sits at a single-frame signal-to-noise of **0.05-0.50** across six runs, never once above 0.5.
  > The count gate resolves that same delta **exactly** (0 vs 1) with no instrument at all. Quoting a
  > round number instead would be exactly the move this ballot exists to prevent.
  >
  > **Rejected alternative and its price.** Keeping a loose companion (say "must stay under 1 ms")
  > costs a line that can never go red — it is 3000× the measured cost, so it survives any regression
  > the count gate would catch, and it would be read by later maintainers as timing coverage that
  > does not exist. That is strictly worse than no clock: the corpus already struck the
  > delta-subtraction form as unfalsifiable, and re-admitting it in a looser dress re-creates the
  > defect with a fig leaf. **A gate that cannot fail is worse than an absent one, because it is
  > counted.**
  >
  > **The condition under which a clock returns, stated so the drop is not permanent by accident.**
  > A wall clock over the **scan itself** — `any_changed_since` benchmarked with archetype count,
  > bound-type count and row count all pinned, and the number reported per row rather than per frame
  > — is a legitimate instrument, and it is exactly what **GK-2**'s design pass needs to justify a
  > per-column tick (GK-2's row already calls today's scan *"an O(live rows) scan documented as
  > 'cheap'"*). That bench belongs to GK-2 and is not a companion to G7's count gate.
  >
  > **Corroborated in passing, not reopened:** the `any_changed_since` doc comment (`:386-389`) claims
  > the scan is *"Bounded to the archetypes that actually host a bound id (typically 1-few)"*. The
  > loop at `:404` is over **all** archetypes; only the inner work is bounded. The corpus already
  > owns this as GK-2's companion doc fix ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §UI bindings,
  > item 8) — recording that the reading holds, and changing nothing else.
  >
  > **Riding lines moved in the same edit:** [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) row G7,
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §UI bindings item 9,
  > [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) §Work order,
  > and [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md).
- **GB-8** — **RESOLVED 2026-08-30** (orchestrator ruling; the owner delegated this ballot as one of
  the twelve mis-routed under CLAUDE.md's "perf and architecture forks are decided with numbers").
  Original question: the corpus-wide link/id census — which rung owns it, and whether it may carry
  per-site waivers. **Precedent, not a reopen** (no ratified item is touched). Ruling recorded in the
  campaign's own decision log: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Census discipline.

  **Measured first, because the ballot's own ground is not this tree's.** `188 of 302` appears in
  four corpus files and in the census test's doc comment, and **no in-tree gate produces it** —
  `tests/internal_docs_anchors.rs` states neither number. Run live (2026-08-30) it prints **735
  anchors, 116 waived**: ARCHITECTURE.md 6/0, FEATURE_MAP.md 222/7, SYSTEMS.md 330/20,
  MESHLET-VIRTUAL-GEOMETRY-PLAN.md **177/89**. The aggregate is 15.8%, not 62% — **and the
  precedent is stronger than the aggregate, not weaker**: the waiver did not spread evenly, it
  concentrated entirely in the one document admitted under the allowance, which now waives
  **50.3%** of its anchors, and a waived anchor "keeps **neither** shape nor identity"
  (`internal_docs_anchors.rs` head, clause 3).

  **Also measured:** the census that landed at `01a4436e` is not one scope but four.
  `tests/gaia_g0_citation_census.rs` (460 lines, **4 tests, all green**): the **AIR** census covers
  `docs/gaia` + `docs/aether-v2` (18 definitions, **114 citations across 13 files**); the **KE/KM**
  census **already runs corpus-wide over all of `docs/`** (16 rows, 101 citations, 352 files); the
  retired-form census also runs over all 352 files under a narrowed predicate; the staleness census
  covers the 1 file the revision record marks `ratified-stale`. None carries a waiver list.
  Widening the AIR census corpus-wide is **green at zero remediation**: the `AIR-##` citations
  outside G0's two directories sit in **5** files (`OPEN-QUESTIONS.md`, `ru/OPEN-QUESTIONS.md`,
  `AETHER-GAIA-REVISION-2026-08-29.md`, `FEATURE_MAP.md`, `AETHER-V1-SURFACE-REVIEW.md`), and
  **every id cited lies inside the carrier's `AIR-01..AIR-18`** — so none of them is a repair.
  ⚠ **The count is deliberately not pinned here, and the reason is this ruling's own footprint.**
  It measured **21** before the ruling was written and **40** after, because the ruling text itself
  cites `AIR-06` a dozen times in this file and its Russian twin. A citation count is a moving
  target that goes stale on the next edit — which is the exact defect this ruling corrects two
  paragraphs above. **The property is the ruling; the number is an observation with a timestamp.**
  **The LINK half had never been measured and is the half with a cost**: over the two G0
  directories, 116 relative markdown targets, **0 dead**; over all of `docs/`, **1636 targets, 59
  dead across 11 files — 44 of the 59 in one file**, `docs/AUDIT-2026-05-23.md` (the other 15: 8 in
  `docs/plans/`, 5 in `docs/archive/`, 2 in `docs/diagnostics/`).

  **RULING, three parts.**
  1. **No per-site waivers, at any of the four censuses, ever.** The ground is the live measurement
     above, not the cited 188/302. Where a property is not decidable as written, the remedy is the
     one this census file already practises and states at its own site — **narrow the predicate and
     print the narrowing in the failure message** (its retired-form test replaces ~1400 per-site
     dispositions with one decidable rule). A **scope statement** ("this census covers directory X")
     is not a waiver; a per-site skip list is.
  2. **The AIR census widens to all of `docs/`, and it lands at G0 — not R8.** It is a one-constant
     change (`G0_DIRS` → `markdown_under("docs")`), it is green at zero remediation, the sibling
     census in the same file already runs corpus-wide from G0, and G0 is still open (its row carries
     no ✅ and is held by F1/F4), so it can still take work. R8 depends on R3 and would date a
     one-line edit months out.
  3. **The link census is a SEPARATE deliverable and it lands at R8**, with the 59 dead targets as
     its red-first evidence and with `docs/archive/` + `docs/plans/` **in scope** — excluding them
     would be the scope-statement route and it is not needed, since only 13 of the 59 live there.

  **Rejected, with the price.** *Per-site waivers so the whole thing lands in one commit* — price,
  measured on the sibling gate: 50.3% abdication on the document that used the allowance, with
  `check_anchor` returning at the waiver branch before the shape test, so a waived anchor that is
  simply **wrong** about which line holds the symbol still passes. *Give the whole census to R8* —
  price: a green, free, one-line widening waits behind R3 and R8, while the KE/KM half in the same
  file already contradicts that placement by running corpus-wide from G0 today. *Land id and link
  together at one rung* — price: the free half is held hostage by the half carrying 59 repairs,
  which is how a gate gets deferred until it is convenient.
- **GB-9** — **RESOLVED 2026-08-30** (orchestrator ruling under the same delegation). Original
  question: does the `Or`-over-dense generated-code ban ([`gaia/DECISIONS.md`](gaia/DECISIONS.md)
  §UI bindings, item 8, second row) survive the kernel fix (KERNEL-BACKLOG **KE1**)? Keep it with a
  stated ground, or delete it with a record. The deadline "decide before R0 lands" **expired
  unanswered** — R0 landed at `01a4436e` — so the original ground can no longer be observed in the
  tree and this ruling reconstructs it from the record.

  **Measured.** (1) KE1 is fixed and gated: `crates/boyko_ecs/tests/ke1_or_dense_blindness.rs`
  **8/8 green** (run 2026-08-30; 7 of 8 red before the fix), and `impl_or_filter_tuple` now folds
  `HAS_DENSE = false || $F::HAS_DENSE` and forwards `resolve_dense` per arm.
  (2) **The ban's ground CANNOT have "moved to KE13", and a ruling that said so would be false.**
  The same macro folds `NEEDS_CHANGE_DETECTION = false || $F::NEEDS_CHANGE_DETECTION`, and
  `EcsMaster::query<D, F>()` opens with `const { eval_query_no_change_detection::<D, F>() }` — a
  **compile error** whenever `F::NEEDS_CHANGE_DETECTION`. So `Or<(Changed<A>, Changed<B>)>` cannot
  reach a `QueryView` at all, and KE13 is a `QueryView::get`/`get_mut` defect over a dense
  `With`/`Without` — a different shape from the banned one.
  (3) What *does* survive R0: the three shapes R0's own landing note says it did not cover
  (`Query<Entity, Or<..dense..>>`, `Added<Dense>` inside `Or`, arity > 2 with more than one dense
  arm); `par_iter`/`for_each_chunk`, which now **compile-refuse** a dense-armed `Or` — loud, not
  silent; and D4, whose reserve R0 narrowed to the consumer clause alone.
  (4) **The banned shape is not generator-reachable.** Gaia's ratified GN2 (§UI bindings item 4)
  rejects "bake emits Rust into the game build"; the runtime is a **closed bindable-type set**
  (`register_bindable::<C>`, shipped at `boyko_ui/src/interaction/plugin.rs:189`) plus type-erased
  fn-pointer accessors. The `Or<(Changed<C1>, …)>` change-gate in the shipped precedent is
  **host-composed** — `boyko_ui/src/binding/bind_system.rs` says so in as many words — hand-written
  engine-side, not emitted per binding. All four production `Or<(` type positions in the tree are
  hand-written and none is over dense (the only production dense component is `GpuTransform3D`).

  **RULING: option (b) — DELETE the ban, and replace it with a rule that is not about `Or`.**
  Its recorded ground is fixed and gated. Option (a)'s named candidate ground — D4's coupling — does
  not survive contact: **D4 reserves the Aether *surface* `or(...)`, while the ban governs *generated
  code*, and GN2 says the baker emits no Rust** — a ban on a shape the generator cannot emit,
  grounded in a reserve on a surface the generator does not write, is a rule with no subject. The
  row is replaced by **"a bind source or change-gate over a dense (or bitset) component"**, which is
  item 8's *third* row and whose ground (`any_changed_since` / `get_component_changed_tick` blind to
  dense — **GK-2**) is still live and unfixed; the `Or` row was always the weaker statement of the
  same hazard, aimed at one filter shape instead of at the storage kind. The fixture discipline is
  kept and re-aimed: the red fixture asserts the **generator does not emit a change-gate over a
  non-signature storage kind**, which cannot go green on a kernel commit. The three uncovered
  `Or`-dense shapes and KE13 are filed against G7's codegen rules as *"if a generator ever emits
  `Or` over dense, these are the shapes with no oracle"* — deleting the ban must not delete the
  knowledge.

  **Rejected, with the price.** *(a) Keep the ban.* Price: a ratified line whose stated ground is a
  fixed defect, whose fallback ground (D4) is about a different artifact than the one the ban
  governs, and whose only remaining candidate ground (KE13) is factually unavailable — i.e. exactly
  the "recommendation printed as if ratified" defect this ballot list exists to catch, re-created by
  the act of resolving it. Second price: the red fixture stays aimed at a shape nothing emits, which
  is how a gate becomes unfalsifiable.

### Aether v2 — the `AB` series

- **AB-1** — auto-registration of events: ratify on ergonomics alone, or STAGE it under D4 until an
  in-tree consumer exists. ⚠ **reopens the ratified C3 grant, and the reopen is licensed by
  measurement**: the grant's recorded ground was that the unregistered case "fails silently on both
  ends", and both generated ends are in fact a loud init-time panic. Blocks **R3**'s event
  construct.
- **AB-2** — ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E4** in
  [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md). **`lanes N` is redefined from a count to a
  MINIMUM**: the effective count is `max(N, worker_count + 1)`, resolved where the worker count is
  first known, and the raise is **reported once at boot**, not silent.
  *Measured, not reasoned:* the floor is unchecked on every path because
  `current_worker_id_or_dispatcher_lane` returns a worker's own id and **ignores** its
  `worker_count` argument — a correct denominator clamps nothing. The violation is a panic in both
  profiles (`thread_index 3 >= thread_count 1` in debug; `index out of bounds: the len is 1 but the
  index is 3` in release), never a `Result`, so the ballot's characterisation is **confirmed** — and
  it is a *safe* bounds panic, not UB, which is what makes raising affordable.
  ⚠ *The probe also found the floor is already violated by the DEFAULT path:* `EcsMaster::new()`
  hard-wires `EventDispatcher::new(1)` with no setter, so `preregister_event_default` allocates one
  lane on every machine — a 4-worker pool + default registration + 64 worker sends **panicked 3
  times** in release. Feasible because `App::new()` builds the pool before `add_plugin` runs.
  *Rejected:* **boot refusal** (a source correct on a 4-core laptop becomes unbootable on a 64-core
  server, for a number the author cannot know); **dropping the knob** — technically the cleanest
  answer, since `lanes` is not author-meaningful, but it **deletes a ratified language surface**,
  which is a SCOPE call: **escalated to the owner as a recommendation, deliberately not taken.**
- **AB-3** — ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E5**. **Per-thread claimed host lane,
  one claimer enforced.** `MAX_EVENT_THREADS` 65 → **66**, const-assert strengthening to
  `MAX_WORKERS + 2 <= MAX_EVENT_THREADS`.
  ⚠ **Two premises in this ballot were false and are corrected at source.** (1) `MAX_EVENT_THREADS`
  in the tree is **65, not 64** — `constants.rs:400`, raised together with its const-assert in
  `01a4436e`, the very commit this corpus was written alongside (`git log -L400,400:…` dates it).
  KE8's *lane-constant* half has **landed**; its `&self` send, `send_slice` and re-aimed debug
  assert have not. So this option buys **one** lane, not two. (2) It is not true that "every OS
  thread that is not a pool worker maps to lane 0": the mapping has **three** arms, and the
  **dispatcher gets its own reserved lane** (`worker_count`). Only `WORKER_ID_UNATTACHED` maps to 0.
  *The hazard, measured in release* — an unattached thread and worker 0 each sending 4000 events,
  released on a rendezvous barrier: **6 of 6 runs lost events** (1891, 2295, 2070, 183, 1473, 1940
  of 8000 = 2.3 %–28.7 %), and **every send returned `Ok`**. Without the barrier 6/6 runs lose
  nothing — which is precisely how a stress test that does not force overlap stays green over this.
  *Why "accepted hazard, documented" was unavailable:* the loss is silent **and** it is UB (two
  threads writing one `MaybeUninit<E>` slot through an `UnsafeCell`), and the tree already holds
  that option's output as a **false** `// SAFETY:` clause — `send_one`'s U4 clause 2 claims "only
  the worker pinned to `thread_index` accesses this UnsafeCell". Ratifying (c) would ratify an
  invariant measurement has refuted; principle 8 forbids it. *Rejected:* **`Err` on unattached** —
  breaks the escape hatch the engine itself documents (`EventWriter::send`'s doc sends main-thread
  and FFI callers to `send_event`, calling that route "safe"), and buys nothing the claimed lane
  does not. The chosen contract concedes that breakage for the **second** unattached claimant only,
  which is the genuinely unsound case.
- **AB-4** — ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E6**. **Registrant: the generated
  path**, calling a new `register_ordered_emitter(event_id, system_id)` at plugin build, beside the
  `preregister_event[_default]` it already emits. **Predicate** (the half the ballot said was
  missing): at build end, any `ordered` event id with an emitter count **> 1** is a hard boot
  failure naming both systems. **Verbatim escape: refused at the param list** — a hand-written
  `EventWriter<E>` for an `ordered` `E` is refused at `EventWriter::init_state`, the site that
  already panics loudly for an unregistered event and already has `E::event_id()` and the dispatcher
  in hand.
  *Rejected — `SystemMeta` emit-access:* measured, `EventWriter::init_access` is an **empty body**
  carrying "events stay OUTSIDE the conflict graph" (Phase 12 EW5 / Q2 Option A), so `SystemMeta`
  holds zero event information and there is no axis to read. Adding one as a *write* would make the
  scheduler serialise every pair of same-type emitters — surrendering exactly the parallel emission
  E1 exists to buy; adding a non-conflicting axis is option (a) with worse placement.
  *Rejected — the `#[event]` macro side:* structurally impossible, since exclusivity is a property
  of the emitter **set** and the macro sees one type and zero systems (it does not even read its
  attribute arguments today — `boyko_macros/src/lib.rs:261` takes `_args`).
  *Price of the refusal, stated:* a hand-written system cannot emit an `ordered` event even as sole
  emitter; the escape is to declare the emitter in Aether.
- **AB-5** — **RESOLVED 2026-08-30** (orchestrator ruling under the owner's delegation of the
  perf/architecture forks). Original question: the machine event router's random-access mechanism
  and the tick visibility following from it — (a) amend M4 to `Query::get_mut`; (b) keep
  `get_component_mut`. **Decision: (a).** Full ruling with the measurements:
  [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) **M4a**, with M7 and **D6a** amended in the
  same edit and [`aether-v2/MACHINES.md`](aether-v2/MACHINES.md) §Event routing + CAMPAIGN R5's
  Depends cell moved with them.

  *First, a correction to this item's own body.* It said the `Query` SystemParam "has **no**
  `get`/`get_mut` today". That was true when written and is **not** true now: both landed with R1 at
  `01a4436e` (`iters/query/query.rs`, `get` and `get_mut`), so (a) was a citation fix, not a new
  dependency.

  *The tick half, measured* under an explicit ordering edge — which the pre-existing tests lack, and
  without which "same frame" and "one frame later" are indistinguishable: `Query::get_mut` +
  `Mut<T>` is seen by a `Changed<T>` reader **in the same frame**; `get_component_mut` is **not**,
  even with the reader ordered after it.

  *But the decisive ground turned out not to be the tick at all.* `get_component_mut` takes
  `&mut self`, so its router can only be an **exclusive** system — and an exclusive body is
  `FnMut(&mut EcsMaster)` with, in the kernel's own words, "no param tuple, no per-param state"
  (`system/exclusive_function_system.rs`). Such a router **cannot take `EventReader<E>`**. It must
  read through `EcsMaster::events_of`, whose contract is "Returns an **empty slice** if `E` was not
  registered or if no events were sent last frame" — the one silent path ruling C3 already
  identified, collapsing *unregistered* and *nothing happened* into one value. Option (a)'s router is
  an ordinary system: it takes `EventReader<E>`, and a forgotten registration is a **loud boot
  panic**. `events_of` also serves the **previous** frame, making (b)'s latency **two** frames
  against the one MACHINES.md documents.

  *Rejected — (b), and its price:* a machine whose `inbox` event is un-preregistered routes nothing,
  silently, forever; doubled latency; plus a bypass mechanism that exists only to repair (b).

  *A ground I expected and did NOT get, recorded so nobody re-argues it.* Exclusive systems declare
  `Access::universal()` and are single-per-round, so (b) looked like a per-event-type scheduler
  barrier. **Measured against a control that proves the schedule had real concurrency to lose**
  (1 thread 235–250 µs vs 8 threads 79–133 µs, **2.7–3.2×**): an exclusive router cost **0.78–1.00×**
  a `Query` router at both 1 and 8 routers — below the ±20 µs noise, sign flipping between runs.
  **Scheduler cost grounds neither option.**

  *The two riding questions, answered here rather than left to drift:* `publish tracked` **does**
  now see the router's deposit in the same frame; and M7's tick bypass is **withdrawn, not
  re-derived** — measured, a plain `&mut T` through `get_mut` bumps **no** tick while `Mut<T>` bumps
  at `this_run`, so all-or-nothing falls out of the emitted term. (b) had no such lever:
  `get_component_mut` returns `Mut<T>` unconditionally, which is exactly why a bypass had to be
  invented for it.
- **AB-6** — `requires` of a dense-storage component: parse refusal / a dense required-ctor route /
  known-open plus a hook workaround. ⚠ the refusal option **narrows the ratified
  `storage = table|dense` × `requires` surface**. The red tests land now under any disposition —
  they demonstrate the panic either way. Blocks KERNEL-BACKLOG **KE11**'s disposition and **R3**'s wording.
- **AB-7** — R-DENSE: unconditional with a driver-independent ground that must be ESTABLISHED rather
  than asserted, or lifted by `publish tracked`. ⚠ **re-grounds a ratified refusal**. Blocks **R5**.
  **STILL OPEN — STILL THE OWNER'S.** What was delegated was not the choice but the *measurement*
  underneath it, and it was taken on **2026-08-30**: **the candidate driver-independent ground is
  REFUTED, on both of its conjuncts.** The ballot therefore goes to the owner with a refuted premise
  rather than an open question. Evidence, all read or run in this tree at `01a4436e`:

  1. *"the router's deposit path assumes a table row"* — **false.** Both random-access deposit APIs
     carry working dense arms: `EcsMaster::get_component_mut` resolves the global `DenseStore` and
     bumps the per-slot `changed_tick` (`ecs_master/component_api.rs`, `StorageKind::Dense` branch),
     and `Query::get`/`get_mut` carry `HAS_DENSE` gates. Pinned green in-tree by
     `tests/ke3_query_random_access.rs::{get_applies_a_dense_with_filter,
     get_mut_applies_a_dense_with_filter}`.
  2. *"the layout const-assert assumes a table row"* — **there is no such assert.** Every dense
     `const { assert!(…) }` in the kernel belongs to a **driver** (`for_each_chunk`,
     `par_for_each_chunk`, `par_iter`, `Query::contains`; `dense_iter` asserts the converse). Every
     other `StorageKind::Dense` site is a branch that *handles* dense, not one that rejects it.
  3. Measured positively: a dense component is fully iterable **and** change-tracked under the
     sequential `iter_mut` driver — 64 of 64 rows reached a `Changed<>` reader — which is exactly the
     lowering `publish tracked` selects.

  **What the measurement did find** is a real constraint, but narrower than "driver-independent":
  dense is compile-rejected (`E0080`, reproduced) by **all three non-sequential drivers** —
  `for_each_chunk`, `par_for_each_chunk` **and `par_iter_mut`** — and `par_iter`'s own message gives
  the reason as *"the parallel path does not resolve the dense store into each worker chunk's
  `Fetch` (the chunk runner has no world cell)"*. The rejection tracks **parallelism and chunking**,
  not chunking alone. Two consequences for whoever takes this: **(i)** option (b) is what the engine
  supports today, and lifting the refusal does **not** make `parallel` machines work over dense,
  because both parallel drivers reject it too; **(ii)** a genuinely driver-independent ground may
  exist but lives in a **different ballot** — `requires` over a dense component panics
  (KERNEL-BACKLOG **KE11** / ballot **AB-6**). That is also the owner's and is **neither settled nor
  assumed** here.
- **AB-8** — **RESOLVED 2026-08-30** (orchestrator ruling; a performance fork, so it is decided with
  numbers rather than escalated). Original question: the `each par` driver, plus three riders. Full
  ruling: [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) **C5a**.

  **Decision: `each par` → `par_iter_mut`.** Measured rather than taken from the labels — over 2048
  rows (clear of the 1024-row inline floor), with the writer keyed `.before` the reader: a
  `Changed<>` reader sees **2048 of 2048** `par_iter_mut` writes and **0 of 2048**
  `par_for_each_chunk` writes. Cost, release, 200 000 rows × 200 frames, 8 workers, five samples:
  `par_iter_mut` is **1.17–1.47×** `par_for_each_chunk` (median ≈1.19), a delta of **0.03–0.07
  ns/row**. Absolute ns/row is **not** reproducible run to run (0.13→0.34, machine noise) — the
  *ratio* is, which is why the ratio is what is recorded. At the machine cost model's own 10 000 rows
  that delta is **0.3–0.7 µs/frame**, against the **34–40 µs** ruling D1 measured for the 5-arm jump
  table at the same row count: **≈1–2 % of the pass**. The measurement carried a falsification guard
  (row count and per-row increment count asserted after timing), so a no-op could not have produced
  the numbers.

  *Rejected: `par_for_each_chunk` as the `par` driver.* Price — the most inviting parallel spelling
  in the language would be structurally tick-blind, silently killing every downstream `Changed<>`.
  That is C5's own recorded defect, and the 0-of-2048 above is it reproduced.

  The three riders, answered with it: **`soa par` EXISTS**, and is the only route to
  `par_for_each_chunk` (tick-blindness stays behind the word that announces it). **The batching key
  is NOT author-visible in v1** — measured ground, not taste: `BatchingStrategy` has **three** fields
  (`batches_per_thread`, `min_batch_size`, `max_batch_size`), so `parallel (batch = N)` cannot name
  what it appears to name; *rejected alternative* — mapping `N` to `batches_per_thread`, whose price
  is that an author writing `batch = 64` expecting 64 rows per chunk gets 64 chunks **per thread**.
  The knob stays reachable via `batching_strategy(…)` from the verbatim escape, and the form is built
  when a measured in-tree consumer appears (D4's rule). **Machines and `each` share the LADDER, not
  the default** — both select tracking by term and climb `iter_mut` → `soa` → `par`, but `each`
  defaults to `iter_mut` (C5) and the machine pass to the chunked driver (M7/D6), each measured
  separately; unifying the defaults would reopen M7/D6, which AB-5 licensed only for the citation and
  the withdrawn bypass. Left standing deliberately.
- **AB-9** — **RESOLVED 2026-08-30** (orchestrator ruling under the same delegation). Original
  question: `boyko_reflect` sequencing for AIR-06(b). Options were: R8 waits on the merge /
  AIR-06(b) is descoped to the reflection-free halves (asserted to still satisfy the oracle) / the
  merge is pulled forward. Ruling recorded in the campaign's own decision log:
  [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) §Sequencing rulings, entry **AB-9**.

  **Measured, and the ballot's figures were stale in both directions.**
  1. `feat/reflection` is **15 commits ahead of and 20 BEHIND** `feat/aether-v2` (merge-base
     `5ec1699f`). "18 commits ahead" was measured against a different base and before
     `f7c46c76`/`d272e1fd`/`0e0b4c68` landed. The behind-count is the number the ballot never
     carried and it is the one that grows.
  2. Merge cost, from `git merge-tree --write-tree` (**no merge performed**): **7 conflicted
     paths** — 4 docs (`FEATURE_MAP.md`, `SYSTEMS.md`, `OPEN-QUESTIONS.md`, `ru/OPEN-QUESTIONS.md`),
     2 trybuild `.stderr` (`unknown_key_rejected.stderr` content; `on_despawn_rejected.stderr` a
     **modify/delete** — `01a4436e` deleted it on this branch while reflection modified it, so it
     needs a decision, not a re-bless), and **one source file**,
     `crates/boyko_macros/src/component.rs`, at **2 hunks / 23 lines**. The merge brings 15 commits,
     124 files, +44 956/−214, and **3 new workspace members** (`boyko_reflect`, `reflect_fixture`,
     `reflect_dogfood`).
  3. **The ballot's load-bearing claim — that the reflection-free halves "still satisfy its oracle"
     — is false as it would be used.** AIR-06's oracle has three clauses; the second is *"a
     probe-crate component must appear in the dump"*, and "the dump" is (b)'s **project** schema — a
     *grammar* manifest generated from parser dispatch tables cannot contain a workspace component.
     Descoping (b) does not leave that clause satisfied; it removes its subject. It is *satisfiable*
     reflection-free only by declaring the probe component in an `aether!` block — which makes the
     gate green over a dump covering **0 of the 138 `#[derive(Component)]` sites in the engine
     crates** (`boyko_ecs` 43, `boyko_ui` 36, `boyko_render` 21, `boyko_physics` 15, `boyko_scene`
     14, `boyko_demo` 8, `boyko_input` 1; 185 across all of `crates/*/src`). Measured: **zero
     production `aether!` declaration blocks exist** — all 50 live under test files, and the 6
     `*/src` hits are the macro's own implementation and doc comments. Gaia declares nothing (no
     baker).
  4. **And the merge alone does not fix that either.** `boyko_reflect::registry::type_info_of`
     returns `None` for "a component without `#[component(reflect)]`" — reflection is **opt-in per
     component**. On `feat/reflection`, `#[component(reflect)]` appears at **106 lines across 33
     files, and not one is in an engine crate** (23 files `reflect_fixture`, 4 `boyko_reflect`, 3
     `reflect_dogfood`, 3 `boyko_macros`).

  **RULING, three parts.**
  1. **Pull the merge forward** — `feat/reflection` merges into `feat/aether-v2` as its own rung
     before R8, at the measured cost above. "Waiting" is rejected as an option with no event to wait
     for: nobody else is merging the branch, and it is already 20 commits behind, with the lag
     concentrating in `crates/boyko_macros/src/component.rs` — the one file both the derive work and
     the reflect work touch.
  2. **AIR-06(b) is NOT descoped.** The descope buys a vacuous green.
  3. **A fourth item, which no option on the ballot named, is the real precondition and is attached
     to R8's Lands**: engine components must opt into reflection — either a sweep marking them
     `#[component(reflect)]`, or a decision that the derive opts in by default. Until one exists,
     AIR-06(b)'s dump covers the empty set under **all three** of the ballot's options. **Which of
     the two routes is taken is R8's own design pass and is NOT decided here** — it has a
     per-component static + `OnceLock` cost that has to be measured, not argued. In the same edit,
     AIR-06's oracle clause is corrected from "a probe-crate component" to "a hand-written
     `#[derive(Component)]` component **from an engine crate**", because the probe-crate form is
     satisfiable by a route that measures nothing.

  **Rejected, with the price.** *R8 waits on the merge* — price: R8's gate goes green on a dump
  covering 0 of 138 engine components (the opt-in set is empty), while the branch keeps diverging
  and the one real source conflict keeps growing in exactly the file both campaigns edit.
  *Descope AIR-06(b) to the reflection-free halves* — price: the same zero coverage **plus** the
  loss of the oracle's only workspace-facing clause, i.e. a gate that cannot fail.
- **AB-10** — AIR-10 residue: is the measurement script required when the audit branch is taken, and
  is PENDING's widening to the joint Aether+Gaia vocabulary ratified? ⚠ the second is a **scope
  change to a ratified requirement** and must not arrive as a side effect of an edit.
- **AB-11** — `with`/`without` over a `flag`. Recommended: a parse refusal with a did-you-mean
  pointing at `enabled`/`disabled`, which dissolves the whole class (`with Flag` matches nothing,
  `without Flag` excludes nothing; both silent). The alternative is forbidden-form-only, caught at
  doc-generation. ⚠ **adds a refusal where v1 documents non-refusal**. Blocks **R3**'s filter
  goldens.
- **AB-12** — *(a query, not a values call)* do the two dropped measured defects — the old G1/G2 of
  the AI-orientation defect series — exist in the session record? If they do, they return as AD5/AD6
  with repros; if not, the renumbered record stands. Only the owner's session archive can answer.
  Non-blocking.
- **AB-13** — the `flag` initial-value vocabulary, four parts that interact and are answered
  together: (1) the values themselves — `on | off` vs `true | false` / `set | clear` /
  `enabled | disabled`; (2) whether the chosen words are RESERVED keywords or contextual
  (contextual keeps them usable as identifiers, at the price of a grammar that reads differently in
  two places); (3) disambiguation across all THREE `on` positions already in the surface
  (`machine … on entity`, `on E => T`, `flags (X = on)`) — a reader and a generator must tell them
  apart without lookahead; (4) the group's NAME. On (4), read what PENDING says now: Tier 3
  **withdrew** its own `flags` → `initial` rename, on the ground that `initial` is already the
  machine's initial-state keyword, so the rename recreates the collision it was meant to fix
  ([`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md) Tier 3). The open question is
  therefore whether that withdrawal STANDS, or a different rename is wanted. ⚠ any rename here
  touches a ratified keyword, so AIR-10's bar applies and the offered measurement is the collision
  audit over the three `on` positions. Blocks **R3** (the `flag` construct surface and its filter
  goldens).

**Two items on this list are NOT ballots, and are named so they are not mistaken for one.** K8 (no
honest spawn spelling for a bundle carrying `link Entity`) and K9 (the kernel's relates/related
macro demands a private collection field plus `retain_empty`, inexpressible on a pub-field Aether
group) are architecture gaps, routed to R3's design pass rather than to the owner. R3's `bundle` and
relation constructs must not be declared done while they stand open.

---

## 2026-08-28 — Gaia: the owner ballots, two of them blocking

The Gaia research (two multi-agent passes, ~25 systems surveyed; plan corpus now at
[`gaia/`](gaia/CAMPAIGN.md)) closed every architecture/perf fork by standing rule, and left seven
VALUES/SCOPE ballots. The 2026-08-29 corpus audit added three more (F8-F10, logged in the entry
above), so the Gaia `F` series now numbers **ten**; the seven raised on this date carry their full
bodies below. [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) §Owner ballots is the one-line INDEX only — it
used to point here for the bodies while this file pointed back at it, which is the circle that kept
five of them from ever being written.

The two that BLOCK rungs:

- **F1 — the bake route into the byte format.** `save_world` is the only legal printer, so the
  baker must build a live world from text. Route (a), the EG2 reflection seam, is already rejected
  by the owner's own audit ballot of 2026-08-27; route (b), macro-time GK-4 (derive-emitted
  name-keyed field tables + typed constructors), needs no reflection at all and detaches Gaia from
  both EG2 and the unlanded C11. Recommendation: (b) now, (a) as a later upgrade. The real ballot
  is SEQUENCING: decide EG2 first (it unblocks more than Gaia), or detach now. Blocks G1.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (b), macro-time GK-4, taken NOW.** *"Do it properly right
  > away. But bear in mind the world must support streaming."* Both halves bind: G1 takes the GK-4
  > route, and no part of the bake design may foreclose streaming — which is why F5 lands with the
  > scene profile rather than after it. **Rejected: decide EG2 first.** Price: G1 waits on a seam
  > this campaign does not need. ⚠ **One inference in the body above is refuted by the ruling** — the
  > baker is a Rust program linked against the derive-emitted tables, so it calls a `RequiredCtor`
  > fn pointer trivially (measured through today's public `required_ctor_in_set`); what cannot call
  > one is the *evaluator over Gaia text*, a different program. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline.
- **F4 — what "loaded" means. WIDENED: the loader misses more of the insert path than hooks.**
  The finding that opened this ballot was "the loader runs no hooks". That is ONE mechanism of
  several, and the ballot as first written would have bought a hook-coverage census and still left
  a loaded world differing from a spawned one. Everything the insert path does that the load path
  does not:
  - **Hooks.** Reverse indexes (`Children`, `LikedBy`) are absent after a load.
  - **The `#[require]` closure. MEASURED: nothing on the load path adds a component the file
    omitted.** A document naming `Foo` but not its required `Bar` loads an entity with no `Bar`,
    where `Commands::spawn` of the same bundle would have both. Nor can the baker close the gap:
    `RequiredCtor` is `unsafe fn(dst: *mut u8)`
    ([`required.rs`](../crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs)) —
    a raw constructor pointer, which a total, closed, build-time evaluator cannot call and
    therefore cannot bake into the file. This is the half that decides the answer.
  - **Flag / `EnableTag` initial state**, whatever an insert would have stamped.
  - **Relation reverse indexes** — hook-built, and what makes a relation queryable from the other
    end at all.
  - **Asset refcounts stay at 0**, so every mesh of a loaded scene is subject to retirement
    mid-game.

  The same document is therefore correct in the dev loop (`Commands` spawn fires all of it) and
  broken in the shipped game. Options: (a) a specified post-load fixup pass whose coverage census
  spans ALL FIVE mechanisms, not hooks alone — recommended; (b) the load path runs the real insert
  path (correct by construction, but a hook then mutates the world mid-load — a real semantic fork
  plus a per-row price); (c) forbid load-incomplete components in baked assets — untenable, it
  forbids `MeshHandle` and every `#[require]`d component at once. **Addendum (the G6 carrier):**
  whichever option wins must also NAME the stable asset-id carrier a loaded scene's references use.
  `MeshHandle(u32)` blits a process-local slot, and `MeshRef` — the name the language sketch
  writes — does not exist in the engine, so the carrier is unlanded on both spellings; F4's answer
  is what makes a loaded reference mean anything after a restart. This defines the engine's load
  semantics generally; decide before the first Gaia scene. Blocks G6.

  > ✅ **RESOLVED 2026-08-30 [delegated] — option (a), specified as SUPPRESS-THEN-FIXUP: four ordered
  > sub-passes, on the clone path's own precedent.** Full ruling and every measurement:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Load semantics. In brief:
  >
  > **All five reproduce at `6f75ee9e`** (`cargo test -p boyko-serialize --test
  > gaia_f4_load_path_fixups -- --ignored --test-threads=1` → `0 passed; 5 failed`, controls green),
  > and the hook census is `grep -rEn "trigger_on_[a-z]+\("` → **0** dispatch sites on the load path
  > against **38** on the command/master path.
  >
  > **The order is the ruling:** (1) require closure at **parse** time — widen the id list with
  > `for_each_required_id_excluding` and emit the **existing** `LoadColumn::Construct`, which already
  > takes a `RequiredCtor` and is simply never reached for a component the file does not mention;
  > (2) declared flags via `flags_direct_for`; (3) the existing remap; (4) hooks, then drain deferred
  > commands to a fixpoint. Requires must precede hooks; **hooks must follow the remap.**
  >
  > **(b)'s MECHANISM is adopted as sub-pass 4; only its TIMING is refuted — and the ballot's stated
  > price for (b) was wrong.** `DeferredEcsMaster` statically withholds every structural-change
  > method, so a hook *cannot* mutate the world mid-load. The real hazard is ordering against the
  > remap: `relationship_on_insert` reads a **raw saved FK** and guards on `is_alive`, and the
  > measured saved/fresh id overlap across a normal round trip is **8/8, 100%** — so the guard does
  > not fire and the hook builds a reverse index pointing at a **live but wrong** entity, strictly
  > worse than today's empty one. **This bug already has a name in this kernel: BUG-EDGE-CLONE-1**,
  > and the clone path's shipped fix is exactly a suppression bracket plus a relink after the FK is
  > remapped. **The kernel has already answered F4 once, on the sibling path, and it answered (a).**
  >
  > **(c) verified, not inherited, and worse than "untenable":** an attribute-line census over
  > `crates/*/src/`, hand-audited, finds **10 require-bearing shipped components** (three light
  > types, `ParticleEffectHandle`, three camera types, `MeshHandle`, `MaterialHandle`,
  > `UiWorldProjection`'s owner) plus **3 explicit hook-bearing** and the whole
  > `Relationship`/`RelationshipTarget` family. (c) forbids **every mesh, material, light, camera and
  > particle effect**; `MeshHandle` alone is load-incomplete on three counts at once.
  >
  > **Two corrections to the body above.** (ii) is **not a load-path defect** — it is the *dynamic
  > by-id entrance*'s, which the loader shares: `EcsMaster::create_entity` is public, shipped, and
  > produces no required component where `Commands::spawn` does, so the silent wrong answer is live
  > one call from gameplay code, and a GK-4 baker built on it would bake the defect **into the file**.
  > (iii) **splits in two**: the *declared* half is reconstructible with **no format change at all**,
  > and only the *authored per-entity* half is F9's.
  >
  > **The addendum is answered too: the carrier is a stable asset NAME in the file, resolved to the
  > existing `MeshHandle(u32)` at load — no new component type.** `grep -rn "\bMeshRef\b" crates/` →
  > **0** (all 7 repository hits are corpus prose), while `MeshRefGen`/`MaterialRefGen` is a
  > **generation counter** — the name is one suffix from a live type meaning something else, so
  > **`MeshRef` must not be minted**. And there is no name- or path-keyed asset lookup in the engine
  > at all (**0** hits), so the registry is a **G3** build item. ⚠ This names a *carrier form*, not a
  > name-vs-id ruling: **F8** stays free.
  >
  > ⚠ **Two escalations the ruling does not own.** The `create_entity` require-drop is an **engine**
  > defect independent of Gaia and needs its own carrier — filing it under F4 would let a
  > Gaia-scoped fix leave the gameplay-facing hole open. And `remap_loaded_entities` is **O(whole
  > world) per load**, which under F5 now runs on every `load_cell`.

The remaining five, in the same shape:

- **F2 — where a DataAsset's rows live at runtime.** (a) rows are ENTITIES: each row lands in a
  dense-column archetype, with derive-emitted row constants (a `[DefOf]`-shaped surface) so Rust
  names a row without a lookup. Price: a row costs an `EntityId` and its archetype slot — a
  4000-row item table is 4000 entities the world carries from boot. (b) a new RESOURCE region in
  the byte format: rows are a flat column owned by a resource and addressed by index. Price: a
  second load path in `boyko_serialize` beside `load_archetype`/`load_dense_store`, and the format
  grows a region it does not have today (inventory finding 3). Recommendation (a); its VALUES half
  is the sentence *"a table is entities"* — everything stays queryable, one storage, Principle 0
  intact. Rejected without a ballot: a `HashMap<Name, Row>` side store — a parallel data system,
  which the standing rule forbids outright. Blocks G5.

  > ✅ **RESOLVED 2026-08-30 [delegated] — (a) rows are entities, AMENDED three ways the ballot did
  > not carry.** Full ruling: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables.
  >
  > ⚠ **The ballot's phrase "a dense-column archetype" is a category error, and it is the thing the
  > owner's remark caught.** A dense id is signature-excluded, so `get_or_create_archetype(&[DenseRow])`
  > returns **the empty archetype** (measured: `dense_arch == empty_arch → true`) — under the ballot's
  > own wording 4000 item rows would land in the world's component-less bucket beside every bare
  > entity in the game. **(i) Storage is `StorageKind::Table`**, which is what *"data that doesn't
  > have to be dense"* denotes in this engine's vocabulary: a table whose rows carry the row component
  > and nothing else is **one archetype with exactly one `ComponentPool`, already contiguous**, while
  > dense would add `s2e`, an `e2s` sized to the maximum entity id ever inserted, a live bitmap and a
  > free list for a contiguity `Table` already has. The mechanical rule for which kind a table gets:
  > *is the row component carried by entities outside the table?* **(ii) Rows materialize EAGERLY at
  > load**, never lazily — under lazy materialization `save_world` records which rows *happened to
  > have been touched*, so the same game state saves to different files and a round trip is no longer
  > one; and every insert-path mechanism F4 enumerates would fire at an arbitrary later frame.
  > **(iii) A table is loaded by its own explicit load and PINNED**, so `unload_cell` never enumerates
  > it; the runtime handle is an **`Entity` captured at load**, never a row index, because
  > `Archetype::swap_remove` moves rows and loads append into dedup'd archetypes.
  >
  > **The price, measured rather than asserted.** 4000 rows of a 40-byte row type as table entities
  > cost **522 752 B ≈ 511 KiB** marginal against a running world (pool data 262 144 + two tick
  > regions 65 536 each + `entity_ids` 65 536 + inland 64 000), and **0.006%** of the inland store's
  > slot ceiling; the per-frame cost of those idle entities is **one extra archetype mask test per
  > query**, not 4000 row visits. **Rejected (b), and its price:** it saves **129 536 B (25%)**, or
  > 62% *only if a new tick-free VM primitive is also written*, because `VmColumn<T>::new` **panics**
  > unless `size_of::<T>()` divides the commit granule and 40 does not (nor 48, 56, 72). Against
  > 0.13 MB it costs a format version bump, **≥ 607 production + ≥ 444 test lines** by the dense
  > region's own precedent — for a region whose payload shape, unlike dense's, has **no existing
  > analogue** — and **the entire per-`ResourceId` serialize seam from scratch**:
  > `grep -rniw "resource" crates/boyko_serialize/src/` returns **0 occurrences of the word anywhere
  > in the crate, comments included**. It also forfeits queryability, which is the shape Principle 0
  > exists to refuse.
  >
  > ⚠ **Two corpus corrections made in passing:** `load_archetype`/`load_dense_store` are **not** in
  > `boyko_serialize` (they are `pub fn` in `boyko_ecs::…::serialize::load_writer`; the crate's own
  > halves are `load_one_archetype`/`load_dense_region`), so the ballot's "second load path" straddles
  > two crates; and two engine docs contradict each other on the W4 anchor — one says loads go into
  > freshly created archetypes, the other says `create_archetype` **dedups** and the anchor is
  > relaxed. Neither is Gaia's to fix.
- **F3 — the shape of a table file.** Single-file-per-asset only (one document = one asset), or
  ALSO a table file that bakes N rows into one dense column. Price of the second form: the grammar
  grows a row-repetition shape and identity has to name a row INSIDE a file (which is F8's
  territory) — against the authoring win of one document holding 400 items instead of 400
  documents. The ballot is not "one form or two": it is whether both forms bake through ONE schema,
  which is the recommendation — the table file is a spelling, not a second type system. Rejected:
  a table dialect, which the one-language-three-profiles ruling already refuses in the large.
  Blocks the grammar (G5's authoring surface).

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — both forms, over ONE schema.** The table file is a
  > **spelling**, not a second type system. Rejected: a table dialect. The owner asked for a
  > recommendation on the details, which is answered separately and is not this line. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables. ⚠ Its identity half — how a row is named
  > *inside* a file — is **F8**'s and stays open; F2's ruling constrains only the *runtime* handle
  > (an `Entity` captured at load) and says nothing about the authored identity.
- **F5 — streaming scope, and the alternative nobody wrote down.** The recommendation on the table
  is (a) *format-ready-loader-later*: the cell catalog, its attribution, and the persistent id map
  (GK-1) land NOW as one design unit; `load_cell`, unload, and cross-cell references land at G8.
  **The alternative was never named, which is exactly why it is on this list.** It is (b) — the
  streaming half INSIDE G6: `load_cell`/`unload_cell`, GK-1's cross-load map with a declared
  lifetime, and cross-cell reference resolution, shipped with the scene profile rather than after
  it. Price of (a): a catalog with no loader is a datum nothing consumes until G8 — this
  repository's own recurring defect class — mitigated only by G6's reconstruct-and-compare gate,
  which does read the catalog, so it is a datum with a consumer before the loader exists. Price of
  (b): G6 absorbs the hardest half of the design, because a per-load `LoadEntityMap` cannot express
  references BETWEEN chunks (measured), and the scene profile cannot land until that is solved.
  Blocks G6, through what its catalog is required to contain.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — option (b), the alternative nobody had written down.**
  > *"Все сразу грамотно по списку с самого начала."* `load_cell` / `unload_cell`, GK-1's cross-load
  > map **with a declared lifetime**, and cross-cell reference resolution ship **with** the scene
  > profile at **G6**, not after it at G8. **The rejected option's price is the one the body already
  > states** (a catalog with no loader is a datum nothing consumes until G8); **the ruling's price is
  > the larger one and is accepted knowingly** — G6 absorbs the hardest half.
  >
  > ⚠ **This is the single largest change to the campaign's ground, and it lands directly on F4.**
  > Six GK-1 requirements now belong to G6, each from a measured blocker; the two that must not be
  > lost are (i) **insert-after-`finalize` must become legal**, because today a second cell's inserts
  > land in an unsorted tail that `binary_search` **silently misses** in release, and (ii) **the
  > fresh-world contract must be lifted** — `load_dense_store`'s guard is a `debug_assert!` whose own
  > message reads *"merge load is unsupported"*, and it **vanishes in the shipping build**, so
  > `load_cell` would corrupt the dense store silently in release and loudly only in dev. ⚠ **GK-1 and
  > GK-3 must land together** or the cross-cell half is silently half-present: a `ChildOf` across a
  > cell boundary only enters the reverse index when F4's sub-pass 4 fires. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Streaming scope; the rung rewrite is in
  > [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) rows **G6** and **G8**.
- **F6 — the fate of `.ui` once Gaia absorbs it.** (a) migrate the existing `.ui` documents to the
  Gaia `ui` profile and DELETE the old format in the same campaign; (b) freeze `.ui` where it is
  and decide after an owner-eval of a real Gaia HUD. Price of (a): the migration is done blind —
  G7's prerequisites (a windowed UI pass, a real `UiPlugin`) do not exist, so nothing renders a
  Gaia HUD to judge before the old format is gone. Price of (b): two authoring formats for one
  subsystem, which is the diverged-pair cost this repository has already measured on `docs/ru/` —
  a reader cannot tell which is current and finds out by acting on the stale one. Recommendation
  (a). Blocks G7.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a): migrate to the Gaia `ui` profile and DELETE the old
  > format in the same campaign.** Rejected: freeze until an owner-eval of a real Gaia HUD, on the
  > diverged-pair cost already measured in this repository. **The ruling's own price is recorded at
  > G7 rather than left to be discovered:** the migration is done **blind**, because G7's
  > prerequisites — a windowed UI pass and a real `UiPlugin` — do not exist. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape.
- **F7 — mods, and the executable the ratified refusal does not mention.** ⚠ **This borders a
  ratified refusal and has to be framed against it, not asked fresh.** What is ratified: *own text
  → build-time bake → binary; reflection only at bake, behind a default-off feature; the shipped
  load path has zero reflection* ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline) —
  a constraint on the GAME BINARY. A text-mod pipeline does not violate it as written: it ships the
  bake tool to players as a SEPARATE executable, and the game still loads only bytes. So the
  question is not "may reflection ship" — that is answered, no — but **whether the refusal meant
  "no reflection in the game binary" or "no bake tooling in a player's hands at all"**. The two
  readings differ only where mods exist, which is why the corpus carried both without noticing.
  Options: (a) ratify "mods are out of v1" explicitly AND record the constraint in its narrow form,
  so a later mod campaign is not blocked by a sentence that never meant to block it; (b) accept the
  separate-executable pipeline now and design the format's stability guarantees for it from the
  start. Price of silence: it is a DEFAULT DECISION — the wide reading calcifies by never being
  contradicted. Blocks nothing today; it decides what a later campaign is allowed to propose.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a): mods are NOT supported for now, ratified
  > explicitly.** The ballot existed because silence was itself a decision; it is no longer silent.
  > **What the explicit ratification buys is the NARROW form of the constraint**: what is ratified is
  > *no reflection in the GAME BINARY*, not *no bake tooling in a player's hands at all* — so a later
  > mod campaign is not blocked by a sentence that never meant to block it. **Rejected (b) — accept
  > the separate-executable pipeline now and design the format's stability guarantees for it.**
  > Price: v1's format gains a compatibility surface for a consumer that does not exist, which is the
  > dead-datum class. Price of the ruling, accepted knowingly: if a mod campaign is ever taken, those
  > guarantees are retrofitted rather than designed in — and the *"for now"* in the owner's answer is
  > what makes that a schedule rather than a prohibition. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Refusals.

---

## 2026-08-28 — The `machine` "one transition per frame" claim is FALSE: two same-frame events run BOTH exit/action/enter chains — FIXED 2026-08-30 (rung R2)

Found while designing per-entity machines, confirmed by two independent reads of the emitter and
the schedule. `run_state_transitions` executes ONCE, before the executor loop, so `State<S>` is
constant for the whole frame — while the emitter gives every route its OWN system gated only by
`.run_if(in_state(leaf))`, with no ordering edges and no latch between siblings. Two events of
different types arriving for one leaf in one frame therefore run BOTH exit/action/enter chains, and
`NextState` is decided by the last write. The emitter's own comment (`expand.rs`, the transition-fn
doc) calls this "§5.1 one transition per machine per frame", and the registration-order note claims
declaration order makes two same-frame transitions deterministic — it determines WHICH wins, not
HOW MANY run.

**Scheduled fix (owner-approved direction):** the `state_chart!` move into `boyko_macros` (campaign
rung R2, [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md)) merges a leaf's routes into one dispatch,
which makes a second same-frame chain structurally impossible AND lands the arbitration alignment
(first-declared-wins) the owner chose. **The red test comes first**: two events, one leaf, one
frame → today it must FAIL by observing two chains; after R2 it pins exactly one.

Nothing else is blocked; the global `machine` misbehaves only under same-frame multi-event load,
which the shipped tests deliberately avoid (distinct event types per edge).

**RESOLVED 2026-08-30, rung R2.** The red test is
[`aether_tests/tests/r2_chart_arbitration.rs`](../crates/aether_tests/tests/r2_chart_arbitration.rs),
and it was watched failing before anything moved. MEASURED pre-fix, all five counters at once on
one leaf with `on Alpha => ToAlpha` declared before `on Beta => ToBeta`:

| exit | alpha action | beta action | enter ToAlpha | enter ToBeta | settled state |
|---|---|---|---|---|---|
| **2** | 1 | 1 | 1 | 1 | **ToBeta** |

Two exits, two actions, two enters — and the surviving state was the LAST-declared route's, which
is the arbitration artifact M6 called out. Post-fix the same run reads `1 / 1 / 0 / 1 / 0 /
ToAlpha`.

The fix is the per-leaf **route merge** in `boyko_macros::state_chart!`
(`state_chart::emit::leaf_fn`): one system per leaf drains every lane, a single `__sc_route`
selection takes the first-declared accepting route, and one `match` arm runs the only chain. A
second same-frame chain is now structurally impossible — there is one selection point, so the
question "how many chains ran" has no way to answer anything but one.

Two things a reader should not have to rediscover:

* the emitter comment that made this claim is gone with the emitter — the flattening now lives in
  `boyko_macros`, and Aether lowers to it;
* every lane is still drained even when its route loses, which preserves the pre-merge per-lane
  drain exactly. The kernel's `EventIter` advances the cursor only past what it yielded, so a
  merge that skipped the loser's lane would have left this frame's events to re-fire on the next.

---

## 2026-08-27 — The event participant context is a DEAD DATUM: computed, leaked, stored, never read — RESOLVED 2026-08-28

Found while designing the Aether sugar for `event`, when the owner asked whether a participant could
carry query-style filters (`victim: entity(with Health, without Invulnerable)`). Before answering I
followed what today's `entity(Health)` actually reaches, and it reaches nothing.

The chain is complete and every link is real:

```text
Aether  entity(Health)
  -> #[participant(components = "Health")]
  -> ParticipantInfo { name, required_components: &[Health::component_id()] }
  -> EventTypeInfo::participant_info                    (event_registry.rs:126, :175)
  -> pub fn get_event_participants(event_id)            (event_registry.rs:272)
  -> (nothing)
```

**Grep over the whole workspace** (`crates/`, `--include=*.rs`): `required_components` occurs
**once** — its own field declaration at
[`participants.rs:28`](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs). `get_event_participants` has
**zero callers**. Not in the dispatcher, not in `EventReader`/`EventWriter`, not in a `debug_assert`,
not in a test, not in a bench.

So every event registration pays for it — one `<Comp as Component>::component_id()` per context
component, a `&'static` leak per participant list behind a `OnceLock` — and no code path consults the
result. This is the class already recorded as recurring in this repo (five prior instances); this is
a sixth, and it is on the PUBLIC event surface, which is why it is worth a decision rather than a
silent deletion.

Scope of the check: `get_event_participants` is `pub`, so an out-of-tree consumer is possible in
principle. Inside the engine, its tests and its benches there is none.

**What is worth deciding — three readings, and they are not equally cheap.**

1. **A debug-time assertion.** On `send`, `debug_assert` that the participant entity actually carries
   the declared components. This is what the field looks designed for, it vanishes in release, and it
   turns a whole class of "the event fired but the reader's `get_mut` returned `None`" into a loud
   failure at the send site. Cost: one archetype lookup per participant per send, debug only.
2. **A read-side filter.** Events are double-buffered and cross a frame boundary, so by read time the
   participant may be dead, may have lost `Health`, or may have gained `Invulnerable`. Today every
   reader re-checks this by hand (`let Ok(h) = q.get_mut(d.participants.victim) else { continue }`).
   If the context were live, `EventReader` could skip such events itself and the check would leave
   every reader. This is the reading that would make `without` mean something, and it is the most
   valuable — and the most expensive, because filtering must not cost the readers that do not need it.
3. **Documentation, and say so.** Keep it descriptive, and write in the doc comment that it is
   never consulted — so the next person does not spend the same half hour looking for the consumer.

**What it blocks.** The Aether sugar `entity(with A, without B)` is on hold until this is answered.
Extending a list nobody reads would double the dead datum, and it would ship a syntax that *looks*
like a guarantee while giving none — which is worse than not having it. Reading 2 additionally needs
engine work (`#[participant]` grows a second channel, `ParticipantInfo` a second slice) that should
not be started before the first slice has a consumer.

Nothing in the engine is blocked: events dispatch correctly today, because none of this is on the
dispatch path. What is blocked is the language surface above it.

**RESOLVED 2026-08-28 — reading 1, scoped to the machine event router (owner delegated the call).**
The per-entity machine design gives the datum its first consumer: the generated router that
deposits an event into the victim row `debug_assert`s, in debug builds only, that the victim
actually carries the declared context components. A stun sent to a crate becomes loud at the send
site; release cost is zero; a miss stays the safe silent `None`. The `without`-filter extension
remains unbuilt (reading 2 stays unfunded until the checked half proves itself). Recorded in
[`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) §M4.

---

## 2026-08-26 — A worker panic under the windowed host FREEZES the window instead of ending the process

MEASURED while wiring `PhysicsPlugin` into `examples/playground.rs`. The plugin's
`.colored_solve()` registered `physics_solve_colored` — a stage that reads
`ResMut<ColoredSoftStepSolver>` **by name**, not through the pipeline's generic `S` — while the
resource inserted was `SoftStepSolver`. The param resolve did exactly what it should:

```text
thread 'boyko-worker-1' panicked at crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:22:5:
Resource `boyko_physics::solver::colored::ColoredSoftStepSolver` not registered.
```

**What the operator saw was none of that.** The window went "not responding" and stayed there: no
crash, no exit code, no CPU (6.5 s of CPU over 9 minutes of wall clock — the process was blocked,
not spinning), and the panic text scrolled past in stderr where a windowed run does not look. It was
reported to me as "boyko playground is not responding", and I had to bisect the frame loop with
`eprintln!` probes to find a *panic*.

**The plugin bug is fixed in this commit** — `insert_physics_resources` now inserts
`ColoredSoftStepSolver` whenever `colored_solve` is set, so the stage's parameter always resolves.
That closes THIS instance and none of the class.

**The class, and the question.** `Schedule::run` re-raises the first worker panic (the event-lane
campaign's fix, `schedule.rs:223`), and `CommandQueue::apply` calls `resume_unwind` — so a panic is
*supposed* to leave the frame loop. Under `boyko_app`'s windowed runner it did not: the process
stayed alive with a dead schedule and a live window. Whether the panic is swallowed on the way out
of the Fixed schedule specifically, or the re-raise lands somewhere the runner never observes, I did
not chase — it needs a deliberate look at the propagation path rather than an incidental one.

Two things that are worth deciding rather than assuming:

1. **Should a worker panic take the window down?** A frozen window is the worst of the three
   outcomes (crash / degraded frame / freeze): it hides the diagnosis the engine already produced,
   and an operator cannot tell it apart from a GPU hang or a deadlock. My inclination is that the
   host should catch the propagated panic and exit with the message on the console, but that is a
   VALUES call about how a shipped game should die.
2. **Should a registered system with an unresolvable resource fail at BUILD time?** Every parameter
   a schedule needs is known when `ScheduleBuilder::build` runs, and the world is right there. A
   build-time check would have turned this into a startup error naming the missing type, rather than
   a first-substep panic on a worker thread. The cost is that it forbids "insert the resource later,
   before the first run", which some legitimate boot orders may rely on.

Nothing is blocked on either — the scene runs. What is blocked is anyone else meeting this class and
spending the same hour on it.

---

## 2026-08-20 — Gate #17: two findings about the INSTRUMENT, one fixed in this commit and one still open

Measuring the particle passes (213 legs over four sessions) produced two facts about the measuring
apparatus itself. They are recorded here because both outlive the measurement: anyone who reads a
`ZONE_PARTICLE_DRAW` number, or who wires a harness around `particle_lab`, walks into them.

**1. `ZONE_PARTICLE_DRAW` was readable only WITHIN one scene. — RESOLVED (architect's ruling,
2026-08-20); the fix lands in this same commit.**

MEASURED: putting an `SdfPrimitive` slab in the scene moves `ZONE_PARTICLE_DRAW` by **+74 752 ns** at
65 536 alive and by **+369 664 ns** at 102 400 — for work the particle draw does not do. The three
`BOTTOM_OF_PIPE`-stamped compute rows (kickoff/emit/sim) read **exactly 0 ns** for the same change at
all eight density cells, so this is not scene noise: it is the `TOP_OF_PIPE` drain absorption
`gpu_zone.rs` already warns about (*"the `TOP` rows … may each include a share of the drain ahead of
them"*), now measured on the particle family. The consequence while it stood: **any cross-scene DRAW
comparison was invalid**, and nothing said so at the point of reading.

The premise id 51 topped on was *"the whole lit producer runs before the particle draw, so its
isolation is unconditional"*. That is true — and it is a statement about **BRACKETING**, which the
measurement shows is not the same claim as isolation from **DRAIN**. **The id is restamped to
`BOTTOM_OF_PIPE`** (the DP6-0b precedent for ids 10/11, and cheaper here: unlike `ZONE_VB_SHADE`, no
published number is defined against id 51's `TOP` stamp, so there is no compatibility pin to break).
**The fix is measured, not assumed.** Re-taking the same null at 65 536 alive, 3 legs per arm, after
the restamp: base DRAW **93 184 ns** (was 106 496 — the absorbed drain leaving the bracket) and
**ctrl − base = +3 072 ns / +3.2 %**, against **+76 800 ns / +72 %** before. **96 % of the cross-scene
absorption is gone**; the resolvable 3-step residual is stated rather than rounded away. The three
compute rows are unmoved (SIM 73 728 on both arms), which is the control, and all six legs report
`measured = 315`, `lost = torn = not_bracketed = 0`, `frames_checked = 21`, `violations = 0`. The five
image goldens are byte-identical — a begin stage moves no pixel, and that is proven rather than
argued. Gate #17's own DRAW column is void as a baseline across the restamp by construction — the
same treatment DP6-0's four cells got, and for the same reason.

**2. Every gate-#17 run is a RED TEST BY CONSTRUCTION, and a harness cannot tell it from a real
failure. — STILL OPEN.**

An armed-zone `particle_lab` run writes its profiling artifact and *then* fails the process. The
cause is two **mutually-exclusive** exits from the same frame loop in `crates/boyko_app/src/runner.rs`:

* the **zone-budget exit** — `vb_zone_seen >= VB_BENCH_WARMUP(20) + vb_zone_frames`, which writes the
  artifact and `return`s from `App::run`;
* the **capture-driver exit** — the conjunction over the five settle/drain drivers (host dump, census,
  HZB, VB probe, cull readback, particle readback), which fires at presented frame 30 (+3 drain).

With `BOYKO_VB_BENCH_FRAMES=1` the budget is 21 retired frames, so the first exit always fires first
and the frame-30 dump is never written — after which the fixture panics reading a file that does not
exist. Every number in gate #17 came out of a process that exited non-zero. **What is open: a harness
that keys on exit codes cannot distinguish "the measurement completed and the dump was skipped" from
"the run genuinely broke".** I am not fixing it here — the fix is a scope call (either the two exits
learn about each other, or the measurement legs stop asking for a dump they cannot reach), and both
touch a loop that five other capture drivers share.

**Also recorded, as an environmental shape rather than a defect: 1 windowing flake in 213
artifact-producing runs (0.5 %).** One leg reported `frames=400` (the `BOYKO_WINDOW_FRAMES` cap) with
no artifact: the window's client area was 0×0, so the runner's minimized path `continue`s before the
fence, the uploads and the render, and no zone frame ever retired. Re-run in isolation it gave
`frames=23` with a clean census. It is written down because **a silent zero-artifact run is
indistinguishable from a disarmed instrument to anything that only checks "did the file appear"**.

---

## 2026-08-20 — Particles P2 item 1: a dead fixture knob (repaired), and one pin I will not re-point without you

Landing the Deferred `-D DEPTH_LINEAR` arm turned up two things that are yours rather than mine.

**1. `BOYKO_PARTICLE_RATE` was a DEAD KNOB, and gate #17's density numbers depend on it.** MEASURED
while trying to drive a denser fan for the occlusion control: a `BOYKO_PARTICLE_RATE=8` run produced
a dump **byte-identical** to the rate-1 one (`sha256 60f39a3c…`). Cause: `particle_scene::setup`
seeds `ParticleEmitter::burst` from the env value, but `lab_arm_burst` — ordered BEFORE the fold,
including on frame 0 — overwrote it with a hardcoded `1` before anything consumed it. The knob's own
carrier advertised it as live in two places (the env table and `spawn_per_frame`'s doc, which names
gate #17 as its consumer), so the next person to run a density measurement would have measured a
1-per-frame scene and reported it as 8.

**Repaired here** (the re-arm reads `spawn_per_frame()`, the one fn both sites now read), because
the alternative — documenting it dead — leaves gate #17 undrivable. Verified in both directions:
rate 8 now renders **1737** saturated particle pixels against **265** at rate 1, and the default is
unchanged, so `particle_additive` and the other four image goldens re-proved byte-identical.

**What is yours: the P0 measurement rows this invalidates.** Gate #17 asks for kickoff/emit/sim/draw
µs at 10k/100k/1M. Any number produced through this knob before today came from a 1-per-frame scene
whatever the env said. I do not know which of P0's reported measurements, if any, were taken that
way — the ones I can see in the plan are stated as budgets, not as captures — but if a density
number was ever quoted from this fixture, it needs re-taking.

**2. `particle_additive` is still pinned on VisibilityBuffer, and re-pointing it is a VALUES call.**
The pin's comment said "re-point this pin at Deferred when that arm lands"; the arm has landed and I
did **not** re-point it. Doing so replaces a blessed digest with one nobody has looked at, and the
two paths shade the scene differently (only the particle pixels are identical between them — the
full BMPs are not). The comment now records the landing and leaves the decision open. **Say the
word and it is one `-Bless` run; the reason to want it is that the pin would then cover the engine's
DEFAULT path instead of the one it fell back to.**

---

## 2026-08-12 — L8a: two SCOPE calls I made narrowly, and one I did not make at all

Rung L8a migrated `boyko_render` / `boyko_image` / `boyko_serialize` / `boyko_physics` (16 sites,
twelve codes). Three things sat on the boundary between "decide it yourself with numbers" and "this
is the owner's". I decided two and am recording them for review; the third I am not deciding.

**1. `resolve_candidate` conflates an ABSENT optional texture with an UNREADABLE one, and I left it
conflated.** `crates/boyko_render/src/texture.rs`'s `load_slot` has two arms: a file that decodes
wrong is now `boyko-W2206` (a `Warn`), and a file that does not resolve is an `info!` with no code,
because all five material-folder slots are documented as optional and warning on absence would put
a `Warn` in the log of every material that ships four maps instead of five.

The problem is that the second arm also covers a file that *exists and cannot be read* — a
permissions fault, a locked file, a bad mount — because `resolve_candidate` discards the
`io::Error` with `.ok()`. So a real fault is reported at `info` level, indistinguishable from a map
somebody simply chose not to author.

**The cost of splitting it**, measured: `resolve_candidate` returns
`Option<(PathBuf, Vec<u8>)>` and would have to return the reason, which means re-blessing the three
tests that pin its `Option` signature (`resolve_candidate_prefers_the_first_existing_candidate_in_order`,
`..._falls_back_to_a_later_alias_when_the_first_is_absent`, `..._returns_none_when_no_candidate_resolves`).
That is a signature change to a helper in the asset load path, inside a rung whose scope is
"replace `eprintln!` with a coded emitter". I kept the rung's scope and recorded the hole rather
than widening quietly. **If you want it split, it is a small, self-contained change and I will do
it in its own commit.**

**2. `RatePolicy` is declared on every registry row and applied by nothing.** Measured by reading
the expansion, not the design: `warn!`/`error!` gate on the three ceilings and call `emit_impl`;
neither reaches `rate::admit`, which still has zero production callers. Every `Once` in this
registry works because a human placed an `OnceSite` at the emitter.

That is not itself a defect — `Once` and `Every` need no machinery. What it means is that a row
declaring `EveryN` or `MinIntervalMs` would be a **promise with nothing behind it**, and no check
would notice. I added `no_live_row_declares_a_policy_the_emission_path_cannot_honour` to `codes.rs`,
which reds on exactly that. It cannot prove a declared `Once` has an `OnceSite`; its failure text
says so.

**The question is what happens to `rate::admit`.** It has been carried for several rungs with no
caller. Either L11a/L14 wire it into the emission path — which puts a rate check on the enabled
path of every `Warn`/`Error` — or it is deleted and the registry column narrows to the three
policies the engine can actually honour. I have no measurement that favours either, and it is a
scope call.

**3. `E2203` floods, and I did not damp it.** `GpuSystem::run_unsafe` has no `Result` channel, so a
device fault that recurs reaches an operator only as a record per frame. I declared `Every`,
matching the `eprintln!` it replaced, on the reasoning that a `Once` would report the first bad
frame of a session and let an hour of broken frames look identical to a good one. The flood is
bounded by the ring, which drops and counts. If you would rather see one line per second than one
per frame, that needs item 2 resolved first — there is no mechanism today that could deliver it.


## 2026-08-11 — ⚠️ MEASURED: validation DOES run on this box, and the 2026-08-06 entry below is narrower than it reads

Opening logging rung L7 (migrate `boyko_rhi_vulkan`) started by re-deriving the site list, because
the rung's row cites line numbers that have drifted. It also had to establish what `E2101` — "add
an `error!` when validation is requested but the node was not chained" — can actually observe here.
Two measurements, both against HEAD, **before any L7 code was written**:

1. **A validation-ON boot works.** With `BOYKO_DISABLE_VALIDATION` **unset** and
   `enable_validation: true`, `cargo test -p boyko_rhi_vulkan --test compute` boots and passes
   **4 of 4**. The standing note that the SDK's MSVC-built `VkLayer_khronos_validation.dll` crashes
   this MinGW process on load is not true of the **headless compute path**. Whatever it describes —
   most likely the windowed/golden path — it is narrower than "validation cannot run here", and I
   have been treating it as the wider claim.
2. **The chained validation-features node is built, not unbuildable.** `create_instance` enables
   `VK_EXT_validation_features` when present and chains `VkValidationFeaturesEXT` with
   synchronization validation as the head of the instance `p_next`. Disposition **F2** ("a chained
   validation-features node is unbuildable here") is refuted by the tree.

**What that costs the plan.** G7's first clause — "`E2101` fires on a validation-**on** run" — holds
only if the node can never be chained. It can, so on a correct box a validation-on run must be
**silent**, and the gate as specified would be red against a working engine. I have re-cut `E2101`
to mean *validation was requested and this process is not getting it* (the escape hatch took it, or
the extension is absent), which makes G7 two-sided and runnable here: positive = escape hatch set,
negative = unset. The full argument is in the corpus (`logging/ladder`, the L7 block).

**This is an architecture call and I made it** rather than waiting — it is a gate's polarity, not a
value. It is here because it **contradicts a disposition the owner may have relied on**, and because
of what it does NOT change: `M25` stands. `compute.rs`'s own `negative_chained_barrier_hazard`
documents in the tree that sync-validation is enabled and still does **not** flag a compute→compute
RAW hazard on this path. The layer being *present* and the layer being *sensitive* are two
questions; L7 can gate the first and nothing gates the second.

> ⚠️ **A question I asked here and then measured, and it should not have been asked.** The first
> version of this entry offered the owner a choice: every golden runs under
> `BOYKO_DISABLE_VALIDATION=1`, so after L7 each one emits a `boyko-E2101` line — *"should it be
> suppressed for the golden legs?"* **Both halves of the premise are false**, and the question's
> shape was worse than either: it invited weakening a diagnostic to protect a channel, when
> **saying that a golden run's validation was disabled is the entire reason the code exists.**
> Suppressing it there would deliberately rebuild the defect the 2026-08-06 entry below describes.
>
> 1. **No collision is possible.** `scripts/golden.ps1:226` scans with the literal pattern
>    `\[vk-validation\]`. `boyko-E2101` cannot match it.
> 2. **In a golden run the line does not exist at all.** Measured: **no host calls
>    `boyko_log::lifecycle::boot` or `enable`** — the only callers anywhere are `boyko_log`'s own
>    tests and `boyko_ecs/tests/log_seam.rs`, and `crates/boyko_ecs/src/ecs/core/log/plugin.rs:40`
>    says so in its own doc comment. So the `error!` goes into a `.bss` lane ring nothing drains,
>    and not one byte is printed.

### And (2) is the finding that matters more than the question it answers

**The logger is in exactly the state the profiler was in at `e0160555`: complete, gated, and
unreachable from every host.** L5 landed the ECS seam, L6 landed the engine's emitters, and nothing
turns it on — so every record L6 just wired up is written into a ring with no consumer. Twelve
`Live` rows, five new codes, ten doc pages, and in a shipped run the whole apparatus is silent for a
reason no gate reports.

It is not a defect *of* L5 or L6 — `boot`/`enable` belong to `boyko_app`, which is **L8b's** row, and
`plugin.rs` was written knowing it. What is worth the owner's attention is that this is the **same
shape, in the same campaign, two rungs after it was found the first time**: every gate builds its own
world, enables logging itself, and asks whether the record arrived — so none of them can see that no
host ever does. I am recording it now rather than at L8b because the last time this shape appeared,
fifteen green rungs had passed over it.

**Nothing is blocked and no decision is needed**; L8b closes it by construction. If the owner wants
it closed *earlier* — a host that boots the logger before L7's migration lands, so L7's own sites are
observable in a real run rather than only in tests — that is a scope call and the only one here.

---

## 2026-08-11 — L6 found three mechanisms that exist and are unreachable, and left them that way on purpose

Logging rung L6 migrated `boyko_ecs` and `boyko_threadpool`. Three things it touched are **built,
correct, and consumed by nobody**. None of them blocks the rung; each is recorded here because
"reached for it and decided not to" is the only thing that distinguishes a deliberate gap from an
oversight, and because two of them are the same shape as the defect L6 opened with.

**1. `TargetControl`'s sync-route bit has no reader.** `target.rs` packs `bit [7] sync route —
format on the caller, write synchronously`, with a constructor, an accessor, a CAS that preserves
it and its own unit tests. `grep sync_route` over `crates/boyko_log/src` returns **`target.rs` and
nothing else**: `emit_impl` never consults it. A target with the bit set behaves exactly like one
without. Its intended writer is `apply_control_spec` (L14, the `net=debug/6!` form), so the bit is
early rather than wrong — but a *control* nobody reads is exactly what `site.decode` was, and that
one went three rungs unnoticed. **Not implemented at L6** because honouring it means a second
emission path (render on the caller, take `OUT_LOCK`) which is L14's row and needs L14's gates.

**2. `rate::admit` has zero production callers.** The rate limiter is complete and unit-tested —
`EveryN`, `MinIntervalMs`, the 512 cache-line slots, the suppressed counter. Every engine registry
row declares `Every` or `Once`, and both are answered by a **site-local latch** by design (F11), so
`RATE` is never touched. L6 considered `MinIntervalMs(1024)` for `W0701` — an event lane that
refuses every frame is exactly what a per-second cap is for — and **refused**: it drags a clock read
onto a cold ECS path and puts the rate decision *ahead* of the macro's own runtime gate, so a
disabled target would pay for a policy on a record it will not emit. The honest statement is that
`RATE`'s 32 KiB of `.bss` is reserved for a policy no engine row currently declares.

**3. `E0201`'s stderr fallback owes a `print_allowlist.txt` row at L8c.** `abort_on_task_panic`
prints for itself **iff** `flush()` answered `NoConsumer` — see the L6 decision block for why the
ledger's `error!` + `flush()` alone would have made the abort decision invisible in a
diagnostics-off process. L8c's `print_census.rs` bans `eprintln!` in production; this site needs a
row naming that reason. Flagged now because L8c is four rungs away and an unexplained allowlist
entry written then would read as laundering.

**What the owner may want to decide**: nothing is blocked. If (1) or (2) should be *deleted* rather
than left for L14/L11a — a bit and an array that cost `.bss` and reader attention — that is a
values call, and it is the opposite of the call this campaign has been making (absent rather than
stubbed). My own reading is that both are fine to keep, because both have a named consumer at a
named rung, which `site.decode` never did.

---

## 2026-08-11 — ⚠️ A profiling test's HAND-PICKED zone id is a bet against every schedule the rest of the crate runs. `ZONE = 7` still is one.

Found by rung 12's `G18`, which is the first profiling gate to assert an **exact session total** rather
than a single cell. In a full-workspace sweep it counted **20 013 of 20 000** samples: thirteen it
never pushed.

**The mechanism, and why the module lock cannot help.** `profiling/tests.rs` serialises every test
that arms, on one `test_serial()` lock — which is correct and insufficient. `ARM_MASK` is
**process-global**, so while any profiling test holds the profiler armed, **every other test in the
`boyko-ecs` binary that runs a schedule emits `SystemSpan` samples** (`profiling/zones.rs:193`) —
on its own thread, into its own lane, which the fold drains along with everything else. Those samples
carry per-system zone ids minted at `try_build` out of the same monotone `ENGINE_ID_NEXT` the static
zones use.

A hand-picked `const ZONE: u16 = 7` is therefore a bet that no system anywhere in the crate's test
suite lands on id 7 — and the bet is **re-rolled by every change to test execution order**, which is
why it had never fired before.

**Fixed for rung 12's own zone:** the tier tests now use a `declare_zone!`-minted handle. The counter
is monotone and shared, so an id that handle owns is one no `SystemMeta` can ever be given. Not a
mitigation — the collision becomes unrepresentable.

**NOT fixed, and this is what the owner may want to decide.** `const ZONE: u16 = 7` at
`profiling/tests.rs:41` is still a raw number, used by roughly thirty assertions across the rung-2
and rung-3 suites. They are far less sensitive — they read one `(frame, zone)` cell after a drain
they control, rather than a session total — so a stray sample would have to land in the same frame
*and* the same cell to be seen at all. But the hazard is the same one, and it is the kind that
surfaces as an inexplicable off-by-N years later.

Two ways to close it, neither urgent:

1. **Mint it too** (`declare_zone!` + a `fn zone()`), mechanical but touches ~30 assertions in tests
   that currently pass — a diff whose risk is entirely in the churn.
2. **Leave it and rely on the insensitivity**, with the hazard recorded here, which is the state
   today.

⚠️ **The general form is worth more than the instance:** *any* test that arms the process-global
profiler is measuring a channel the rest of the test binary is also writing to. A profiling gate that
asserts a total — as every rung-12-and-later gate does — must own an id nothing else can be given.

---

## 2026-08-11 — ⚠️ The `trybuild` corpus is blessed for a DIFFERENT rustc than the toolchain the project mandates. 23 fixtures were red at `3163078f`.

Found while sweeping rung 11, and **proved not to be rung 11's** with `git stash`: with the whole
working tree stashed, `cargo test --no-fail-fast` over the eight `compile_fail` suites at
`3163078f` reds **seven of them, 23 fixtures**, under
`RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu` (rustc **1.97.1**, installed 2026-08-04).

**The diff has TWO causes, and my first diagnosis of it was WRONG — recorded that way because the
wrong one is the plausible one.** I wrote "compiler rendering drift between rustc 1.95 and 1.97.1"
and then tested it. Both claims below are probe results, not inference.

**Cause 1 — RUNG 11'S OWN, and it is not drift at all: `impl_self_bundle!` crossed a rustc
rendering THRESHOLD.** `compile_fail_relations/relationship_hook_collision.stderr` changed from a
span-based `help:` block into an inline `= help:` list of bare names. That looks exactly like a
compiler-version change and is not one. MEASURED with a five-line probe compiled by both binaries:
**rustc 1.95.0 and 1.97.1 render the identical trait-bound error identically**, and **rustc switches
from spans to the compact list at FIVE candidate impls**:

```text
3 impls -> help: … --> probe.rs:2:12  |  2 | struct T1; impl Marker for T1 {}      (spans)
4 impls -> spans
5 impls ->   = help: the following other types implement trait `Marker`:  T1 T2 T3  (list)
```

The `Component` list held **three** entries (`ChildOf`, `Children`, `LikedBy`) and rung 11 added
**three more** (`ProfilingScope`, `ProfilingScopeEnabled`, `ProfiledZone`) — six, past the threshold,
so the whole block re-rendered. `compile_fail_relations` was **green on the clean tree and red with
the change**: that suite is entirely mine. The same two `impl_self_bundle!` lines also appear inside
several other fixtures' `Bundle` lists without flipping their format.

⚠️ **The general lesson, which is worth more than this instance: adding ONE trait impl anywhere in
the engine can re-render every pinned diagnostic that lists that trait's implementors — and past the
fifth impl it changes the FORMAT, not just the contents.** A `.stderr` corpus is coupled to the
engine's impl *count*, invisibly.

**Cause 2 — INHERITED, 23 fixtures across 7 suites, and its origin is UNDETERMINED.** Proved not to
be mine by `git stash`, above. The signatures are additions and substitutions the blessed files do
not carry — e.g. `compile_fail_chunk/mut_data_rejected.stderr` gains an entire
`note: required by a bound in Query::<'w, 's, D, F>::for_each_chunk` block; elsewhere
`\| $crate::panicking::panic_fmt(…)` collapses to `= note: the failure occurred here`, and `AtomicIN`
becomes `Atomic<iN>`.

**I could not determine what produced them, and I am not guessing in this file.** The hypothesis I
had — that rung 10's *"543 targets ok, 0 failed"* was measured under the chocolatey `cargo`/`rustc`
1.95.0 that shadows `~/.cargo/bin` on `PATH` (real, found in this session, and the cause of a phantom
`E0133` on `__cpuid` plus a wall of MSVC `link.exe` failures) — is **not supported** by the probe:
the two compilers agree on the renderings I could test. The remaining candidates are a stale bless
predating an unrelated signature change in `query.rs`, or a rustc I no longer have. **What is
measured and certain: 23 fixtures were red at `3163078f` under the mandated toolchain, and rung 10's
green certification did not cover them.**

Both causes are re-blessed here in one pass, under 1.97.1, and are listed separately above so the
diff is reviewable rather than a wall.

**RESOLVED 2026-08-11 (owner: "реши сам"). Both decided; shipped as
`tests/trybuild_corpus_compiler_witness.rs`.**

**1. NO `rust-toolchain.toml`. A COMPILER WITNESS instead — and the reason is that a
`rust-toolchain.toml` would not have caught this.** The shadowing binary is a **standalone**
`cargo.exe`/`rustc.exe` from chocolatey, not a rustup proxy; a standalone cargo ignores
`rust-toolchain.toml` outright, so the file would have looked like protection while providing none.
Worse, the only form of it that would fix the *other* half — pinning the host triple, `channel =
"stable-x86_64-pc-windows-gnu"` — breaks every non-Windows build of a workspace whose stated targets
are *"Windows / Linux (x86_64)"*.

What ships reads the compiler's own version string and compares it to a `BLESSED_RUSTC` const
updated **in the same commit as any re-bless**. It catches a toolchain update *and* the shadow, on
any host, and it fails with both versions named plus what to do about it. **Its two REDs were run:**
naming a compiler that is not running prints `blessed: 1.98.0 … running: 1.97.1 …`; raising the
fixture floor prints `claims to speak for at least 9999 … and found 90`.

**The precedent decides the shape.** This repository already freezes a compiler for a byte-exact
corpus: every committed `.spv` is gated against a **frozen `dxc` recipe in the shader's own header**,
so a compiler change cannot silently redefine the artifact. A `.stderr` corpus is that shape with a
different compiler and had no freeze. Now it does.

**MEASURED while writing it, and it corrected the entry above:** the corpus is **90 `.stderr`
files**, not the 24 this section first said. 24 was the number of files rung 11's *diff* touched. **A
count taken from a diff is a count of what changed, not of what exists**, and the two are equal only
by accident.

**2. `trybuild` STAYS.** A compile-fail fixture proves a property no runtime test can reach — that
the type system *rejects* a shape — and this rung leaned on exactly that for `G12` clause 3 and rung
10 for `G22b`. The price is real and is now **visible instead of silent**: when rustc changes, one
named gate fires and says so, rather than 23 fixtures mismatching under a green-looking sweep.

⚠️ **One coupling the witness does NOT remove, recorded because it is the surprising one.** A
`.stderr` corpus is coupled to the engine's **impl count**, not only to the compiler: past five
implementors rustc switches the *"other types implement trait …"* block from spans to an inline
list. Adding one trait impl anywhere can therefore re-render a pinned diagnostic in a crate that has
nothing to do with it. No gate can prevent that; the witness at least stops it being confused with
compiler drift, which is exactly the confusion it caused here.

---

## 2026-08-10 — RESOLVED (owner: "реши сам"): both rung-10 gate questions, decided and closed

The owner delegated these two rather than deciding them. Both are decided below, with the reason
each way was taken, and both entries in `05-LADDER-GATES.md` are updated to match.

### `G17` keeps the A/B ratio. No release-profile absolute-nanosecond gate.

**Decision: keep what ships.** The question was whether to build a release bench harness and pin a
per-box nanosecond floor so the corpus's literal thresholds (`static-armed ≤ 12 ns`,
`dyn-armed ≤ 14 ns`, …) could run.

An absolute-ns threshold is a claim about the **machine**, not about the code. The property the row
is actually protecting is *"the handle carries its arm bit, so the emission path never dereferences
`REGISTRY`"* — a structural property of the implementation, and one that holds or fails identically
on a 2 GHz laptop and a 5 GHz desktop. The A/B measures exactly that: both variants, interleaved,
one thread, one sitting, and a machine that is slow today is slow for both legs. A 2 ns budget over
12 would additionally have to be re-blessed on every box the repository is ever built on, and would
red-light on a busy CI runner for a reason having nothing to do with this code — which is the
failure mode a gate exists to *avoid*, not to demonstrate.

What is kept from the corpus's intent: the absolute figures **are printed**, with the build profile
named beside them, so a human can compare them to the row's numbers whenever they want to. What is
refused is *asserting* on them.

The one caveat that survives is stated at the file: the ratio was measured in `debug` (2.02×).
`cargo test --release` runs the same test and prints the release pair; nothing in the assertion
depends on which profile it ran under, so there is no separate release leg to build.

### `G22b` clause 2 is REWRITTEN against the real symbol, not deleted

**Decision: rewrite, narrowed to the property that can actually fail.**

Deleting it was the tempting option — the clause as written describes a failure the type system
makes unwriteable (`SyncCells<T, N>` takes its extent as a const generic, so no run-time
`ProfilerConfig` value can size one), so the claim is vacuously true. But deleting leaves the corpus
with **no clause naming `assert_zero_init_eligible` at all**, and rung 10 measured that this is
precisely the property that breaks: `ZoneDesc` carries a `&'static str`, cannot be `ZeroInit`, and
`DYN_DESCS` needed a `MaybeUninit` wrapper that reads like ceremony and is easy to delete.

So the clause now reads: *a `.bss` arena declared over a type whose all-zero bit pattern is not a
valid value must fail at compile time; delete `DYN_DESCS`'s `MaybeUninit` wrapper ⇒ `E0277` ⇒ red* —
which is the `trybuild` case already shipped at rung 10 and already blessed. The old sentence's
claim is kept as a **recorded impossibility** with the const-generic argument beside it, so a future
revision does not re-add a gate that cannot fail.

`G22b` clause 1 is untouched and remains BLOCKED on `rustup component add llvm-tools`.

---

## 2026-08-10 — RESOLVED: retire the whole S1.5 harness, phase driver included. Rung 7's mechanical gate is CLOSED.

Owner's answer to both scope questions was the same: retire. Shipped. `rg
'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` now returns **zero code matches** — the rung's
gate, open since the campaign began, is closed.

Two things worth knowing about what the deletion cost, neither of them a decision to make:

**The S1.5 A/B is gone as an experiment, not only as a printer.** `sv0_bench_lighting_flags` drove
`SHADOWS|AO` off on two frames in four; every frame now pushes what every non-bench frame always
pushed. The transcribed numbers stay in `sv0_deferred_term_bench.rs` with its device-free arithmetic
gates, and this repository's own rule applies to them: a result established on a retired instrument
bounds nothing about the current one. Any rung needing a CURRENT Deferred-marcher figure takes a new
measurement on the zone artifact.

**Rung 7 ends with no A/B gate on any GPU family.** `gbuffer_zone_port_gate.rs` went with its leg A,
as `vb_zone_ab_witness_gate.rs` did. That is correct — there is nothing left to compare — and it
means the stage tables under `zone_begin_stage` are pinned by `const` blocks rather than measured
against an independent copy. Named at the function, not left to be discovered.

---

## 2026-08-10 — Rung 7 step 6c attempted and REVERTED: the SV0 bench is not a printer, it DRIVES the A/B it reports.

You answered the previous item with "retire the harness", and that part is settled — the
`window_present_gbuffer.rs` timing leg goes. The deletion still did not land, and the obstacle is a
new one that only surfaced by attempting it. **The tree is back at the last green commit; nothing is
half-finished in it.**

Rung 7 step 2 deleted the VB *printed measurement channel* as pure output. The corpus carries that
same framing forward to the SV0 half, and it is wrong there. `runner.rs`'s S1.5 harness computes
`sv0_bench_lighting_flags` from an ABBA phase counter and threads it into the scene every frame — it
does not merely REPORT the interleaved A/B, it DRIVES it, by changing what the marcher shades.
Deleting the timing channel therefore deletes a **render-path input**, which is a different act from
deleting a printer.

So there is a second question, and it is yours for the same reason the first was:

* **Retire the whole S1.5 harness** — the phase driver with the printer. Its transcribed numbers stay
  in the plan; nothing in the tree reproduces them afterwards.
* **Keep the phase driver, delete only the timing** — the A/B still runs and still changes the
  frame, but nothing measures it. That is a scene input with no consumer, which this campaign has a
  name for: a value nothing can make move.

My recommendation is the first. The second leaves a mechanism whose only purpose was to be measured.

**This blocks the last two collectors and the rung's mechanical gate.** Everything upstream of it is
shipped and green.

---

## 2026-08-10 — Rung 7 step 6 is blocked on a SCOPE call: does `engine_grand_showcase_512_gpu_pass_cost` get ported, or retired?

The last two GPU collectors (`TimestampCollector`, `Sv0TimestampCollector`) cannot be deleted while
something reads their durations, and one thing does: the `#[ignore]`d offline printer
`engine_grand_showcase_512_gpu_pass_cost` in `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`.
Its sibling, `software_ray_baseline_cost.rs`, migrated in ten minutes — it turned out to be using the
collector as a plain array of query pools. This one genuinely reads per-pass timings.

**Why it is not simply a port.** `run_showcase_body_ddgi` builds ONE `GBufferScene` literal (~230
lines) and holds it across the whole timing loop. The zone leg needs `open_frame` (`&mut`) → a
shared borrow parked in `scene.gpu_zone` → present → `retire` (`&mut`), every frame. The shared
borrow's lifetime is in `GBufferScene<'a>`'s type, so setting the field to `None` between frames does
not release it and the `&mut`/`&` cannot alternate. `boyko_app`'s runner never hits this because it
rebuilds the scene every frame; this fixture would have to move a 230-line literal into the loop.

**Three ways out.**

1. **Rebuild the literal per frame.** Mechanical, contained, and makes a 230-line construction run
   200+ times where it now runs once. Nothing measures that construction, so the cost is unknown
   rather than negligible.
2. **Give `GpuZoneRecorder` interior mutability** so `open_frame`/`retire` take `&self`. `FrameSlot`
   already holds two atomics and an `UnsafeCell`, so this is the same change rung 5c made to
   `CommandWitness`, one level down. It is the most reusable answer and the one that widens the
   kernel's surface: anyone holding a `&GpuZoneRecorder` could then claim a ring slot.
3. **Retire the harness.** It is an `#[ignore]`d printer; the zone artifact carries the same four
   brackets; its numbers are already transcribed into the HW-RT plan. This is the option the corpus's
   own precedent points at — `vb_bench_totality_gate.rs` and `vb_zone_ab_witness_gate.rs` were both
   deleted rather than migrated once their subject moved.

**This is a SCOPE call, so it is yours.** My recommendation is **3**, and the reason is that 2 buys a
kernel capability for one caller that a per-frame scene rebuild already gives that caller for free —
but the harness is a published measurement channel for HW-RT R0, and deleting a channel is not mine
to decide. **Until it is answered, rung 7 step 6 stops** and the mechanical gate stays unsatisfiable.

---

## 2026-08-10 — Rung 7's mechanical gate would be satisfied by deleting the record of what it gated. Corrected in the corpus; disclosed here because it is a SPEC change.

The corpus gates rung 7 on `rg 'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` returning zero
matches. After the VB family's half of the deletion the tree has **zero code references and roughly
a dozen prose ones** — `gpu_zone.rs` explaining what its ten `ZONE_VB_*` constants are what remains
of, `command_witness.rs` reconstructing the rung-7c stage defect from the collector that carried it,
`vg_occ_split_timing.rs` naming the channel its table used to read.

`rg` cannot tell a surviving CONSUMER from a comment that records what was deleted and why.
Satisfying the gate literally means erasing exactly the measured history the campaign exists to
keep. **I scoped it to CODE** and wrote the reasoning at the gate — the same scoping the second
mechanical gate already has (`crates/*/src`). Recorded here rather than decided silently because it
narrows a specified gate, and a narrowed gate is the owner's to widen back.

## 2026-08-10 — Deleting the old VB collector deleted the only gate on the stage table. Stated, not repaired.

`zone_begin_stage` says which pipeline stage each VB zone opens at. It had a real gate while
`VbTimedPass::begin_stage` existed as an independently-written second copy: `G10`'s stage clause
compared the two stamp for stamp — 26 frames, 520 timestamps, all identical. That clause is what
caught rung 7c's silently-changed stages after five green commits.

Rung 7 step 5 deletes leg A, so the comparison has no second side. What replaces it is a `const`
block pinning each of the ten ids to the stage both tables agreed on. **It catches a row edited by
hand and cannot catch a bracket moved to a site where the other stage is the right one** — that
question is a measurement, and after this rung it belongs to rung 8. No action is requested; the
loss is named at the function it guards so nobody re-derives the table believing it is checked.

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
  ⚠️ **Narrowed 2026-08-10, and the obvious cause is ruled OUT.** The module already serialises
  itself: `armed()` takes `test_serial()` and hands the `MutexGuard` back to the caller, so every
  test that arms a store holds the module's one lock, and a second explicit site covers the
  plugin test. So the contention is **not** profiling test against profiling test. What remains is
  state global to the PROCESS rather than to the module — `boyko_diag::profiling_abi`'s zone
  `REGISTRY`/`NEXT_SLOT` (slots are minted lazily by `declare_zone!` and never returned) and the
  store's `bind_world` — which any of the other ~880 tests in the same binary can move. Both flakes
  pass 915/915 in isolation and in repeated full-lib runs; they fail only under the full workspace
  sweep. **Next step is to identify which non-profiling test touches the registry**, not to widen
  the module's lock, which is already as wide as the module.
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
- **⚠️ A KNOWN-RED TARGET IS A SHADOW — the workspace gate must run `--no-fail-fast`.**
  MEASURED 2026-08-10. `cargo test --workspace --all-targets` **stops at the first failing target**,
  so `internal_docs_anchors` — red for everyone, long known — had been hiding every target ordered
  behind it. Every "workspace suite green except the pre-existing anchors failure" reported during
  the diagnostics campaign was a claim about **what cargo reached**, not about the workspace. Run
  with the flag, there were **three** red targets, both of the others older than the session that
  found them:
  * `boyko_rhi_vulkan --test compile_fail_frame_write_token` — `26b385eb` (2026-08-06) added one
    line to `token_use_after_submit_rejected.rs` and never re-blessed its `.stderr`; the entire diff
    was `41:5` vs `42:5`. **Red for the 87 commits since.** FIXED by hand-editing the two line
    numbers rather than `TRYBUILD=overwrite`, because that fixture's own comment warns that blessing
    can turn a right-for-the-wrong-reason red into a wrong green — the error kind, the moved value
    and the move site were all verified unchanged first.
  * `boyko_rhi_vulkan --test cluster_bound_arraylength` — VG R3's `vb_batch_cull` (+ its `-D DEBUG`
    sibling) gained an `OpArrayLength` and `BOUND_BY_ARRAYLENGTH` was not updated in that commit,
    which is what the gate's own message instructs. FIXED, and the pin **widened to carry each
    entry's SUBJECT**: the set no longer has one, since those two bound `VbLateCount`'s reserved
    tail slot rather than a froxel light walk, and the old failure text would have given a reader
    advice about a `use_clusters` guard those shaders do not have.
  * `boyko-ui --test p3_watch_zero_alloc` — `watch_nochange_path_is_tiny_and_far_below_reload`
    reported `nochange 1, reload 1`: the "reconciling" tick was reporting the no-change path's cost
    because it WAS the no-change path. The watcher's signature is `(mtime, size)` and the fixture
    rewrote `Px(77)` over `Px(40)` — **the same byte length** — so detection rested entirely on the
    filesystem clock, and this box's mtime granularity swallows the fixture's 3 ms sleep. `4956420c`
    had already diagnosed this class once ("the hot-reload flake was the FILESYSTEM CLOCK, not
    shared state") and answered it with longer sleeps, which buys margin against a granularity
    nobody measured. FIXED by making the rewrite change the SIZE, which removes the dependence
    instead of widening it: `nochange=1 reload=53` after.
  ⚠️ **And my own reporting of this finding was truncated the first time.** The first sweep's
  failure list was piped through `head -10` and I read the truncation as the total — "three red
  targets" when the honest count was six. The same mistake one level up from the one being
  reported. The full picture, measured:
  * **Genuinely red, now FIXED**: `compile_fail_frame_write_token`, `cluster_bound_arraylength`,
    `p3_watch_zero_alloc`.
  * **Genuinely red, NOT fixed**: `internal_docs_anchors` — 25 stale anchors across three internal
    docs plus an over-waiver count above its cap. Pre-existing, unrelated to any campaign here, and
    large enough to be its own unit of work.
  * **Not red at all, but FAILING UNDER THE FULL PARALLEL SWEEP**: `boyko-log --lib` (84/85 in the
    sweep, **85/85** in isolation), `boyko_rhi_vulkan --test sdf_gbuffer_hybrid` (**43/43** in
    isolation, 54 s), and the two `boyko-ecs --lib` profiling tests recorded above. Four targets in
    one class. **The workspace suite is therefore not deterministic**, and a real regression in any
    of them would be indistinguishable from the noise. Every one touches process-global or
    device-global state — profiling lanes and zone slots, logging sinks, the GPU device — which is
    what a `--test-threads` bound or a per-target serial marker would address.
  `CLAUDE.md`'s build-command block now carries the flag and the reason.
- **`.claude/settings.local.json` is dirty** from earlier sessions and is deliberately never staged.

## Rung 9 — `resolve`'s session check refuses every real leg pair, and reports it as `EpochBreak`

**Found while wiring rung 9's correlation, which gives `clock_epoch` a real meaning in this tree for
the first time. Not introduced by rung 9 — surfaced by it.**

`crates/boyko_app/src/profiling/contrast.rs`'s `LegSummary` carried a field named `clock_epoch`
holding `(header.session_lo, header.session_hi)` — a `SessionId`, not
`boyko_diag::clock::clock_epoch()`. Rung 9 renamed it to `session`, because the artifact now carries
a real `cpu_gpu_epoch` and two different things would otherwise have shared one name in one module.
The rename is done. **Two things about the check it feeds are not, and both are the owner's call:**

1. **The refusal reports `NotResolvedReason::EpochBreak` for a SESSION difference.** That is
   defensible in spirit — `clock_epoch()` is a per-process counter, so "both at epoch 0" from two
   processes compares two numbers that mean nothing to each other — but the reason word names
   something the check does not test. `G13`'s sibling clause pins the word `EpochBreak`, so
   renaming it is corpus surface, not a local edit.

2. ⚠️ **On real inputs the check refuses unconditionally.** MEASURED: `resolve` and
   `LegSummary::from_artifact` have **no caller outside `contrast.rs`'s own tests** — rung 8 shipped
   the comparator to *license* later verdicts, not to serve one — and every leg pair a real harness
   would build today comes from two spawned child processes (`vg_decidability_floor.rs`'s protocol
   is seven processes per condition). Two processes have two session ids by construction, so the
   first production consumer of `resolve` will find that it returns `NotResolved { EpochBreak }` for
   every pair it is ever given. The existing tests do not catch this because both legs are hand-set
   to the same value.

**What a fix would have to decide** (not decided here): whether cross-process legs are comparable at
all. If they are — and the whole floor protocol assumes so, since it pools seven sessions — then the
check is wrong as written and the real guard is something else (same `workload_tag`, same box, same
`clock_epoch` *within* each artifact). If they are not, then the floor protocol and this check
contradict each other and one of them is the error.

## Rung 9 — the per-frame ring is now TWO deferrals pointing at one mechanism

The correlation is published once per window with a measured 173 ppm drift across it
(`Correlated::deviation_at_ns` interpolates). Rung 8's per-zone `vkCmd*` counters reach the printed
census but not the artifact, for a structural reason of the same shape (the witness resets each
frame while `retire` yields a frame recorded ~4 frames earlier). Both want a per-frame channel that
does not reduce to medians — which is also what the owner's original ask needs ("break a frame down
by system and pass, catch per-frame spikes"). Recorded so it is built once rather than twice.

## The `hwrt` feature leg does not COMPILE — pre-existing, found by rung 9's clippy sweep

**MEASURED, and proved not to be this campaign's:** `cargo clippy -p boyko-app --lib --features
boyko_rhi_vulkan/hwrt` fails with

```
error[E0063]: missing fields `atrous_layout_denoise_hwrt`, `motion_cam_ubo_ring`, `mv_bind_group`
and 21 other fields in initializer of `boyko_rhi_vulkan::present::GBufferScene<'_>`
    --> crates\boyko_app\src\gpu_scene\mod.rs:6298:25
```

Twenty-four missing fields. Confirmed pre-existing by `git stash`ing every rung-9 source change and
re-running: **identical error on the untouched tree at `71085737`.**

**Why nothing caught it.** `hwrt` is `default = false`, and every gate in this tree —
`cargo check --workspace --all-targets`, the clippy gate, the test sweep — runs the DEFAULT feature
set. The leg is never built, so it can rot without turning anything red. This is the same shape as
the two findings already recorded above (a target ordered behind a known-red one; a virtual manifest
type-checking a subset): **a configuration nothing builds is a configuration nothing gates.** Rung 8
recorded that lesson for `profiling-alloc` and fixed it by making both configurations buildable;
`hwrt` is the larger instance of it and has been un-built for long enough that the drift is
twenty-four fields wide.

**Not fixed here.** It is unrelated to rung 9, it needs `GBufferScene`'s twenty-four `hwrt` fields
understood one at a time, and guessing at them would be worse than the current honest break. **Owner
call:** repair the leg and add it to the gate set, or state that `hwrt` is dormant and stop
implying otherwise.

## Rung 10 — two corpus gates could not run as written, for two different reasons — **RESOLVED 2026-08-10, see the top of this file**

Both are recorded in `docs/diagnostics/profiling/05-LADDER-GATES.md`'s rung-10 record with their
arithmetic. Surfaced here because each is a **decision the owner may want to take differently**, not
merely a note.

**1. `G17`'s absolute nanosecond thresholds were replaced by an A/B ratio.** The row asks for five ns
budgets in one sitting, the tightest pair being "static-armed ≤ 12 ns" against "dyn-armed ≤ 14 ns" —
two nanoseconds of headroom. This campaign measured its own artifact-channel floor at **6.5 %**,
with repetitions spanning 4.7–14.3 %, on GPU passes costing microseconds. A 2 ns budget is inside
that noise by an order of magnitude. What ships instead implements **both** variants — the shipped
gate and the `REGISTRY[id]`-dereferencing one the row names as its RED — and interleaves them in one
process, asserting the shipped one is not slower. MEASURED (debug): 10.89 vs 22.01 ns/iter, 2.02×.
**If the owner wants the absolute thresholds, they need a release-profile bench harness and a
recorded per-box floor for the ns scale** — neither exists, and inventing a threshold without one is
how a gate comes to fail for a background process.

**2. `G22b` clause 2 names a symbol that does not exist and a failure that cannot be written.** The
clause says *"a `#[test]` declaring a `.bss` array sized from a `ProfilerConfig` value must fail
`assert_bss_eligible` at compile time; remove the const-assert ⇒ it compiles ⇒ red."* MEASURED:
`assert_bss_eligible` has **zero hits** in `crates/` (the symbol is `assert_zero_init_eligible`), and
the failure it describes is impossible here — `SyncCells<T, N>` takes its extent as a **const
generic**, so a run-time `ProfilerConfig` value cannot size one whatever any assertion says. There
is no const-assert to remove because nothing needs one. A `trybuild` case gating the property rung 10
DID introduce ships instead (deleting `DYN_DESCS`'s `MaybeUninit` wrapper must be `E0277`). **Owner
call: rewrite the clause against the real symbol, or delete it as satisfied by the type system.**

**And `G22b` clause 1 remains BLOCKED on the same missing tool as `G22a`** — no
`llvm-readobj`/`objdump`/`nm` under the active `stable-x86_64-pc-windows-gnu` toolchain. Rung 10
added two more symbols (`DYN_DESCS`, `DYN_NAMES`) to the set that probe must cover when
`rustup component add llvm-tools` lands, so the D0 line item now unblocks four names rather than two.

## Rung 10 — `G23b`'s literal RED is not producible, and no setting of the constant makes it so

The row's RED: raise `MAX_USER_BUDGET` in the **shipping** profile from 512 to 3072 ⇒ +20 KiB
`REGISTRY` and +120 KiB `DYN_DESCS` ⇒ 1 348.2 KiB crosses the 1 280 KiB budget.

There is no shipping profile: the `BOYKO_PROFILE` axis is rung 14, and MEASURED, **no `build.rs`
exists anywhere in this workspace**. So the residency gate runs at the dev row against a **16 MiB**
budget. And the constant has a hard ceiling that is not a policy choice: zone ids are `u16`, so
`ENGINE_ZONE_SLOTS + MAX_USER_BUDGET` must stay under `u16::MAX`, capping `MAX_USER_BUDGET` at
~61 439. At 8 B/id in `REGISTRY` plus 24 B/id in `DYN_DESCS` that is **~1.9 MiB of growth against a
16 MiB budget** — it does not cross, and nothing a caller can set makes it cross.

An upper bound nothing can push past is a gate that cannot fail. What ships is a **composition
identity** — domain 3 must equal the four `.bss` terms, computed independently — whose RED *is*
producible: dropping `dyn_descs_bytes()` from the sum gives `left: 143360, right: 217088`. That is
the claim the row's own title makes (*"this row is what puts their bytes inside the budget sum"*),
and it is gated. **The budget clause itself stays honest but toothless until rung 14 gives it a
shipping row to be tight against.**

---

## Rung 13 — a second CRC-32 table, and the graph edge that would remove it

**Recorded rather than decided, because both options cost something real and neither is urgent.**

`boyko_diag::telemetry` computes the block CRC with its own 256-entry const table (1 KiB of
`.rodata`). `boyko_image::png` already has one — same polynomial, IEEE 802.3, private, and shaped
for PNG chunks (`crc32_chunk` takes the chunk kind and prepends it).

The duplication cannot be removed by using `boyko_image`'s: `boyko_diag` must keep an **empty
`[dependencies]`**, which is the property that makes it the bottom of the graph. The only direction
that works is the other one — hoist a shared CRC-32 *into* `boyko_diag` and have `boyko_image`
depend on it. That is refused here for now, on the crate's own rule: a checksum is not a diagnostics
primitive, and §4's growth checklist admits a module only when **both** subsystems write it and a
disagreement between two copies would be observable in a joined artifact. Two CRCs over two
different byte streams cannot disagree with each other about anything.

**Cost of leaving it:** 1 KiB of `.rodata` and six lines, twice.
**Cost of hoisting it:** the bottom crate gains a general-purpose utility, `boyko_image` gains an
edge into the diagnostics substrate, and the growth rule loses the property that makes it hard to
satisfy.

Owner's call if the second one is ever preferred. Nothing is blocked either way.

## Rung 13 — `G26`'s budget is a RELEASE claim, and the gate says so instead of pretending

`G26`'s figures (`__telemetry_reduce` p95 ≤ 150 µs, `__telemetry_write` ≤ 200 µs, sum ≤ 350 µs) are
asserted only under `not(debug_assertions)`. MEASURED, this box, 64 quantile zones, p95 over 32
runs:

| | debug | release |
|---|---|---|
| `__telemetry_reduce` | 5 820.8 µs | **128.0 µs** |
| `__telemetry_write` | 163.1 µs | **11.2 µs** |
| **sum** | 5 983.9 µs | **139.2 µs** |

A debug build is **43× over** the total budget. Asserting the budget there would red on every
developer's machine and prove nothing about the shipped one, so what the gate asserts in *every*
profile is the property the budget encodes and a profile cannot change: the reduce dominates, and it
is the term that scales with the quantile count.

~~**The open half:** the release leg is not in CI today. `scripts/` has no release test step, and the
five-profile CI matrix is rung 14's content. Until rung 14 lands, **the budget clause runs only when
somebody runs `cargo test --release`**, and this note is the record that it is not automatic. It is
the same shape as rung 10's `G17` and is expected to be resolved by the same rung.~~

**CORRECTED at rung 14, 2026-08-11: THAT PARAGRAPH WAS FALSE WHEN IT WAS WRITTEN.**
`.github/workflows/ci.yml`'s `test` job is a `matrix: profile: [debug, release]`, and its release
arm has run `cargo test --workspace --all-targets --release` since long before rung 13. A second
job, `force-alloc-panic`, runs the release suite again under `--cfg force_alloc_panic`. Neither
excludes `boyko_app`, so `G26`'s budget clause has been running in CI on every push the whole time.

The defect is not the conclusion, it is **where I looked**: I checked `scripts/` for a release step,
found none, and reasoned from that to a claim about CI — without opening the CI file. That is the
root cause this corpus has already written down in as many words: *verification is an ACTION, not an
understanding*, and errors land exactly where checking something would have meant doing something.
`scripts/` and `.github/workflows/` are two different places and only one of them was read.

The note is struck through rather than deleted because the false claim is the useful half. Rung 14's
`profile-legs` matrix is still net new and still worth having; what it does **not** do is close a gap
that was never open.

## Rung 13 — `W9214` has an emitter and a doc page, but no test observes it

`W9214` (telemetry path unwritable) is `Live` in the registry, is raised by `TelemetryStream::open`
and has a `docs/diagnostics/W9214.md` page. Checks 2, 3a and 3b all pass. What does **not** exist is
a test that observes it being emitted, which is the obligation every `Live` row owes.

The reason is that producing it needs an unwritable path, and the ways to get one are all
platform-specific and flaky in CI: a directory that does not exist works on both platforms but is
the least interesting case; a read-only file needs `chmod`/`icacls`; an open-without-sharing needs a
second handle and is Windows-only.

`W9215` and `W9218` **are** observed (`crates/boyko_app/tests/profiling_telemetry_stream.rs`), so
this is one row rather than three. Recorded rather than papered over with a
directory-does-not-exist test that would pass on a typo.

## Rung 14 — `profiling-analysis` is now OPT-IN, and that changes what a plain `cargo build` gives you

**This one is a behaviour change and the owner should know about it before it surprises him.**

`boyko_ecs` used to declare `default = ["profiling-analysis"]`, so every build carried the interval
ring and `ConcurrencyReport` — the "did this schedule actually run in parallel?" answer. It is now
`default = []`, and that answer requires `--features boyko-ecs/profiling-analysis`.

**Why it had to move**, measured rather than argued:

- An environment variable cannot set a cargo feature. Cargo resolves features before any build
  script runs, and `cargo::rustc-cfg` reaches only the crate that emitted it. So `BOYKO_PROFILE`
  could never have switched it, whatever the corpus's table says.
- While it was default-on, **no command line could turn it off**. `cargo tree --workspace -e
  features --no-default-features` still reported it enabled: **nine** sibling manifests depend on
  `boyko-ecs` and not one says `default-features = false`, so unification restored it. Moving the
  request onto a dependency edge would not have helped either — an explicit `features = [...]`
  survives `--no-default-features` by design.
- So the axis's `shipping` row would have been a claim nothing could honour, and `G14(c)` would have
  been a gate with no reachable state.

**What replaces it:** the axis emits `ANALYSIS_ADMITTED`, and `boyko_ecs` refuses at compile time to
be built with the feature on under a profile that does not admit it. The refusal is one-way on
purpose — analysis missing from a `dev` build is a developer who passed fewer flags than they meant
to; analysis present in a `shipping` build is the profile being a lie.

**A coverage consequence, found by asking rather than by a gate.** Six tests in `boyko_ecs`'s
profiling suite are `#[cfg(feature = "profiling-analysis")]`. Opt-in means a bare
`cargo test --workspace --all-targets` no longer compiles or runs them — and a sweep that silently
stops running six tests looks exactly like a sweep that passes. Every CI leg that used to get the
feature from the default list now names it explicitly (`check`, both `test` arms, `clippy`,
`force-alloc-panic`), and the feature-OFF side is covered by the four `profile-legs` builds that
refuse it outright. **A local sweep must pass `--features boyko-ecs/profiling-analysis` too, or those
six do not run.**

**The owner's call, if he wants one:** whether a plain local `cargo build` should carry it. The only
way to have both is to accept that no flag can remove it, which is the state we just left.

## Rung 14 — the symbol census needs `lto = "fat"`, and that raises a separate question about the shipped profile

`G14(a)` is only decidable under LTO. MEASURED on this box, `deep_zone` (one `Deep` zone site,
`boyko_diag` its only dependency):

| link configuration | `dev` | `shipping` | can the gate fail? |
|---|---|---|---|
| default release | `mint_cold` = 1 | 1 | **no** |
| `-C link-arg=-Wl,--gc-sections` | 1 | 1 | **no** — no effect at all |
| `lto = "fat"`, `codegen-units = 1` | 1 | **0** | yes |

The default-release image contains `drop_glue::<boyko_diag::telemetry::Block>` in a binary whose
source never mentions telemetry: the whole rlib rides in and nothing collects it, so a whole-image
census answers "was this codegen'd on the way here?" rather than "can this program reach it?".

The gate passes LTO through `--config` for its own two builds only, so nothing else in the
repository changed. **The open question is a different one:** `[profile.release]` here sets
`codegen-units = 1` for benches and no LTO anywhere. A shipped title almost certainly wants
`lto = "fat"` — it is the difference between `deep_zone` at 1433 symbols and at 4647. That is a
build-configuration decision with compile-time cost, and it belongs to the owner rather than to a
diagnostics rung.

## Rung 14 — `BOYKO_PROFILE=off` does not turn the profiler off, and the thing that would does not exist

`SEAM.md` §S9's table gives the `off` row the tier-column entry *"feature `profiling` off"*.
MEASURED at this rung: **there is no `profiling` cargo feature anywhere in this workspace.**
`boyko_diag` declares `section-gate`; `boyko_ecs` declares `profiling-analysis`, `big_query_table`
and `bench-alloc`; no crate gates `zone!` or `declare_zone!` on a feature at all. And `ZoneTier`'s
three values are `Always`, `Dev` and `Deep` — there is no position below `Always`, so the lowest
compile ceiling the profiler has still admits every `Always` site.

`off` therefore ships as a **logging** off switch: `LOG_CEILING = 0`, `LANE_ARRAY_LEN` becomes
zero-length, which is `G2`'s subject and works. The profiler stays at its floor.

Two ways to close it, both out of this rung's scope: land the FEATURE axis (`G1`) so a `profiling`
feature exists and `#[cfg]` can delete the macro definitions before name resolution; or accept that
"off" means the runtime axis (`ARM_MASK`, `GJ1`) and rename the row. Nothing is blocked either way —
the row is honest as built, and it is written down here because "off" is a word a reader will trust.

## Rung 14 — J1's logging half is owed, and it is owed to rungs that have not landed

`J1` is one rung by construction (S9: one compile axis cannot be split across two rungs), and the
axis is now whole. `L17`'s **other** content is not:

- `LogRuntimePreset` — the five-preset runtime axis.
- The three header facts: `build_profile`, `runtime_preset` and `ceiling` printed as three
  independent values, plus a fixture proving the first two can differ in one binary.
- `G16(d)`, which is the gate over those three fields.
- A **dynamic** logging site in the census fixture. `dyn_debug!` is `L10`'s and does not exist, so
  `G16(a)/(b)` covers the static path only.

All four need a sink header to print into. MEASURED: `boyko_log` has `census`, `codes`,
`drain_owner`, `lane`, `level`, `lifecycle`, `macros`, `rate`, `record`, `site`, `sync_out`,
`target` and `sink/{ecs,file,mod}` — and **no** `sample.rs`, `sink/binary.rs`, `sink/request.rs`,
`sink/crash.rs` or `bin/logdec.rs`. The logging ladder stands at roughly L5 of 17.

This is not a defect in the axis and not a shortcut taken: the axis is the indivisible part and it
landed indivisibly. It is a scheduling fact, recorded so nobody reads "J1 shipped" as "the logging
plan reached L17".

## Rung 16 (J2) — REFUSED ON THE MEASUREMENT: the "both-present" configuration does not exist

J2 is the joint baseline sitting: re-take `zone_cost`, `fold_cost`, `P1` and `P2` **with the profiler
and the logger both present**, in one sitting, and run `GJ1` (the measured off-cost) there. Attempted
after rung 15 and **refused**, because the configuration it is supposed to baseline is not one this
workspace can currently be in. MEASURED:

| | measurement |
|---|---|
| `boyko_log::{error,warn,info,debug,trace}!` across every crate's `src/` | **2 hits, neither an emission site** — a *comment* in `boyko_log/src/lib.rs:86` and rung 15's own `profile_fixture_log` |
| callers of `boyko_log::enable` / `boot` | **none** — no sink thread, no consumer, no panic hook |
| manifests depending on `boyko-log` | **two** (`boyko_ecs`, `profile_fixture_log`); absent from `boyko_app`, `boyko_render`, everything that runs a frame |
| non-test callers of `Profiler::arm` | **none** — rung 11 measured this and it is unchanged |

So `GJ1`'s leg **A** — *"profiler armed, logger enabled, at the shipping ceiling"* — cannot be built,
and legs B and C are defined relative to it.

**Why this is a refusal and not a deferral.** The tempting move is to take the sitting anyway and
stamp the files `both-present`. That is strictly worse than having no baseline: every later
regression gate compares against these files, `config_tag` is what tells a reader the comparison is
legitimate, and a tag that says `both-present` on a run with neither present makes every one of those
gates confidently wrong. The corpus's own rule — *"whichever subsystem landed second must not be
measured against a baseline taken without it"* — is exactly the rule being obeyed here.

**Consequence, stated so it is not read as progress:** the `UNPROVEN` state REMAINS IN FORCE. The
+25 % gate, the revert clauses and `GJ1` still record `UNPROVEN` and still may not fail a rung.

**Preconditions, so the rung can be re-entered rather than re-argued:**

1. Logging **L3** — the sink thread, `enable`, the drain. Without a consumer there is nothing to
   measure the cost of.
2. Logging **L6–L8** — the migration that gives the engine emission sites at all. Today it has none,
   so "logger on" and "logger off" are the same binary doing the same work.
3. ~~A **non-test arm path** for the profiler, so "profiler armed" is a state a shipped frame
   reaches.~~ **DONE, and the note above UNDERSTATED the problem.** It said `Profiler::arm` had no
   non-test caller. Measured immediately afterwards: **`ProfilerPlugin` was added nowhere outside
   tests either** — so the store was not merely unarmed, it was never *inserted*, and fifteen rungs
   of profiler were unreachable from any host. `App::update_with_delta` had been calling
   `fold_frame` all along; it found no `Profiler` and returned. `EnginePlugins` now adds the plugin
   unconditionally — safe by the store's own design, `Profiler::new` *"reserves nothing, commits
   nothing, calibrates nothing"* — and `BOYKO_PROFILE_ON` arms it, which is `SEAM.md`'s route (a).
   Gated by `crates/boyko_app/tests/profiling_host_reachable.rs` (installed + disarmed) and
   `profiling_host_arm_flag.rs` (the flag arms it), both REDs shown.

   **The shape is worth keeping separately from the fix.** Every one of those fifteen rungs was
   green, and none of them could see this: each gate builds its own world and inserts its own store,
   so "does a HOST have one?" was a question no test in the campaign was asking. A subsystem can be
   fully gated and entirely unreachable at the same time.

   **And a second constraint fell out of writing the gate:** `EnginePlugins` **cannot be built twice
   in one process** — the second build panics in
   `register_component_hooks::<boyko_render::light::DirectionalLight>`, because component hooks are
   process-global and the derive's installation is not idempotent. That is why the two legs are two
   test *binaries*. It is pre-existing, it belongs to the render plugins rather than the profiler,
   and it is invisible until something builds the host twice.

**And one defect to fix before the rung, not during it: `config_tag` is already taken.**
`boyko_app::profiling::artifact::config_tag` exists and returns a `String` FNV-1a hash of
`boyko_render::ResolvedRenderPath`'s `Debug` — it identifies the **render path** (Deferred/Forward/VB
× Both/Mesh/Sdf), and `ArtifactHeader::workload_tag` is built from it. S10 asks for a field of the
same name meaning `{profiler, logger}`, in the same crate. Landing it under that name would put two
facts under one identifier, and the failure mode is specific: a reader compares a VB baseline against
a Deferred one, the tag matches, and the difference is reported as a regression. The J2 field needs a
different name (`diag_tag`, say) or the render one does.

---

## L8b — VALUES: L6/L7/L8a silenced 31 diagnostics that used to print unconditionally, and no document says so

**This is not a bug report.** The behaviour is specified and it is gated. It is an owner call about
what a default run of this engine tells its operator, and it is raised here because the migration
rungs took the decision as a side effect of a cost argument rather than as a decision.

**Measured, in this order:**

1. `boyko_app::plugins::boot_and_enable_logging_from_env` calls `boot()` unconditionally and then
   **returns before `enable()`** when `BOYKO_LOG` is unset. `CONTROL` stays `.bss`-zero, so every
   target's runtime ceiling is `Off`.
2. So a migrated `warn!`/`error!` in a default run produces **nothing** — not a dropped record, not
   a counted loss. The macro's third gate is false and the site is one predicted branch.
3. `git show` on the three migration commits: **31 `println!`/`eprintln!` lines were removed from
   production sources** — 3 at L6 (`49cf2230`), 12 at L7b (`b30fa810`), 16 at L8a (`1a76e4a9`).
   Spot-checked against the parent commit, `boyko_render`'s `W2201` site was an **unconditional**
   `eprintln!` behind a one-shot latch, not an env-gated or `debug_assertions`-gated one.
4. The silence is **deliberate and pinned**: `logging/sink-lifecycle` Decision 25 states *"a
   flag-off run of any other preset configures nothing either, because `enable()` never ran and no
   sink slot was ever opened"*, and `crates/boyko_app/tests/log_host_reachable.rs` asserts
   `flush() == NoConsumer` after a full `EnginePlugins` build, calling it *"the half that makes the
   cost claim true rather than merely stated"*.

**What no document in the corpus says** is what (4) does to (3). The plan gated the *cost* — one
sink thread, a 20 ms clock calibration in `enable()`, a panic hook — and in doing so gated the
*diagnostics*, and the migration rungs then converted 31 unconditional prints into records behind
that gate without the trade being written down anywhere.

**The question, stated as a fork:**

* **(A) Diagnostics stay opt-in** (today's behaviour). A shipped run is silent until an operator
  sets `BOYKO_LOG`. Cost: nothing. Consequence: a `Warn` nobody sees is a `Warn` that does not
  exist, and the engine's 24 Live `W`/`E` codes are documentation rather than diagnostics.
* **(B) The host enables at a `Warn` floor unconditionally**, and `BOYKO_LOG` raises it. Cost:
  ~20 ms of clock calibration and one sleeping thread per process — including every child process
  the test suite spawns. Consequence: the pre-migration behaviour is restored and the codes become
  reachable without foreknowledge.
* **(C) Wire the synchronous route.** `TargetControl::SYNC_BIT` is declared, `sync_out` exists, and
  `emit_unlaned_line` already renders and writes through it — but the bit **has no reader**:
  `lane.rs`'s only synchronous path is the no-lane fallback, not a route the control byte can
  select. Wiring it would let `error!` reach `stderr` with no thread and no calibration. This is
  the architecturally right answer and it is L12-shaped work, not L8b's.

**L8b did not wait on this.** The three terminal-exit codes (`E3002`/`E3003`/`E3004`) fall back to
`eprintln!` when `flush()` answers `NoConsumer`, on `boyko_threadpool::worker`'s already-blessed
precedent — so the host cannot exit silently under any of the three answers. The degrade codes and
the thirty `info!` sites follow (A) as specified. **The 31 already-silenced sites from L6/L7b/L8a
are untouched and remain silent**, which is what this entry is about.

---

## L8b — `boyko_app` never calls `flush()` or `shutdown()`, so SEAM S5's teardown half does not exist

Measured: `boyko_app` names `boyko_log::lifecycle` in exactly one place, `plugins.rs`, and calls
`boot` and `enable`. There is no `flush()` and no `shutdown()` anywhere in the crate.
`SEAM.md`'s S5 gives `boyko_app` *"the boot and teardown order, `flush_gpu` ahead of `flush`"*; the
boot half landed at L7 and the teardown half never did.

The consequence is not theoretical. `lifecycle::enable` spawns the sink thread and **drops its
`JoinHandle`** (`.spawn(sink_loop).map_or_else(…, drop)`), so nothing joins it. A record emitted
just before `return AppExit(true)` races the drain and loses more often than not — and the sites
L8b migrates are exactly the print-then-exit ones.

L8b's three terminal reporters call `flush()` themselves, so those records leave. **Every other
record emitted late in a run is still exposed**, including the `VB-ZONE summary` and artifact lines
that a measurement run ends with. The fix is a `flush()` on the normal teardown path and a
`shutdown()` after it, and it wants doing with the rung that owns the lifecycle rather than bolted
onto this one.

---

## L8b — deleting `boyko_demo`'s `log` facade left two channels with no replacement

The ledger specifies the deletion of `log = "0.4"`, `env_logger` and `console_log` from
`boyko_demo`, and L8b did it. Two things went with them:

* **Native**: `env_logger` was the only subscriber for the `log` facade in that binary, and
  `eframe`/`egui`/`wgpu`/`naga` all emit through it. **wgpu adapter selection and validation
  messages now go nowhere.** The replacement is a `log`-facade bridge feeding `boyko_log`, which no
  rung owns.
* **wasm**: `console_log` was the only channel reaching the browser console, and `boyko_log`'s
  console sink writes to `stderr`, which is a no-op on `wasm32-unknown-unknown`. So `E3001` — the
  record whose entire purpose is to explain a blank canvas — is emitted and unreachable there.
  It costs nothing **today**, because the wasm build is blocked upstream in `boyko_ecs` (the layout
  asserts fail 32-bit const-eval) and its CI leg is explicitly non-fatal. Whoever unblocks wasm
  owes the console sink a `web_sys::console` arm, or the failure goes back to being silent.

Both are recorded in `crates/boyko_demo/Cargo.toml` beside the dependency, so the next reader of
that manifest finds them without finding this file.

---

## L8c — four `Pending` code rows name profiling rungs that have SHIPPED, and all four conditions exist and are silent

Check 3c (`Pending == 0`) is L8c's, and it cannot arm while these four rows stand. Measured against
HEAD, each condition **exists in the tree and reports nothing**:

| row | condition, located | state |
|---|---|---|
| `W9202` `Pending("profiling 5")` | `boyko_rhi_vulkan::present::gpu_zone::alloc_pair` returns `None` once `used_pairs >= MAX_GPU_PAIRS` (128) | the bracket is simply unrecorded; nothing reports it |
| `W9217` `Pending("profiling 5")` | `runner.rs` calls `flush_vb_zone` **only** inside `vb_zone_seen >= WARMUP + frames` | a run that ends earlier — window closed, or `E3003`'s terminal `return` — leaves slots in flight, unflushed, unreported |
| `W9205` `Pending("profiling 8")` | `reduce.rs` increments `census.lost`; `contrast.rs` reads it as `window_complete` | counted, carried into the artifact, never warned about |
| `W9206` `Pending("profiling 8")` | `contrast.rs` has `NotResolved` + `NotResolvedReason` fully built | a refusal is returned; nothing warns |

**`GpuZoneRecorder::flush` itself is correct** and labels every in-flight slot `Flushed` — its own
doc says it exists so *"the last `GPU_RING_DEPTH` slots would [not] be dropped silently, which is
the loss a profiler exists to report rather than to commit"*. `W9217`'s hole is not in `flush`; it
is in the one path that never calls it.

**Also measured, and it is the shape of the thing:** `boyko_app::profiling` contains **zero**
`warn!`/`error!` calls across fifteen shipped rungs. The `92xx` emitters all live in
`boyko_ecs::…::profiling::diag`, which its own header names as the **sole** emitter of the block —
*"which is what keeps a profiler drop reported as a counter read rather than as a log record that
can itself be dropped under exactly the load that produced the drop"*. So these four do not become
`warn!` at the condition site: they route through `boyko_diag::loss::raise(DiagFlag::…)` sticky
bits that `diag.rs` reads. `flag_code`'s `match` is deliberately not `_`-terminated, so a new
`DiagFlag` variant **fails to compile** until it is paired with a code — the mechanism is already
built and simply has four unused inputs.

**The question for the owner is not how, it is whether these belong to L8c at all.** They are
profiling conditions, in profiling crates, whose rungs are marked shipped. L8c inherits them only
because `Pending == 0` is its gate. Either the profiling ladder reopens rungs 5 and 8 to land the
emitters it reserved codes for, or the four rows are re-dispositioned. Recorded rather than decided
because it moves work between two ladders.

---

## ~~L13b's revert clause has fired~~ — RESOLVED 2026-08-17: keep L13b, the 5× was an estimate

**OWNER RULING: keep L13b. The `5×` was an estimate, not a requirement.** Recorded below as it was
asked, because the measurement is the reason the threshold moved and a resolved question that
deletes its own evidence teaches nothing.

**What changed in the tree**: `02-SINK-LIFECYCLE.md`'s clause is re-cut from an acceptance
threshold into a **regression guard** at `≥ 4.0×` and `≥ 3 M rec·s⁻¹`, set from the four readings
and deliberately below the observed minimum rather than pinned to it — a bound at today's number
reds on ordinary variance, and a gate that cries wolf gets ignored. The bench prints `PASS` /
`REGRESSION` instead of `PASS` / `FAIL (revert clause)`, and its RED was shown by raising the guard
to `6×`.

**The lesson the corpus keeps**: the `5×` was written before anything was measured and nothing was
ever measured against it until the bench existed. A number invented in advance is a guess about the
answer; this corpus does not get to hold a guess and a measurement in one sentence and call the
guess the requirement.

`02-SINK-LIFECYCLE.md` states the clause without hedging: *"the entire justification is throughput.
If `sink_sustained_rate_binary` does not measure ≥ 5× `sink_sustained_rate` in the same sitting,
**L13b is reverted**. A format whose only reason to exist is speed must show the speed."*

The bench now exists (`crates/boyko_log/benches/sink_sustained_rate.rs`) and it was built to answer
exactly this. Four sittings on this box:

| sitting | text ns/rec | binary ns/rec | ratio | A-vs-A' twin gap |
|---|---|---|---|---|
| 1 | 41.02 | 9.54 | **4.30×** | 0.020 ns |
| 2 | — | — | **4.63×** | 0.085 ns |
| 3 | — | — | **4.68×** | 0.120 ns |
| 4 | — | — | **4.54×** | 0.080 ns |

**The instrument is sound and the result is not marginal noise.** The A-vs-A' twin — the same leg
measured twice around the other — drifts by 0.02–0.12 ns while the legs differ by ~31 ns, so the
sitting is not drifting. The separation is ~31 ns against a combined spread floor of ~1 ns, so it
resolves. Four independent sittings land in a 0.38× band, none of them touching 5×.

**The absolute half of the clause passes by a wide margin**: 104.8 M rec·s⁻¹ against a floor of
3 M. It is only the *ratio* that misses.

**And the measured scope is the one most favourable to L13b.** The bench times only where the two
paths differ — `render_payload` against `encode_record` — because everything upstream of the drain
and downstream of the sink's `write` is shared. An end-to-end sink rate would add a constant both
formats pay, which can only push the ratio *down*. So 4.5× is an upper bound on the end-to-end
figure, and the clause still misses.

**The three dispositions, and why this is not mine to pick:**

1. **Revert L13b as written.** The clause is unambiguous and the number is reproducible. Costs:
   `binary.rs`, its dictionary, `W0116`, the format tests and the offline decoder plan all go.
2. **Keep it and amend the threshold.** 4.5× at 105 M rec·s⁻¹ is a real improvement; a 5× line
   drawn before anything was measured is not obviously the right line. This requires the owner to
   say the threshold was the estimate, not the requirement.
3. **Keep it and make it faster.** The text leg's 41 ns is dominated by `core::fmt`; the binary
   leg's 9.5 ns is already close to a `memcpy` of 39 bytes. The ratio is more likely to move by
   *slowing nothing and speeding the text leg less* than by optimising the binary one — i.e. this
   route probably does not reach 5× without changing what the text sink does.

Each of these trades shipped, tested, documented code against a number, which is a values call, not
a performance fork. Recorded and surfaced rather than decided.

### Correction to the readings above, made after the instrument was fixed

The four sittings quoted in the table were taken with a spread floor that was **2 % of the reading
and nothing else** — the IQR was exactly zero, so `se.max(med * 0.02)` reported the subject's size,
not the clock's resolution. Fixing that (`benches/instrument.rs`) and re-measuring gives a fuller
picture:

* **Eleven sittings on an idle box: 4.29× – 4.68×.** The original four sit inside that band, so the
  ruling rests on the same evidence it always did, and the `4.0×` guard keeps its margin.
* **One sitting read 5.94×**, taken immediately after a build with the machine still busy. It is
  not a contradiction: the TEXT leg is the load-sensitive one, so load inflates the ratio.
* **The A-vs-A' twin test was too lax and has been re-cut.** It compared the twin gap against the
  ~32 ns *separation*, which admitted a sitting that drifted 3.4 ns on a 41 ns leg — 8 %, enough to
  move the reported ratio by ~0.35, wider than the entire band. It is now proportional: the twin
  must agree to within 2 % of the leg.
* **That change caught a false red.** A drifted sitting produced `2.61×`, which under the old test
  would have been reported as `REGRESSION` — an accusation against a format that had lost nothing.
  It is now correctly `NOT MEASURABLE (instrument)`.

The direction of the ruling is unaffected. What changed is that "reproducible" is now a claim about
an idle box with a drift-rejecting twin, rather than a claim resting on byte-identical numbers that
were byte-identical because the floor was fictional.

---

## `Once` is now the ONLY policy honoured by hand, and 39 sites do not obviously honour it

**Status:** OPEN — measured 2026-08-19, not fixed. Raised because the fix is a rung, not a footnote.

`rate::admit` is wired (`__log_rate_admits!`, the fourth gate). `Every`, `EveryN` and
`MinIntervalMs` are now applied mechanically by the emission macros. `Once` and `OnceCounted` are
NOT, deliberately: the latch stays a named `OnceSite` the site declares, because a `static` inside
a macro expansion cannot be named and `OnceSite::reset` exists precisely so an observer can reset
the latch it is about to test.

That leaves `Once` as the last policy whose declaration is kept by human diligence — which is this
campaign's signature defect shape. **Measured across `crates/**/*.rs` and `src/**/*.rs`, excluding
`codes.rs`, `tests/` and `benches/`: 45 `Live` rows declare `Once`/`OnceCounted`, and 39
(identifier-use, file) pairs have NO `OnceSite` anywhere in the file.** Two were read by hand and
are real:

* **`W0111` (`crates/boyko_log/src/census.rs:122`, `report_unsunk`)** — `#[cold]`, no latch, called
  from inside `census::rows()`, which is a **public iterator** any host may walk per frame. Its own
  doc comment says `Once`, "because the condition is a CONFIGURATION and not an event". A per-frame
  census overlay would emit it once per unsunk target per frame.
* **`E0109` (`crates/boyko_log/src/sink/crash.rs:82`, `report_unopenable`)** — `#[cold]`, no latch.
  It fires once today only because `arm()` is called once on the enable path; the row's `Once` is
  honoured by the CALL STRUCTURE, not by anything at the site.

The crude scan cannot tell an emitter from a mention (a `use`, a doc link, a test assertion), so 39
is an upper bound and the real count needs an emitter-aware walk — the shape `code_registry.rs`'s
existing checks already have.

**Two dispositions, and the second is the one that needs a ruling:**

1. **Audit the 39 and place the missing latches.** Mechanical, site by site, and each site's
   correct granularity is a judgement (`W2205` deliberately keeps its latches in a `Resource`, not
   a `static`, so one world's first divergence cannot silence another's).
2. **Place the latch in the macro after all, and make it resettable.** The objection above is
   testability, and it is answerable: register every macro-placed latch against its `&LogSite` in a
   walkable table and give `test-probe` a `reset_all_once_sites()`. Then all 45 rows are honoured
   mechanically and an observer resets everything before driving its site. This changes behaviour
   at 45 rows and costs `.bss` plus a registration on first emission, so it is a scope call.

Recorded rather than decided.

**Addendum (same session): the corpus's own accounting for `Once` is not built either.**
`00-GOAL-TARGETS.md:37`, `01-EMISSION-RING.md:273` and `05-LADDER-GATES.md:918` all specify an
`ONCE_SITES` intrusive list and one `LOG-ONCE` census row per fired site
(`code=W2102 site=device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)`). **Neither exists.** Two
doc comments in the tree named it as though it did — `crates/boyko_log/src/rate.rs` called it "the
`ONCE_SITES` walk's answer", and `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` claimed a
site "enrols itself in `ONCE_SITES` so the `LOG-ONCE` census can report that it fired at all". Both
corrected in place with the wiring commit.

This is the same finding as the 39 latch-less sites, from the other end: **nothing enumerates
`Once` sites, so nothing could notice.** Building `ONCE_SITES` + the `LOG-ONCE` rows is the natural
pair to disposition 1 above — the audit needs the enumeration, and the enumeration makes the audit
mechanical instead of a grep.

---

## UPDATE (same day): the enumeration is BUILT, two of the sites are fixed, and the audit is now a number

`crates/boyko_log/src/once_sites.rs` is the register the corpus specified. The drain notes every
emission from a site whose `LogSite::rate` is `Once`/`OnceCounted` — off the emitting thread, from
cold `'static` data — and `census::print` emits one row per fired site:

```
LOG-ONCE code=W2102 site=crates/boyko_rhi_vulkan/src/device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)
```

**`fired > 1` is the defect, stated as a number.** The row even says so:
`  <-- DECLARES Once AND HAS NO LATCH`. The 39-pair grep was an upper bound with no way to tighten
it — it cannot tell an emitter from a `use` or a doc link — and this replaces it with a per-site
run-time count.

**Two engine sites are fixed with it, and the first was found BY it:**

* **`W0111`** — `report_unsunk` was called from inside `census::rows()`, a **public iterator** a
  host may walk every frame. Reverting the fix and walking `rows()` ten times makes the register
  read `fired: 10` for `census.rs`. The report moved to `census::print()` (flush and shutdown)
  behind a named `UNSUNK_REPORTED` latch. **A query must not have a diagnostic as a side effect**,
  which is the general form of this defect.
* **`E0109`** — `report_unopenable` had no latch; its `Once` was honoured by the call structure
  (`arm` runs once on the enable path), so a process that disabled and re-enabled would report
  again. It has a named `UNOPENABLE_REPORTED` latch now.

**What is still open.** The remaining ~37 pairs are not audited: the register only reports sites
that FIRE, and a site that never fires in a test run leaves no row. Reading it properly means
running the engine and reading the census, which is the next step rather than a grep. The
macro-auto-latch question (disposition 2 above) is untouched and still needs a scope call.

---

## The certification recipe never runs 311 tests, and 177 of them do not say why

**Status:** OPEN — measured 2026-08-19. Surfaced rather than half-fixed, because the fix is a
policy call.

The two-half recipe in `CLAUDE.md` reports green without running a single `#[ignore]`d test:

```
cargo test --workspace --exclude boyko_rhi_vulkan --all-targets --no-fail-fast
cargo test -p boyko_rhi_vulkan --all-targets --no-fail-fast -- --test-threads=1
```

Measured across `crates/`, `src/` and `tests/`: **311 `#[ignore]` sites in 90 files.**

**Do not read that as 311 defects.** Of the 134 that state a reason, 122 name a GPU, device,
window or swapchain requirement, 3 name process-wide state and 2 name a dump or golden — all
legitimate reasons for a test to be driven by hand. The number worth acting on is the other one:

**177 sites carry `#[ignore]` with NO reason at all.** They sit in 78 files, mostly windowed/GPU
suites whose module docs do explain the requirement — so 177 is an upper bound on "silenced with
no record", not a defect count. But a bare `#[ignore]` cannot be told apart from a test that went
red once and was quieted, and that is precisely the distinction this campaign exists to make
mechanical.

**One instance is verified and load-bearing right now.**
`crates/boyko_log/tests/l14_sink_policy.rs`'s `an_armed_target_no_sink_accepts_is_unsunk_and_says_so`
is the **only** observer of the `W0111` latch this rung introduced, and the recipe does not run it.
It was run by hand for this commit (`-- --ignored --test-threads=1`, green) and that is a process
step, not a gate.

I did not merge it into its sibling: the second test re-`boot`s and re-`enable`s, and `enable()`
on an already-enabled process is a different code path — a merge could produce a test that passes
for a new and wrong reason, which is worse than one that does not run.

**Two questions for the owner:**

1. Should the recipe gain a third leg, `-- --ignored --test-threads=1` per crate — accepting that
   the GPU/windowed suites will then need a device present?
2. Should a bare `#[ignore]` become a tidy-check failure, so that switching a gate off always
   leaves a written reason? That is the same rule as the mandatory `// SAFETY:` comment and the
   `#[allow(clippy::disallowed_types)]` rationale, applied to the third way to make a check
   disappear.

---

## RESOLVED: the 39 was an upper bound, and the sharpened count is 19 latched of 20

**Status:** CLOSED 2026-08-19 — the residue is named below and is small.

The entry above reported "39 (identifier, file) pairs carry no `OnceSite`" and said in the same
breath that it was an upper bound with no way to tighten it, because a grep for identifier USES
cannot tell an emitter from a `use`, a doc link or a test assertion. It has now been tightened, and
the answer is different in kind:

**Emission-aware, production-code-only: 20 files emit a `Once`/`OnceCounted` code. 19 hold a latch.
The one flagged was `boyko_log/src/macros.rs`, whose `warn!` DOC COMMENT explains the class/number
pairing using `W2102` as its example** — not an emission at all, and gone once the scan reads the
production stream instead of raw text.

**A middle draft was wrong in the other direction and is worth recording.** Requiring the latch
inside the emitting FUNCTION returned 21 — of which **19 were correct code**:
`boyko_ecs::…::profiling::diag` holds one `OnceSite` per live code in a `LATCHES` array behind a
`claim(number)` helper, so no emitter there names `.claim()` itself. A gate written that way would
have accused nineteen sites that latch properly. Measuring before writing the check is what caught
it.

**Two real defects came out of this and are fixed**, both found by the run-time register rather than
by either grep: `W0111` emitting from inside `census::rows()`, a public iterator a host may walk
every frame; and `E0109`, whose `Once` was honoured by the call structure rather than by anything at
the site.

**The gate is `check_8_every_file_emitting_a_once_code_has_a_latch`** in
`crates/boyko_log/tests/code_registry.rs`, with an anti-vacuity floor (a scan finding fewer than ten
emitting files fails rather than passes) and two REDs shown.

**What it still cannot prove**, stated in the check's own doc: that the latch guards THAT emission
rather than another in the same file. That residue is `boyko_log::once_sites`' at run time, where a
`Once` row reading `fired > 1` is the defect stated as a number.

**Disposition 2 above — placing the latch in the macro — is therefore NOT needed** and is withdrawn
as a question. The human link is now checked mechanically at build time and observably at run time,
which is what auto-latching was going to buy, without making all 45 rows untestable in isolation.

---

## 2026-08-20: the census reaches only the console — a `shipping` log never carries its own loss summary

**Context.** The windowed runner now ends the diagnostics session (`lifecycle::shutdown()` at the
end of `run_windowed`), so `close_out` runs in every real host: the final drain delivers the lane
tail and `census::print()` fires. Measured on live 16-frame runs of the `clear` example.

**The question.** `census::print` writes through `sync_out::write_oracle_line` — the synchronous
console channel — and through nothing else. Under `dev`/`editor` (console on) the census reaches
the operator. Under `shipping` (binary file, console off) and `shipping-min` (text file, console
off) the rows are refused at the console gate and reach **no destination at all**: the uploaded
log a released title produces does not say whether it lost anything, which is the one question a
reader of that log asks first.

**Two dispositions, neither taken without you:**

1. **Route the census through the ring** (ordinary records, `Diag` target) so it lands in
   whatever sinks the preset opened. Cost: the census becomes subject to the same admission
   control it reports on — a storm that drops records could drop the census rows that say so.
   The current synchronous channel exists precisely to be outside that machinery.
2. **Render the census into the file sinks directly under the drain token at `close_out`**,
   beside the console write. Cost: a second delivery path for one report, and the binary sink
   would carry text lines outside the record format (or needs a frame kind for them).

**RESOLVED same day, by the owner's standing directive to decide without asking (2026-08-20 chat).
Disposition 1 was taken, narrowed to shutdown-time.** Disposition 2 fell to a fact discovered on
inspection: the `.blog` is a framed format, so "write the rows into the sink directly" means
constructing record frames anyway — which IS disposition 1 with extra steps. `census::print` now
emits every row as an ordinary ring record (`LOG-CENSUS …` / `LOG-ONCE …` under the `log` target),
`shutdown` orders itself "emit → deliver → close" in both arms, and the sinks — the binary one
included, which previously had no shutdown-time close at all — close only after the final pass.
The stated cost stands and is accepted: the census obeys the admission control it reports on,
which at shutdown means a quiet ring and an immediate delivery pass. Gate:
`log_host_shipping_min.rs` asserts `LOG-CENSUS` rows in the preset's own file after `shutdown`.

---

## 2026-08-20: SV0 returns as the dedicated pass — decision taken, not asked

Diagnostics closed; rendering resumed per the standing directive. The first decision was SV0's
disposition, OPEN since Rev 9: **taken — `sdf_mesh_shadow.comp`, RENDER-PARITY-PLAN §3.2's
critic-agreed Option B.** The numbers that decided it: the inline carried a MEASURED ~+75%
dark-path tax on every VB frame (reverted at `13f1c9a3`), the armed ratio 2.34× was the VB tail
being a worse HOST for the march (not the march's own cost), and the "dedicated pass rejected by
measurement" premise was retracted as false — the pass never existed to be measured. Ladder and
gates: `docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 10 (DP1–DP5), including the two gates the inline never
had — a per-producer byte budget and a dark-dispatch ABBA A/B with a pre-registered grid-step
budget.

---

## 2026-08-20: two Aether A7 candidates, recorded here so the book's "recorded" claim is true

Both were found by review during A6 and are documented at their parse sites, but no planning
document listed them until this entry. Neither blocks anything; both are DX defects of the shipped
surface.

1. **`at BARE_PATH { … }` swallows the node body as a struct literal.** `camera at MY_POSE
   { aspect: 1.5 }` parses `MY_POSE { aspect: 1.5 }` as one expression, and the diagnostic that
   results — ``the `camera` node needs an `aspect:` key`` — contradicts what the user wrote. The
   workaround (parenthesize the pose) is documented at `parse_at` and at the `sdf` arm, but the
   error message carries no hint. The fix is a hint on the required-key diagnostic when the `at`
   expression was a brace-suffixed path. Same hazard exists for `sdf EXPR`.
2. **`clippy::too_many_arguments` on a generated system fn lands on the whole `aether!` block.**
   A `system` with 8+ demand-driven params emits a fn clippy flags, and the span is the macro
   token — the user cannot act on it. An `#[allow]` inside `system_fn` would re-bless every A2
   token pin, so the fix belongs to a deliberate A7 pass, not a drive-by.

Also recorded: `engine_packages_census` does not classify the three `aether*` crates — it reds on
every full-workspace run for a reason unrelated to whatever is being tested. The census's
classification table needs three rows (they are engine crates: the language front-end, the shim,
and the integration-test crate).

> **RESOLVED — verified 2026-08-29. The three rows exist, and the disposition is the OPPOSITE of the
> one this paragraph recommended.** `tests/engine_packages_census.rs` now carries `"aether-lang"`,
> `"aether"` and `"aether-tests"` in `USER_PACKAGES` — the list documented as *"workspace members
> that are deliberately NOT engine packages"* — each with its own rationale comment: the transpiler
> half and its two-line proc-macro shim execute only inside rustc's process, so no runtime zone can
> ever exist in them and `Engine` would claim a runtime membership that is false by construction;
> `aether-tests` boots real `App`s the way a game does, on the `bench-bevy-vs-boyko` precedent. So
> the classification hole is closed at source, but "they are engine crates" above is **not** what
> landed, and the sentence is left standing rather than rewritten so the record shows the call that
> was actually made. (What is verified here is the three rows and their reasons, read at the file;
> the census was not re-run for this note, so no claim is made about the run's colour.)

---

## 2026-08-20: DP6a is BLOCKED on arithmetic, not on questions — recorded for the resume reader

DP6-0b's re-taken cells landed §R4.3.7 on branch 2 (MIXED): `E_split_host = 11 264 ns` of the
split tail's hosting surcharge is real (the other 87.8% of DP6-0's finding was instrument skew,
now repaired — the split shade fell 112 640 → 35 328 ns on the restamped instrument). Per the
design's own pre-registered rule, DP6a does not land until Decision 3's fused row is re-derived
with that term. The re-derivation is in flight; this entry exists so a resume reader does not
start DP6a from the ladder order alone.

**RESOLVED, same day.** The re-derivation landed as Decision 3's *"Trade-off, RE-DERIVED at
DP6-0b"* sub-block (design Rev 4.4): `Δ NET_fused ∈ [−5 404, +14 848] ns`, point estimate
`+1 034`, against a `+5 120` bar. §R4.3.7's block is discharged and **DP6a has landed** (design
Rev 4.5 carries its review dispositions). The headline of that revision is a WITHDRAWAL — §Goal's
"fused boots: cost-neutral **by construction**" was a construction claim refuted by measurement,
and neutrality is now claimed at DP6d or not at all.

---

## 2026-08-20: `vb_occ_dense` cites a release-live assert that does not exist — and one citation is inside a panic message

Found while enumerating the fixtures VB-SV0 DP6a turns; **pre-existing, unrelated to DP6a, and
left unfixed deliberately** (out of that rung's scope — filed rather than drive-by repaired).

`crates/boyko_app/tests/vb_occ_dense/mod.rs` twice attributes a guard to
`boyko_app/src/runner.rs:1101-1106`:

* its doc: *"`boyko_app::runner`'s bench arming carries a release-live `assert!(!mesh_geo_shade_split, …)`
  whose message is about **VB-P1d's published break-even**"*;
* the body of `assert_no_split_producer`'s own **panic message**, which tells a tripped author that
  arming a pre-light consumer *"makes `boyko_app::runner`'s bench arming panic at runner.rs:1101-1106
  with a message about VB-P1d's break-even"*.

**No such assertion exists anywhere in `runner.rs`.** A repo-wide search for a release-live
`assert!(!… mesh_geo_shade_split …)` returns nothing outside `boyko_render`'s own unit tests, and
`runner.rs:1100` is the DP6-0b `vb_zone_chain` / `vb_zone_derived` selector — an `if
… mesh_geo_shade_split { VB_CHAIN_SPLIT } else { VB_CHAIN_FUSED }`, which explains nothing about
break-evens and panics on nothing.

Why it is worth an entry rather than a silent fix: **the second citation is inside a panic
message**, i.e. it is read by exactly the person who has just tripped the fixture and is looking
for the cause. Sending them to a line that is a chain selector costs them the one lead they were
given. The class is the campaign's recorded one — a datum nobody re-derives, which the first
"fix" re-blesses. Repair should either name the real guard (if the intent survives somewhere) or
delete both citations and state the fixture's own reason without borrowing another rung's.

---

## 2026-08-20: DP6b's `-P` gate cannot be "character-identical" as the design spells it — and the measured reason

VB-SV0 DP6-DESIGN §Metrics (P1-5) specifies the new two-sided preprocessor gate as: *"`dxc -P
vb_geo.comp.hlsl` (no defines) is **character-identical** to the pre-DP6b file's `-P`"*. **As
written that is unachievable for any additive edit, and the shipped gate
(`crates/boyko_rhi_vulkan/tests/vb_geo_preprocess_sync.rs`) states a normalized form instead.**

Two independent reasons, both measured on the pinned VulkanSDK 1.4.350.0 `dxc`:

1. **`#line` directives carry the path and the line number.** The pre-DP6b side has to be
   materialised somewhere (`git show` writes it to a temp file), so the paths differ; and DP6b's
   edit is additive, so every line after the first insertion is renumbered. A gate comparing the
   raw text would red on a pure comment addition.
2. **`dxc -P` does not preserve blank lines across an elided region.** With the `#ifdef
   VB_SV0_TERM` block present, the single blank line between `} pc;` and `[numthreads(64, 1, 1)]`
   is swallowed by the `#line` jump that replaces the guarded span. **With `#line` stripped the two
   sides differ by exactly one empty line** — on a source every added line of which is inside a
   guard. No source formatting removes it: the jump is emitted whenever the elided run is long.

**Shipped form:** strip `#line` directives and empty lines, compare the remainder
character-for-character. Measured: **571 identical lines** pre-DP6b vs HEAD with the flag OFF;
**916 lines** with `-D VB_SV0_TERM=1`. The gate ships with its own two-directional sensitivity
control — an edit OUTSIDE every guard must move the base text, an edit INSIDE the guard must not
(and must move the `-D` text) — so the normalization is proved not to have removed the teeth
rather than argued to have kept them.

**What the normalization gives up, stated:** a mutation that changes ONLY whitespace outside a
guard. That cannot change a compile, and the `.spv` byte gate
(`vb_raster_geo_classify_spv_sync.rs`) covers the compile independently.

**Not filed as a defect in the design.** The spelling was written before anyone ran `dxc -P` on
this file — the same shape as the design's own P1-5 finding that no `.rs`/`.ps1` in-tree invoked
`-P` at all, only plan prose. It is recorded here so the next reader of that sentence does not try
to "fix" the test back to a literal comparison it can never pass.

## 2026-08-20: DP6b landed its SHADER half only — the `vb_geo_aux_layout` widening is atomic with a file that was checked out dirty

DP6b's ladder entry has two halves. The shader/variant/gate half is landed (guarded span,
`vb_geo_sv0.comp.spv`, `embed_spirv!` + accessor, `spv_sync` row, the new `-P` gate,
`sdf_field_edsl_sync` re-pointed, manifest row). **Decision 5's `vb_geo_aux_layout` 3 -> 5 widening
and its two boot descriptor writes are NOT landed**, because
`crates/boyko_rhi_vulkan/src/present/targets.rs` carried unrelated uncommitted work from a
concurrent lane at the time.

**They cannot be split, and the reason is mechanical rather than stylistic** — recorded so nobody
lands "just the layout" as a smaller step:

* `rhi_impl/device.rs::create_bind_group` sizes the descriptor POOL from the `entries` histogram
  and then allocates a set from the LAYOUT. A 5-binding layout fed 3 entries allocates against a
  pool missing one `STORAGE_IMAGE` and one `STORAGE_BUFFER` -> `VK_ERROR_OUT_OF_POOL_MEMORY` on
  every split boot.
* The same function carries `debug_assert!(count == desc.layout.entry_count)`, so a debug build
  panics before it gets there.

So the widening (`boyko_app/src/gpu_scene/mod.rs`), the `!rg8_ok` placeholder bind
(`targets.rs::vb_geo_aux_set`) and the `vb_geo_sv0` boot pipeline land together or not at all. The
DP6b gate is otherwise met: all `*_edsl_sync`/`*_spv_sync` green, the five named goldens
byte-identical, `gpu_command_census` and `vb_bench_query_validation` green.

## 2026-08-20: the SV0 tuning-block VALUE pin covered two of four shipped hosts — CLOSED at DP6b, with one adjacent claim left open

**Found by the DP6b review, and the finding is partly about the report that preceded it:** the DP6b
implementation report said this gap was "filed". It was not — it was named in a report and filed
nowhere, while the twin defect of the same class (`vb_occ_dense`'s citation of a non-existent
release-live assert, the entry above) was filed properly the same day. *One twin filed, one
claimed.* Recorded because the failure mode is the campaign's own: a datum that exists only in
prose nobody re-derives.

**The defect.** `sdf_shadow_leaf_oracle.rs`'s `SHADOW_CONST_SOURCES` read
`["deferred_pbr.hlsl", "sdf_gbuffer_composite.hlsl"]` above a doc calling them *"the two sources
that carry the SHADOW march tuning block"* — while **six** shipped shaders declared `SHADOW_K`. The
two SV0 marcher hosts, `sdf_mesh_shadow.comp.hlsl` (DP1) and `vb_geo.comp.hlsl` (DP6b), were never
added. An edit to `SHADOW_K` in either one alone stayed green in that oracle, green in
`sdf_field_edsl_sync.rs` (which pins the consts' PRESENCE and their ORDER against the include, never
their VALUES), and green in every golden — the only red would have been the `.spv` byte gate, whose
documented repair for a red is *"re-run the header recipe and commit the result"*, i.e. the exact
motion that blesses the divergence.

Two independent citations pointed at a test that does not exist — `sv0_consts_match_deferred_and_marcher`
— in `sdf_field_edsl_sync.rs`'s panic message and in `docs/VB-SV0-SDF-SHADOW-PLAN.md`. The real
test had been RENAMED to `sdf_shadow_and_ao_consts_match_deferred_and_marcher`, and the rename is
what carried the reader past the coverage hole: looking up the cited name returns nothing, so
nobody reached the list to notice what was missing from it.

**CLOSED, not filed.** `SHADOW_CONST_SOURCES` is now all four shipped hosts; the selection-size
assertion derives from `SHADOW_CONST_SOURCES.len()` instead of a hand-written `21`, so adding or
removing a source can no longer silently shrink the gate; the doc states why the two SV0 hosts were
missing; the `sdf_field_edsl_sync.rs` panic message cites the real test name. **All four hosts fold
identically on the first run** — the blocks were verbatim mirrors, so no divergence had accumulated
in the window; the gate is red-capable, demonstrated by perturbing `SHADOW_K` in `vb_geo.comp.hlsl`
alone (`folds to 9 (0x41100000), but the host mirror is 8 (0x41000000)`).

**Still open, filed rather than fixed (pre-existing, unrelated to DP6b).**
`vb_geom_fetch.hlsli:581-583` claims of `vb_sv0_face_normal`'s cost argument that *"that is verified
on the artifact, not assumed: the committed `.spv` are disassembled and the `Cross`/`InverseSqrt`
chain must appear INSIDE the gate's conditional region, never hoisted into the entry block by
`-O3`"*. **No test in the workspace disassembles any `.spv` for a `Cross`/`InverseSqrt` hoist
check.** The claim's own words are what make it worth an entry — "verified on the artifact, not
assumed" is precisely the sentence a reader trusts instead of re-checking. Repair is either the
census the sentence describes (a `spirv-dis` block-membership assertion, the
`vb_raster_geo_classify_spv_sync.rs` builtin-census idiom) or striking the claim; DP6b widened the
`VB_SV0` definer set to two, so the sentence now also covers a module nobody has looked at.

## RESOLVED 2026-08-20: particles P1's Lipschitz skip — the ARCHITECT ruled the PLAN was the stale artifact, not the code

**Disposition (architect, 2026-08-20): the shipped form stands; `docs/PARTICLES-PLAN.md` §D9 is
amended and carries an ERRATUM.** The original line applied the reported→euclidean transform to an
operand that was already euclidean; the corrected block, the tunneling class it authorized for any
`k > 0` smooth edit, and the `radius·(L−1)` conservative band the shipped form pays instead are all
recorded there. See **§D9 + ERRATUM**. The entry below is kept verbatim as the escalation that
produced the ruling.

**What remains open is only the second half:** no `k > 0` fixture exists, so the two forms are
still discriminated by DERIVATION and not by measurement. Building one is the follow-up, and it is
a scene, not a code change.

**Not a question about what to do — the conservative form shipped. Recorded because the plan's own
text now disagrees with the code on one line, and the reader who checks D9 against
`particle_sim.comp.hlsl` deserves to find the reason here rather than derive it.**

`docs/PARTICLES-PLAN.md` D9 writes rung P1's per-substep skip as

```
if (cached_d - speed*timestep/FIELD_LIPSCHITZ_L > radius) { cached_d -= speed*timestep/L; skip }
```

— the Lipschitz constant DIVIDING the travel. The shipped block multiplies by it instead
(`travel_l = length(vel) * pc.timestep * FIELD_LIPSCHITZ_L`, compared against `radius * L`).

**The defect is a UNIT MISMATCH, and naming it is the point of this entry.** The three quantities
in that line do not live in the same space. `cached_d` is a value the FIELD reported; `radius` and
`speed*timestep` are EUCLIDEAN world lengths. `sdf_field.hlsli` states the conversion between them
in its own words — *"`d / L` is a conservative lower bound on the Euclidean clearance"* — so the
comparison is only meaningful once one side is converted. D9 converts neither: it divides the
euclidean travel by `L` (a quantity that is already euclidean) and compares it against a reported
value left unconverted.

Done properly, in euclidean units throughout: the true clearance `c` satisfies `c >= cached_d / L`,
a move of `s` leaves `c' >= cached_d/L - s`, and the substep is safe to skip exactly when
`cached_d/L - s > radius`. Multiplying through by `L` — which is what the shipped code does, so
that every per-substep operation stays a multiply — gives `cached_d - L*s > L*radius`. **`L`
multiplies the travel and the radius; it divides nothing.**

**What the shipped form costs, stated so it is not mistaken for the hazard.** Against a field with
`L == 1` the shipped test re-evaluates earlier by `(L-1)*(s + radius)` of reported distance. The
travel term `(L-1)*s` is the *necessary* correction for a super-Lipschitz field. The remaining
`radius*(L-1)` is a **conservative band**: the shell is a euclidean length carried through the same
`1/L` clearance bound as the distance, so it is over-stated in reported units. Its price is extra
field evaluations near a surface — never a missed contact. D9's form errs in the opposite
direction, and that direction is unbounded: `d > radius + s/L` passes wherever the safe
`d > L*(radius + s)` does and in a band above it, so it skips substeps in which contact happened.
Every skipped substep is a substep in which the field is not evaluated at all, so this is a
TUNNELING class — the one failure this rung's gate exists to bound.

**Scope of the disagreement, stated so nobody re-opens it as a bug.** The two forms are IDENTICAL
at `L == 1`, which is every hard-CSG (`smoothness == 0`) scene, i.e. every fixture in the tree today
including P1's own live fire. They diverge only where a smooth edit makes the field
super-Lipschitz — precisely the regime the constant exists for.

**What was left open, and how it closed:** whether D9's line should be corrected in the plan — an
architect edit, since the plan is APPROVED Rev 4 and an implementer amending its normative
pseudocode is not the same thing as recording a deviation. **Ruled 2026-08-20: corrected, with an
ERRATUM.** The second half — whether a fixture with a `k > 0` edit should exist to exercise the
`L > 1` regime — stays open: today no shipped scene has one, so the divergence remains unmeasured in
both directions and the shipped form's soundness rests on the derivation.

---

## The owner channel exists twice, and the copies have diverged (2026-08-21)

**Measured, not suspected.** `git merge-tree --write-tree feat/multi-paradigm-render
claude/trusting-ramanujan-0f8927` reports five conflicts, and one of them is an **add/add on this
very file**. Both branches created `docs/OPEN-QUESTIONS.md` independently as "the standing owner
channel per CLAUDE.md", neither knowing the other had:

* `claude/trusting-ramanujan-0f8927` seeded its copy at `867dd734` with the worktree-clippy
  decision and the two-dispatcher-lanes observation;
* this branch grew the copy you are reading now, to 2853 lines.

Neither is a subset of the other. **A reader cannot tell which is current, and finds out only by
acting on the stale one** — the same failure mode the `docs/ru/` rule exists to prevent, where a
diverged pair is worse than a missing one.

**Why this is filed rather than fixed.** The merge that would reconcile them is not mechanical.
Its other four conflicts are `docs/FEATURE_MAP.md`, `docs/SYSTEMS.md`, and — load-bearing —
`crates/boyko_ecs/src/ecs/core/ecs_master/event_api.rs` and
`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`. Both sides edited the same executor: the
event-lane branch moved completion publication under unwind protection, and this branch moved
elsewhere in the same functions. Resolving that requires understanding both changes, not choosing
a side, and it touches the ECS kernel's panic path — the part of the scheduler whose failure mode
was a **silent hang**, which is the worst-shaped bug this kernel has produced.

**What the owner needs to decide:** whether the event-lane fix reaches this branch by merging
`claude/trusting-ramanujan-0f8927` directly, or by landing it on `master` first and merging that.
The kernel conflict is the same either way; the difference is which history carries it.

**Not started because** the main checkout currently holds ~484 lines of live uncommitted work in
`crates/boyko_rhi_vulkan/src/present/targets.rs` and `device.rs`. A kernel merge wants a clean
tree and its own worktree.

---

## No `.gitattributes`, and the worktree rule just armed what that costs (2026-08-21)

**Measured while fixing `the_thresholds_file_is_the_one_r0a_froze`.** Git for Windows ships
`core.autocrlf = true` in its system gitconfig, so every checkout made under it writes text files
with CRLF, and every checkout made before it kept LF. This repository has **no `.gitattributes` at
all**, so on-disk bytes are a property of *when and where the checkout was made*.

The main checkout has LF. All three worktrees (`D:/wt/gates`, `D:/wt/reflect`, `D:/wt/ui`) have
CRLF. Any gate that hashes a file's **raw** bytes therefore reports where it ran.

**Why this surfaced now and not a year ago.** The one-worktree-per-system rule created the first
new checkouts this repository has seen in a while. The defect it exposed was latent from birth —
`assert_thresholds_frozen` has never normalised, across every commit since the module was created
at `21edc80f` — and was invisible for exactly as long as only the LF checkout ran it. Adopting the
rule did not cause the bug; it armed it, and it will keep arming this class.

**Fixed at the one site that could red.** `assert_thresholds_frozen` now hashes LF-normalised bytes,
matching the definition its two sibling sites (`vg_thresholds_freeze.rs`, `vg_r0_reference_rig.rs`)
already used and documented. The pinned literal is unchanged.

**One residual, which cannot red and is therefore worse in a quiet way.**
`crates/boyko_app/tests/vg_r0d_census.rs:332` hashes `assets/vg_corpus/CORPUS.toml` over raw bytes
and *writes the digest into* `docs/VG-R0-DENSITY-CENSUS.md`. Nothing compares it against a pin, so
no gate will ever complain — but the number recorded in that document depends on which checkout
produced it. A future reader comparing two runs would be comparing checkout configurations. It sits
behind a GPU `#[ignore]`, so it is not urgent.

**The decision that is yours, not mine.** A `.gitattributes` — `* -text`, or a narrower rule for the
frozen/hashed files — would make on-disk bytes deterministic everywhere and retire this whole class.
The cost is that it rewrites line endings across working trees on the next checkout, and two lanes
(`feat/reflection`, `feat/ui-advanced`) are mid-flight in worktrees right now. That is a
disruption I should not schedule for you. The thresholds gate is immune either way.
