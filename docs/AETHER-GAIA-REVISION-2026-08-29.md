# Aether v2 + Gaia — revision of 2026-08-29, and the work order that follows

The entry point for both campaigns after their first adversarial revision. It records **what the
revision changed, what it refuted, and in what order the remaining work can proceed**. The campaigns
themselves live in [`aether-v2/`](aether-v2/CAMPAIGN.md) and [`gaia/`](gaia/CAMPAIGN.md); this file
does not duplicate them — it says what is now true across both, and what is owed.

> **Written under the instruction that nothing is committed without explicit approval** (owner,
> 2026-08-28), then *update all the plans and write them down* (owner, 2026-08-29).
> ⚠ **CORRECTED 2026-09-03: this paragraph used to end "the plans are written into the working
> tree; `HEAD` is unchanged at `a4591c56`", and that has been false since the day after it was
> written.** `97c504c8` committed exactly these files — including this one, as an `A` — and is the
> head of `feat/multi-paradigm-render`. A record that misstates its own commit state is the same
> defect class this revision exists to remove, one layer up.

## What was done

Three adversarial passes, each with per-finding refutation before anything was believed:

| Pass | Target | Survived refutation |
|---|---|---|
| Conformance audit | worked example code against the ratified rulings | 35 findings, merged to 21 |
| Corpus revision | the 13 plan files, five lenses (consistency · engine claims · decision debt · gates-that-cannot-fail · integration) | 57 findings |
| Write-back verification | the edited corpus, three lenses (fidelity · cross-references · language and no-new-lies) | 12 findings, then a second repair round |

The adjudicated output was a per-file edit list plus a consolidated ballot list. Both are now
dissolved into the corpus: the edits into the files below, the ballots into
[`OPEN-QUESTIONS.md`](OPEN-QUESTIONS.md) §2026-08-29 and its Russian mirror.

## What the engine refuted

Every claim below was asserted by the plans (some by this session's own analysis) and is false at
source. Each is corrected in place; each is listed here because a plan that has been wrong once
about the engine will be read differently.

| The plans said | The source says |
|---|---|
| `Transform` derives no `Default`, so a partial record cannot be completed | `boyko_scene::Transform` **has** `impl Default` → `IDENTITY`. The hole is real but at **field** granularity: a closed build-time evaluator cannot name an individual omitted field's neutral, and no per-field default table exists |
| `PointLight` carries `intensity` | It carries **`power`** (luminous flux Φ, lumens). Also `range` with no default, `position` that `light_reconcile` **derives** from `GlobalTransform` (so it must be undeclarable in a scene), and `color: [f32; 3]` documented LINEAR against a four-component `#RRGGBBAA` literal |
| `without F` over a `flag` never matches | Inverted. A `flag` is `StorageKind::Bitset`, filtered out of every archetype signature, yet both filters still take the archetypal path over it — so **`with F` matches nothing** and **`without F` excludes nothing**. Two different silent wrong answers from one storage kind. `CONSTRUCTS.md` carried the defect in its own example |
| `MAX_EVENT_THREADS` is 65 | It was **64** when this row was written; 65 was `KE8`'s unlanded plan value, corrected at five sites. ⚠ **Superseded 2026-08-30**: KE8's lane half landed with R1 at `01a4436e`, so the tree now reads **65** (`crates/boyko_ecs/src/ecs/constants.rs:400`), and ruling **E5** raises it to **66**. The row is kept because the *lesson* stands — a plan value was being cited as an engine fact — but the number in it is now history, not the tree |
| Event registration fails silently on both ends (ruling C3) | Both generated ends **panic loudly at init**. Only the direct `EcsMaster::events_of` path is silent, and no generated system uses it — so the auto-registration grant stands on no recorded ground (ballot **AB-1**) |
| `Query` gains `contains`/`first` by porting `QueryView`'s tests | `QueryView` has neither. They are new API needing new tests, including a **stated** order for `first` |
| "~117 production `Or<(` sites" | 112 textual matches: **4** production type positions, 6 doc comments, 48 kernel-internal, 54 test/bench/fixture. `R0`'s priority rests on the silent-wrong-answer mechanism, not blast radius |

## Gates that could not fail

The repo's standing lesson — *a gate that cannot fail is not a gate* — was applied to the plans
themselves. Struck and re-axed onto the property each claimed to test:

- **`bake(1) == bake(W)`** (Gaia G4) — `W` has no referent anywhere in the corpus and nothing makes
  the bake parallel. Replaced by file-order independence: `bake(files) == bake(shuffle(files))`.
- **`build(1) == build(W)`** at four sites — compares two runs of the same code and cannot see a
  scatter defect. Demoted to smoke; the real gate is a serial-emission reference stream.
- **Formatter idempotence** — `fmt(fmt(x)) == fmt(x)` passes on a formatter that does nothing.
  Replaced by convergence over perturbed input (group-order permutations, line-break and
  trailing-comma variants).
- **The still-frame wall clock** (Gaia G7) — replaced by a count oracle: bind-sink writes executed
  must be zero, tick slots read pinned to the fixture's expected value.
- **"All with committed red-first repros"** (AI-ORIENTATION) — the two probe crates exist in a
  session transcript and **nowhere in the repo or its history**, while line 5 of the same file said
  so honestly. Committing them under `crates/aether_tests/tests/ui/` is now an explicit R3
  deliverable, and no oracle may name them as artifacts until it lands.

## 🔑 The class that recurred three times

**Every one of the three repair rounds introduced a fresh false claim while fixing false claims** —
a fabricated `tls.rs` path, `MAX_EVENT_THREADS = 65` written as the engine's value, an inverted
account of the flag-filter mechanism, and — most instructive — a pass repairing a retracted claim at
four sites **created a fifth**. Doc-rot repair is as error-prone as doc-rot authorship. The working
rule this establishes: **after repairing a false claim, run a separate pass asking whether the
replacement is true**, or the repair propagates the lie under cover of a fix.

## One mechanism, three shipped defects

**The sharpest evidence that these are one campaign and not two.** Three defects in **shipped**
code — two of them found by the *data* campaign, in kernel and UI code the *logic* campaign owns —
are the same defect on the same axis: **a mechanism that resolves a per-archetype pool without
screening the storage kind, and so is blind to the kinds that own no pool** (`Dense`, `Bitset`).
Every one compiles, runs, and answers wrongly in silence.

| Site | Symptom | Raised by |
|---|---|---|
| `Or<(…)>` with a dense arm | the arm is never true; the query returns a plausible wrong set | Aether **KE1** / rung **R0** ([`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md)) |
| `any_changed_since` over a dense or bitset bind source | the change gate is never true, so a `boyko_ui` binding over such a component **never updates**, and nothing is logged | Gaia **GK-2** ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §UI bindings, item 8) |
| the load path's `requires` closure | nothing on load adds a component the file omitted, so `query<(&Health, &Regen)>` **silently skips level-authored entities** while gameplay-spawned ones match | Gaia **F4(ii)** ([`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) §Owner ballots) |
| `with F` / `without F` over a `flag` | the *fourth* member, and the one that shows the class is not about Dense alone: `with` matches nothing, `without` excludes nothing — **two different** wrong answers from one storage kind | Aether **AB-11** ([`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md) §`system`) |

Two consequences the corpus has to carry, because neither is visible from inside one campaign:

- **A fix on one ladder falsifies a ground on the other.** R0 removed the ground under Gaia's
  `Or`-over-dense codegen ban — that was ballot **GB-9**, and it is why the ban's site carried a
  superseded-by note pointing at a rung it does not own. ✅ **RULED 2026-08-30: the ban is DELETED
  with a record**, and its hazard re-aimed onto the still-live GK-2 row (a bind source or
  change-gate over a dense/bitset component). ⚠ **The coupling worked exactly as designed and the
  ruling is its receipt** — a kernel-side fix on one ladder did prompt a revisit of a data-side
  ruling that would otherwise never have been reopened. ⚠ **What it did NOT do is meet its own
  deadline**: "decide before R0 lands" expired unanswered, so the record had to be reconstructed
  from prose rather than from a reproducible failure. A cross-ladder note is not a substitute for a
  rung — the next such coupling needs an owner and a rung, not only a deadline.
- **The class needs an enumeration, not four anecdotes — and it now has one.** Its single home is
  [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) **§Census — "resolves a per-archetype
  pool by `ComponentId` without screening the storage kind"**, landed with R0 and carrying its own
  producing command. **Both ladders cite that section; neither re-derives it** — re-deriving per
  campaign is how the same defect gets found a fifth time. Its content is deliberately not repeated
  here.

## Aether and Gaia are one body of work

*Aether is the language for **logic**, Gaia the language for **data*** — scenes, UI documents,
assets (owner, 2026-08-28). The split is by **subject matter, not by project**: the two share a
kernel, a tooling axis, a diagnostic envelope and an id namespace, and each has already found
defects that belong to the other. The couplings that exist **today**, each checkable:

| Coupling | Where |
|---|---|
| **Shared kernel enablers.** Gaia cites the Aether campaign's backlog ids directly; `KE1`'s fix is the disposition question in Gaia's own ballot GB-9, and Gaia's `GK-2` names the same pool-resolution mechanism | [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) is cited from [`gaia/DECISIONS.md`](gaia/DECISIONS.md); the backlog's own header enumerates `docs/gaia/` as a citing corpus |
| **One AI-orientation requirement set, housed on the Aether side but partly owned by Gaia.** `AIR-08` (stable node ids) and `AIR-16` (grammar rulings) carry **"Gaia spec"** in their own Rung column; `AIR-17` carries **"Gaia G0"**; `AIR-09` carries "R8 / Gaia tooling" | [`aether-v2/AI-ORIENTATION.md`](aether-v2/AI-ORIENTATION.md) |
| **One diagnostic envelope and one code-registry discipline.** `AE####` (Aether) and `GA####` (Gaia) are the *same* registry rule, minted by `AIR-02`; Gaia's diagnostics ride the `AIR-01` envelope from the baker's first commit | `AI-ORIENTATION.md` AIR-01/02; [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) §Relations |
| **Cross-campaign ballots.** Two ballots in the **Gaia** `GB` series named rungs on the **Aether** ladder: **GB-9** (decide before R0) and **GB-8** (named **R8** as a candidate owner of the corpus-wide link/id census). ✅ **Both RULED 2026-08-30** — and the ruling **halves one of the two couplings**: GB-8's *id* census lands at **Gaia G0**, not R8, so only its *link* census remains cross-ladder | [`OPEN-QUESTIONS.md`](OPEN-QUESTIONS.md) §2026-08-29; [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) §Open ballots |
| **One id namespace, enforced across both directories.** The `E#`/`M#` → `KE#`/`KM#` rename was scoped wrongly *because* it treated `docs/gaia/` as someone else's corpus, and had to be re-run over all of `docs/` | `KERNEL-BACKLOG.md`'s own header records the miss |
| **A shared surface boundary.** Aether's `scene` narrows to dev-bootstrap and the shipped world form moves to Gaia; Gaia's baker **prints the existing `boyko_serialize` format** rather than minting a second one | [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md) §`material`, `scene`; [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline |

**Working rule.** A reader who arrives at either campaign's `CAMPAIGN.md` must be able to reach the
other from it, and a change that touches a coupling above updates **both** sides in the same commit
— a diverged pair is worse than a missing one, because the reader cannot tell which side is current
and finds out only by acting on the stale one.

## Per-file: what changed

| File | Substance |
|---|---|
| [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) | R8's phantom-artifact oracle; R0's measured `Or<(` breakdown; R1 gains `KE10`/`KE11` and `single_mut`; R4 and R6 gates re-axed; a new **§Open ballots on this ladder** |
| [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) | C3 rewritten against the measurement (→ AB-1, AB-2); D5's five pin tests enumerated so citations resolve; dangling `F#` citations substituted; a new **AIR-10 familiarity / false-friend audit** section; the D3 numbering gap recorded |
| [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md) | `without Stunned` → `disabled Stunned` with the corrected polarity and its mechanism; event bounds symbolic; `requires` over dense storage recorded as a known-open hole (→ AB-6); the `each par` driver honesty edit (→ AB-8) |
| [`aether-v2/MACHINES.md`](aether-v2/MACHINES.md) | scenario 2 declares the `regen_mana` it orders against; the `MIN_ARCHETYPE_FOR_PARALLEL` floor stated; router mechanism held open (→ AB-5, **RULED 2026-08-30: `Query::get_mut`**); R-DENSE re-grounded (→ AB-7, **still the owner's; its candidate ground measured and REFUTED**) |
| [`aether-v2/EVENTS.md`](aether-v2/EVENTS.md) | the false release-mode lane claim replaced by the measured behaviour with a real symbol; `ordered` gate aligned to R4; sender-exclusivity registrant held open (→ AB-4) |
| [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) | series renamed `KE#`/`KM#`; `KE11` widened to **both** poolless storage kinds with both real call sites; `KE3` provenance corrected; `KE9`'s full signature pinned; `boyko_reflect` recorded as external and unmerged |
| [`aether-v2/SPATIAL.md`](aether-v2/SPATIAL.md) · [`OPEN.md`](aether-v2/OPEN.md) | Phase 3 gate re-axed; forced-collision pin test given a shape that can fail; the unverified ledger gains the `Or<(` command and the probe provenance |
| [`aether-v2/AI-ORIENTATION.md`](aether-v2/AI-ORIENTATION.md) | the committed-repros claim struck; defect series renumbered `AD1..AD4` (the old `G#` collided head-on with Gaia's rungs); AIR-03/06/07/10/11 oracles given failing conditions |
| [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) | G4's vacuous gate replaced; G1/G2 gates made text-free; G7's count oracle; ballots F8/F9/F10 added; F4 widened |
| [`gaia/DECISIONS.md`](gaia/DECISIONS.md) | the reference taxonomy's missing third kind recorded (→ GB-3); a third forbidden binding-source row (dense/bitset sources are invisible to the change gate); per-axis bake-budget fixtures |
| [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md) | **ratified-stale**; interim drift-reduction annotations only — `power`, `range`, the colour arity, the `position` ballot. The rewrite into the reworked syntax is owner-gated |
| [`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md) | REV 2: rulings R1–R7 as amended, fix Tiers 0–4, the K1–K16 record, and the id-namespace notes. ⚠ **CORRECTED 2026-09-03: this cell said "new, untracked".** `git log --name-status 97c504c8` shows the file as an `A` — it is tracked on both branches. What it lacks is the owner's APPROVAL, not a commit |
| [`OPEN-QUESTIONS.md`](OPEN-QUESTIONS.md) + [`ru/`](ru/OPEN-QUESTIONS.md) | the 32 ballot bodies; five that were cited but never written; F4 rewritten to the widened question; a corrupted Russian entry restored |

## Work order

**Buildable now, no ballot in the way:** `R0` (the `Or` dense-blindness fix), `R1` (kernel enablers
— `KE11`'s *disposition* is on ballot AB-6, but its red tests land regardless), `R2`
(`state_chart!` and the route merge).

**Blocked, with the blocker named.** ⚠ **This table had FIVE filing errors, repaired 2026-09-03
and left visible with strikethrough rather than silently rewritten** — two rungs whose blocker was
omitted (G6 ← F5, G7 ← F6), one ballot filed against no rung at all (F9), one filed against half
its own scope (GB-2, whose G5 half was dropped), and one filed against half its own scope in the
other direction (GB-8, whose G0 half turned out to carry the work). A sixth, GB-6 against G7 while
its body names G6, had been caught only on `feat/threadpool-ke16`. Twelve of the fourteen Gaia
ballots have since been answered; the strikethroughs record which:

| Rung | Waits on |
|---|---|
| Aether R3 | AB-1, ~~AB-2~~ (event construct) · AB-6, ~~AB-8~~, AB-11, AB-13 (the rest of the surface) · AB-10 on its keyword surface. ~~AB-8~~ ✅ **RULED 2026-08-30** → [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) **C5a**: `each par` → `par_iter_mut` (measured 2048/2048 tracked vs 0/2048 chunked; 1.17–1.47× ≈ 1–2 % of the pass); `soa par` exists and is the only route to the chunked driver; batching key not author-visible |
| Aether R4 | AB-3, AB-4 |
| Aether R5 | ~~AB-5~~, AB-7. ~~AB-5~~ ✅ **RULED 2026-08-30** → **M4a**: the router uses `Query::get_mut`; the decisive ground is that `get_component_mut` forces an **exclusive** system, which takes **no param tuple** and so cannot hold `EventReader<E>` — it would have to read via `events_of`, silent on an unregistered type and one frame stale. **AB-7 remains the owner's**, but its candidate driver-independent ground was **measured and REFUTED** (both deposit APIs carry working dense arms; no layout const-assert exists) |
| Aether R8 | ~~AB-9~~ ✅ **RULED 2026-08-30** (merge pulled forward as its own rung before R8; AIR-06(b) not descoped; engine-crate reflection opt-in attached to R8's Lands) · ~~GB-8~~ ✅ **RULED 2026-08-30** — and the ruling **splits it**: the id census widens to all of `docs/` at **G0**, not R8; only the **link** census lands at R8, with 59 measured dead targets as its red-first evidence. R8 is no longer blocked by either — **Filing repair kept from `feat/multi-paradigm-render` (2026-09-03):** AB-9 · ~~GB-8~~ — ⚠ **MIS-FILED, and the ruling splits it**: GB-8's ID census lands at **Gaia G0**, and only the LINK census rides R8. Filing it against R8 alone dropped the half that carries the work |
| Gaia G0 | — (GB-8's id-census half, per its ruling) |
| Gaia G1 | ~~F1~~ · ~~GB-5~~ — both RULED 2026-08-30; the field-table freeze is unblocked |
| Gaia G2–G3 | ~~GB-2~~ · ~~GB-3~~ — both RULED. ⚠ **MIS-FILED**: GB-2's own body says it blocks **G2/G5**, and this row dropped the G5 half. Still open on G2: **F9's residual**, and on G3: **F8** — **Ruling body kept from `feat/threadpool-ke16` by the merge, 2026-09-10:** ~~GB-2~~ ✅ **RULED 2026-08-30** — the colour transfer function is a property of the **destination field**, not of the literal (sRGB EOTF into a linear float colour, identity into a `u32` STRAIGHT-RGBA8 field, coded refusal into a device-encoded packed carrier); the arity half, which nothing carried, is settled on the same ruling by mirroring Aether's shipped `ColorLit`. ⚠ Carries a **fixture constraint** into G2: the two routes agree at bytes 0 and 255 only, so a white or black colour fixture cannot fail · GB-3 |
| Gaia G4 | ~~GB-1~~ · ~~GB-4~~ — both RULED — **Ruling body kept from `feat/threadpool-ke16` by the merge, 2026-09-10:** ~~GB-1~~ ✅ **RULED 2026-08-30** — a template expansion occupies **no ladder layer**: it is one step on the inheritance-**depth** axis, ranked where `extends` ranks, and resolution is depth-then-ladder; an unlabeled write is `base`. The ratified ladder is untouched, and finding K2 is dissolved rather than diagnosed · GB-4 |
| Gaia G5 | ~~F2~~ · ~~F3~~ — both ANSWERED. ⚠ Plus **GB-2**'s G5 half, which this table never filed |
| Gaia G6 | ~~F4 (widened)~~ · ⚠ **AND F5, which this row omitted** — F5 blocks G6 by its own body, and its ruling REWRITES G6's scope. Both ANSWERED 2026-08-30 |
| Gaia G7 | ~~GB-7~~ · ~~GB-9~~ — both RESOLVED. ⚠ **AND F6, which this row omitted** (F6 blocks G7 by its own body; RULED 2026-08-30). ⚠ **GB-6 was MIS-FILED here**: its own body names **G6**, not G7 — and it is now DISPOSED (2026-09-03), its (b) half moving to **Aether R3**. ⚠ **G7's POSITION changed 2026-09-03**: the owner ruled that editor v1 authors UI documents too, so G7 moves UP the ladder rather than sitting after G6, and the `.ui` deletion census is written before the windowed pass. Its blocker is unchanged — UI reaching the screen, which is a wiring gap, not a missing plugin — **Ruling body kept from `feat/threadpool-ke16` by the merge, 2026-09-10:** GB-6 · ~~GB-7~~ ✅ **RULED 2026-08-30** — **no wall-clock companion; the count gate stands alone.** Measured: the timed loop's inputs are archetype count, bound-**type** count and row count, and **not** the binding count (`dynamic_bound_ids` is a deduplicated type set), so the still frame is 332 ns at **+0% for 10× the bindings** but **+82% for 2× the rows**; at a 100 ns timer step the frame is ~3.3 ticks with a 15-25× single-call tail, and the red-first delta (0 → 1 sink write) sits at signal-to-noise 0.05-0.50. A clock over the **scan itself**, world pinned, belongs to GK-2 · ~~GB-9~~ ✅ **RULED 2026-08-30** — the `Or`-over-dense emission ban is **deleted** and replaced by a ban on a bind source or change-gate over a dense/bitset component (GK-2's still-live ground); the three `Or`-dense shapes R0 left uncovered, plus KE13, are filed against G7's codegen rules as the shapes with no oracle |
| Gaia G2 / G6 | ⚠ **F9, which appeared against NO rung in this table at all** though its own body, `gaia/CAMPAIGN.md` and PENDING Part D all say G2/G6. Partly ruled; its residual VALUES question is still the owner's and blocks **G2** |
| Aether R3 | ⚠ **GB-6(b)**, moved here 2026-09-03: an Aether construct that toggles a flag |

> **Merge note, 2026-09-10 (`merge/ke16-into-render`).** The two branches both edited this
> table. `feat/threadpool-ke16` carried the RULING BODIES written where the rulings were
> decided; `feat/multi-paradigm-render` carried the 2026-09-03 FILING REPAIRS and the later
> dispositions. Both are kept above, row by row. Three of ke16's rows stated only the
> pre-ruling open state and are superseded by the repaired rows above rather than repeated:
> `| Gaia G1 | F1, GB-5 |`, `| Gaia G5 | F2, F3 |` and `| Gaia G6 | F4 (widened) |`. They are
> recorded here verbatim so nothing this merge removed is unrecoverable from the file itself.

**Not a ballot — design debt inside R3**, routed to the architect rather than the owner. Neither
`bundle` nor `relation` may be declared done while these stand:

- **K8** — a bundle carrying a `link Entity` field has no honest spawn spelling. `Health` has no
  `Default` and no complete literal, so `Pawn` cannot be written at a scene prop position. The
  Aether twin of Gaia's bake refusal.
- **K9** — the kernel's relationship macro demands the reverse-index collection field be
  **private** and mandates `retain_empty` in v1. Neither is expressible on an Aether group whose
  fields are `pub`. This is exactly the desync that ratifying `relation` (O4) was meant to make
  unwritable.

## What is deliberately NOT in the repo

- **The worked example code** for both languages (a full `aether!` block and three Gaia profiles,
  rebuilt after the conformance audit) lives in the session scratchpad. It is an exemplar, not a
  spec, and `LANGUAGE.md`'s rewrite is owner-gated — publishing examples in the reworked syntax
  before that rewrite would create a second diverged pair.
- **The conformance-audit finding bodies** (`M#`/`C#`/`I#`/`G-##`). Only the summary at each use
  site in `PENDING-SYNTAX-PLAN.md` is carried. Promoting them, or renumbering to a `PS-` prefix, is
  bookkeeping deferred until the M6 rewrite lands.
