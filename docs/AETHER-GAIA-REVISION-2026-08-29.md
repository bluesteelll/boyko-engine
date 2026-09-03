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
| `MAX_EVENT_THREADS` is 65 | **64** in the tree; 65 is `KE8`'s unlanded plan value. Corrected at five sites |
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

## Per-file: what changed

| File | Substance |
|---|---|
| [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) | R8's phantom-artifact oracle; R0's measured `Or<(` breakdown; R1 gains `KE10`/`KE11` and `single_mut`; R4 and R6 gates re-axed; a new **§Open ballots on this ladder** |
| [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) | C3 rewritten against the measurement (→ AB-1, AB-2); D5's five pin tests enumerated so citations resolve; dangling `F#` citations substituted; a new **AIR-10 familiarity / false-friend audit** section; the D3 numbering gap recorded |
| [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md) | `without Stunned` → `disabled Stunned` with the corrected polarity and its mechanism; event bounds symbolic; `requires` over dense storage recorded as a known-open hole (→ AB-6); the `each par` driver honesty edit (→ AB-8) |
| [`aether-v2/MACHINES.md`](aether-v2/MACHINES.md) | scenario 2 declares the `regen_mana` it orders against; the `MIN_ARCHETYPE_FOR_PARALLEL` floor stated; router mechanism held open (→ AB-5); R-DENSE re-grounded (→ AB-7) |
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
| Aether R3 | AB-1, AB-2 (event construct) · AB-6, AB-8, AB-11, AB-13 (the rest of the surface) · AB-10 on its keyword surface |
| Aether R4 | AB-3, AB-4 |
| Aether R5 | AB-5, AB-7 |
| Aether R8 | AB-9 · ~~GB-8~~ — ⚠ **MIS-FILED, and the ruling splits it**: GB-8's ID census lands at **Gaia G0**, and only the LINK census rides R8. Filing it against R8 alone dropped the half that carries the work |
| Gaia G0 | — (GB-8's id-census half, per its ruling) |
| Gaia G1 | ~~F1~~ · ~~GB-5~~ — both RULED 2026-08-30; the field-table freeze is unblocked |
| Gaia G2–G3 | ~~GB-2~~ · ~~GB-3~~ — both RULED. ⚠ **MIS-FILED**: GB-2's own body says it blocks **G2/G5**, and this row dropped the G5 half. Still open on G2: **F9's residual**, and on G3: **F8** |
| Gaia G4 | ~~GB-1~~ · ~~GB-4~~ — both RULED |
| Gaia G5 | ~~F2~~ · ~~F3~~ — both ANSWERED. ⚠ Plus **GB-2**'s G5 half, which this table never filed |
| Gaia G6 | ~~F4 (widened)~~ · ⚠ **AND F5, which this row omitted** — F5 blocks G6 by its own body, and its ruling REWRITES G6's scope. Both ANSWERED 2026-08-30 |
| Gaia G7 | ~~GB-7~~ · ~~GB-9~~ — both RESOLVED. ⚠ **AND F6, which this row omitted** (F6 blocks G7 by its own body; RULED 2026-08-30). ⚠ **GB-6 was MIS-FILED here**: its own body names **G6**, not G7 — and it is now DISPOSED (2026-09-03), its (b) half moving to **Aether R3**. ⚠ **G7's POSITION changed 2026-09-03**: the owner ruled that editor v1 authors UI documents too, so G7 moves UP the ladder rather than sitting after G6, and the `.ui` deletion census is written before the windowed pass. Its blocker is unchanged — UI reaching the screen, which is a wiring gap, not a missing plugin |
| Gaia G2 / G6 | ⚠ **F9, which appeared against NO rung in this table at all** though its own body, `gaia/CAMPAIGN.md` and PENDING Part D all say G2/G6. Partly ruled; its residual VALUES question is still the owner's and blocks **G2** |
| Aether R3 | ⚠ **GB-6(b)**, moved here 2026-09-03: an Aether construct that toggles a flag |

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
