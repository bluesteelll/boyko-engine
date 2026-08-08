# Diagnostics corpus — index

*Carved from `docs/PROFILING-SYSTEM-PLAN.md` (rev 4), `docs/LOGGING-SYSTEM-PLAN.md` (v4) and
`docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` (rev 1). Those three monoliths are the source of truth until
a later step retires them; until then, every file here names the sections it was carved from so a
reader can diff it against its source.*

**Status:** design. **Both architect blockers on rung D0 are now RESOLVED** — `LANE_COUNT` (Q1)
and the loss fold's lost-update window (Q2); see *Open, and who owns it* below. What remains
there is not a design question at all: one `rustup component add llvm-tools`, a D0 line item.
**D0 is unblocked.**

---

## What this is

One engine, two diagnostics subsystems, and the crate underneath them both:

| | Crate | What it is |
|---|---|---|
| **substrate** | `boyko_diag` (new, zero-dependency, the new bottom of the graph) | the ONE clock, the ONE lane topology, the ONE loss vocabulary, the ONE never-freed-storage policy — plus `profiling_abi`, which is *hosted* here for a graph reason, not because it is shared |
| **profiling** | `boyko_diag::profiling_abi` + `boyko_ecs::ecs::core::profiling` + the RHI zone seam | one ECS-native measurement subsystem replacing three hard-coded GPU timestamp enums, two env-var bench harnesses and ~600 lines of hand-rolled statistics. It answers six questions and **structurally refuses to answer a seventh** — there is no bare-delta constructor, only `Resolved{..}` or `NotResolved{reason}` |
| **logging** | `boyko_log` (new) | one in-house logger replacing 179 raw print occurrences across 36 files, 5 hand-rolled `AtomicBool` once-latches and 9 ad-hoc `boyko-####` codes in three incompatible text formats |

**Both subsystems serve two audiences, and the corpus says so everywhere.** The engine is
audience one: a fixed set of subsystems known at compile time, where the right answer to *"should
this site cost anything when disabled?"* is **nothing at all, not even a load**. A **game built on
this engine** is audience two: hours-long sessions, categories named by data or by a mod, a
console that toggles verbosity without a restart, and gameplay code that reads its own
diagnostics in-frame. The two are not the same customer. Every place they pull in opposite
directions is named, decided, **and costed for the side that loses** — six such places for
profiling (C-I..C-VI), five for logging (C1..C5).

---

## Why this corpus is split, and what the gate does

`docs/PROFILING-SYSTEM-PLAN.md` and `docs/LOGGING-SYSTEM-PLAN.md` were written separately and
reviewed separately, **twice each. Both passed.** The first reader to hold them side by side found
them asserting **contradictory facts about the same object**: profiling justified moving its ABI
into `boyko_utils` *because that crate has zero dependencies*, while logging stated flatly that
`boyko_utils` **depends on** `boyko_log`. Both cannot be true. Underneath that, each document had
independently invented the same four primitives — a per-thread lane index, an `rdtsc` calibration,
a never-freeing lane allocator, a loss accounting — with incompatible semantics.

Nothing caught it for three review rounds, because **nothing was watching the boundary**. Every
gate in both documents pointed inward.

Splitting the corpus into 22 small files makes each piece reviewable, and makes that failure
**more** likely, not less: two documents can disagree in one way, twenty-two can disagree in many.
So the split ships with a gate — [`scripts/check_doc_contracts.py`](../../scripts/check_doc_contracts.py).

Every file except this one declares, in a `CONTRACT` block directly under its H1, the capabilities
it **provides** and the ones it **assumes** from elsewhere:

```
<!-- CONTRACT
provides: substrate/lane-registry     one owner per capability; another file may assume it
exports:  profiling/gates             terminal: a human consumes it, no file need assume it
assumes:  substrate/clock-source      must be provided (or exported) by exactly one other file
-->
```

That turns *"do these documents agree?"* — a question that needs a reviewer to have read all of
them at once — into a graph the machine walks. Eight checks, and the real defect each one catches:

| # | Check | The defect it catches |
|---|---|---|
| 1 | **DANGLING** — every `assumes` resolves to a `provides`/`exports` somewhere | a piece leaning on a decision that was edited away in its owner |
| 2 | **AMBIGUOUS** — each capability id is owned by exactly ONE file | two pieces both claiming to own the clock — the S3/S4 defect, which is how one worker became lane 5 to one subsystem and lane 37 to the other |
| 3 | **SELF** — no file assumes what it provides | a piece that looks connected but is talking to itself |
| 4 | **CYCLE** — the assumption graph is acyclic | **the S2 defect directly.** "Profiling assumes utils is the bottom" plus "logging assumes log is below utils" is a cycle, and a cycle is exactly what a reader cannot hold in their head — which is why three rounds missed it |
| 5 | **ORPHAN** — every `provides` is assumed by someone | dead text, or a missing `exports:`. Both are worth one line to resolve |
| 6 | **INDEX** — every id appears in this README's map | the index cannot rot silently |
| 7 | **UNDECLARED** — a file that discusses another *area*'s file must declare a capability from it | **checks 1-6 all reason about the DECLARED graph, and a declared graph is only as honest as its declarations.** Measured on the first carved revision: 43 capabilities, 131 declared edges, check 4 green — while `substrate/00-GOAL.md` declared **zero** edges and discussed `SEAM.md` and a profiling file in its prose. Those undeclared references point **up**, and had they been declared, check 4 would have found a cycle. **Check 4's green was bought by silence.** Same-area siblings are exempt: within one subsystem a cross-reference is navigation between parts of one argument, and its cycle risk is contained |
| 8 | **ARITHMETIC** — every inline `a + b [+ …] = total` must hold | **the sums in this corpus have been wrong twice in two revisions.** Once inherited (a joint total that never equalled its operands in ANY revision), once introduced by the very edit that corrected it — which replaced the TOTAL and left the stale OPERAND in the same sentence, one commit after a paragraph was written warning about exactly that. **A total that checks out against its printed operands proves nothing about those operands**, and a prose warning did not survive one commit. Skips unit conversions (KiB terms, MiB total); tolerance is half a unit in each printed number's last decimal place |

⚠️ **What the gate does NOT check, measured and left unchecked on purpose: GATE IDS.** It walks
capability ids and area-qualified file paths. It does not know that `G23` was split into
`G23a`/`G23b`, so a re-specified gate leaves stale citations across the corpus and the gate stays
green — **four of the five findings in one post-repair sweep survived it exactly that way.**

A checker for it was written and **withdrawn on its own evidence**, which is worth recording so
nobody rebuilds it the same way. Gate ids in this corpus are **area-local and they collide**:
`G1`, `G4`, `G8`–`G18` and `GJ1` are each defined in *both* `logging/` and `profiling/` as
**different gates**. A corpus-wide id check is therefore unsound by construction — it would
silently identify logging's `G14` with profiling's `G14`, which is the very conflation this
corpus exists to prevent. An area-scoped version is sound but fired **36 times**, of which the
majority were false: ids are declared in more than one textual form, and a shared implementation
legitimately names its consumers' gates across areas. A gate needing human adjudication on every
row is noise, and noise gets switched off.

⇒ **A gate split or rename must be propagated by grep, deliberately, in the same commit.** The
real stale citations that check found before it was withdrawn (`G22`, `G23`, `G4`, `G2`, `G3`
cited bare where only split forms exist) are repaired; the historical ones — text describing the
*pre-split* gate as the defect being recorded — are deliberately left.

**Why capability ids and not file/anchor links.** Anchors churn on every heading edit, so an
anchor-based check becomes a nuisance and gets disabled — the same reasoning that made
`check_hotpath_exceptions.py` match on `(file, count)` rather than line numbers. A capability id
is a **name for a decision**: it survives the section being rewritten, and it stops surviving
exactly when the decision is deleted, which is when the alarm should fire.

**The gate refuses to be vacuous.** If `docs/diagnostics/` is absent it exits 1 rather than
passing quietly — a gate with no subject that reports success is the failure mode this campaign
has now caught nine times. It is wired into CI in the same commit that lands the corpus, never
before.

```powershell
python scripts/check_doc_contracts.py            # the gate
python scripts/check_doc_contracts.py --list     # print the map, in this README's map format
python scripts/check_doc_contracts.py --graph    # print the assumption edges, for eyeballing
```

The map table below is generated by `--list` and pasted, so it cannot drift from the contracts.

---

## Reading order

**Read `SEAM.md` first, whatever you came for.** It holds the decisions neither subsystem may
restate: the twelve seam decisions with both plans' dispositions merged into one row each, the
joint cost, the cross-plan landing order, the vocabulary renames, and **S13 — free when not
enabled**, the owner requirement folded in at the split.

1. **`SEAM.md`** — the boundary. S1..S13, the joint cost table, the landing order, the vocabulary.
2. **`substrate/00-GOAL.md`** → `01-CLOCK` → `02-LANE` → `03-LOSS` → `04-STORAGE` → `05-LADDER-GATES`.
   The substrate is below both subsystems and lands first (rungs **D0** and **D1**).
3. Then whichever subsystem you came for. Both are ordered the same way: goal and budgets → the
   hot path → the durable half → the game-facing half → the ladder and its gates → the
   dispositions.

The corpus is a DAG, not a chain, and the shape is stricter than an earlier draft of this paragraph
claimed: **no substrate file assumes anything at all outside itself** — not a `seam/*` capability,
not a plan capability, nothing. `SEAM.md` assumes only substrate capabilities. No profiling file
assumes a logging capability, and none the other way. That shape is what keeps checks 4 and 7
green, and a new edge that breaks it is a design change, not a formatting one.

*This paragraph previously carved out an exception — "`substrate/05-LADDER-GATES` assumes
`seam/free-when-off` alone" — and named it as the reason check 4 passed. The substrate repair had
already deleted that edge; verified by reading all six substrate contract blocks, which declare no
`seam/*` capability between them. **An index that explains why a gate is green is itself a claim,
and it rots exactly like any other.** Re-read the contracts, do not re-read this sentence.*

---

## The files

| File | Owns | For |
|---|---|---|
| [`README.md`](README.md) | *(index only — no contract block)* | orientation, the reading order, the capability map |
| [`SEAM.md`](SEAM.md) | S1-S13, joint cost, landing order, vocabulary, code space, lifecycle order, owner calls | the boundary between the two subsystems |
| [`substrate/00-GOAL.md`](substrate/00-GOAL.md) | why `boyko_diag` exists, the crate graph and its acyclicity proof, the mute-leaf rule | the four duplications the crate removes before they are written |
| [`substrate/01-CLOCK.md`](substrate/01-CLOCK.md) | the ONE clock and the 128-bit `SessionId` | ticks, calibration, epoch, both backends, both `unsafe` obligations |
| [`substrate/02-LANE.md`](substrate/02-LANE.md) | the ONE lane topology and its three write sites | so one worker carries ONE integer in both artifacts. **Blocker Q1 lives here** |
| [`substrate/03-LOSS.md`](substrate/03-LOSS.md) | the ONE loss vocabulary and the fold | so a profiler drop is a counter read, not a log record that can itself be dropped. **Blocker Q2 lives here** |
| [`substrate/04-STORAGE.md`](substrate/04-STORAGE.md) | S12's never-freed-storage policy and the ONE `.bss` section probe | compile-time extent ⇒ `.bss`; run-time extent ⇒ `VmReservation`. **The `llvm-tools` blocker lives here** |
| [`substrate/05-LADDER-GATES.md`](substrate/05-LADDER-GATES.md) | rungs D0/D1, gates DG1-DG12, the Miri/loom surface, the V1-V9 tree verification | the two rungs both subsystem ladders wait on |
| [`profiling/00-GOAL-TARGETS.md`](profiling/00-GOAL-TARGETS.md) | the six questions and the refused seventh; C-I..C-VI; the budget table; the eight invariants; the environmental constraints | what the profiler is for, and what it may never claim |
| [`profiling/01-EMISSION-STORAGE.md`](profiling/01-EMISSION-STORAGE.md) | the emission ABI (D1/D2/D3/D6/D19/D21/D15/D24) and the durable store (D8/D9/D16/D7) | the hot path and the fold, with the whole multithreading model |
| [`profiling/02-GPU.md`](profiling/02-GPU.md) | the GPU half and the RHI seam (D4/D4a/D5/D12/D14/D17/D18) | availability-polled readback, the witness, and why `WAIT_BIT` is *unrepresentable* |
| [`profiling/03-STATISTICS.md`](profiling/03-STATISTICS.md) | the VG R3 P4-6 facts, rules S1-S8, and the contrast API (D10/D11/D13) | the discipline that makes the wrong answer unrepresentable |
| [`profiling/04-GAME-FACING.md`](profiling/04-GAME-FACING.md) | D20/D22/D23/D25/D26/D27/D28 | the half a game uses: runtime toggling, retention tiers, telemetry, and what is refused |
| [`profiling/05-LADDER-GATES.md`](profiling/05-LADDER-GATES.md) | the 17 rungs, the gate table, the integration inventory | including the single subtractive rung and its two measured consumer lists. **The gate count is deliberately not stated here**: the table is 32 rows (33 with `GJ1`) over 26 distinct base numbers, none of which is the "28" rev 4's own checklist claims, and the owning file carries that as unresolved arithmetic rot rather than guessing which side was meant. An index must not launder a count its owner declines to settle |
| [`profiling/06-DISPOSITIONS.md`](profiling/06-DISPOSITIONS.md) | rev 1 / F1-F28 / X1-X25 / B1-B6 + M7-M13, open questions, checklist | every finding gets a row, including the ones the design refutes |
| [`logging/00-GOAL-TARGETS.md`](logging/00-GOAL-TARGETS.md) | the census arithmetic, C1-C5, the performance targets, the seven invariants, the sync-validation confrontation | what `boyko_log` replaces, and what it plainly cannot substitute for |
| [`logging/01-EMISSION-RING.md`](logging/01-EMISSION-RING.md) | deferred formatting, the three gates, the `.bss` statics, the SPSC ring (Decisions 1/1a/2/3/4/5/8/13/14/15/21) | the producer hot path and the transport |
| [`logging/02-SINK-LIFECYCLE.md`](logging/02-SINK-LIFECYCLE.md) | Decisions 9/9b/9c/10/11/12/22/23/24/26 | everything downstream of the ring, including the crash drain |
| [`logging/03-CODES-REGISTRY.md`](logging/03-CODES-REGISTRY.md) | Decisions 6/7/19, the walker, the migration ledger, the enforcement pair | the diagnostic-code registry and the corpus machinery that keeps it honest |
| [`logging/04-GAME-FACING.md`](logging/04-GAME-FACING.md) | Decisions 16/17/18/19b/20 and the reader surface | the half a game uses, and why a bigger ring does not raise the capture rate |
| [`logging/05-LADDER-GATES.md`](logging/05-LADDER-GATES.md) | L0-L17 + J1/J2, gates G1-G18 + P1/P2, the 34 mandatory tests | every rung independently green under `--workspace` |
| [`logging/06-DISPOSITIONS.md`](logging/06-DISPOSITIONS.md) | v1→v2, v2→v3, v3→v4, the scope extension, the refusals | including the refuted findings, with the evidence |

---

## Open, and who owns it

Three blockers travel with specific files and **must not be softened**:

- ~~**`LANE_COUNT = 32` in shipping is unsound against a worker-anchored topology**~~ —
  **RESOLVED: `LANE_COUNT = 80` in EVERY profile, no profile axis at all.** The defect was the
  axis itself: the const was profile-dependent while `MAX_WORKERS = 64` is not. 40 KiB of
  `LossCell` `.bss`, reserved-not-resident under S13. *(`substrate/02-LANE.md`, Q1.)*
  ⚠️ **It opened a new one, propagated at D1: the shipping budget.** Four cells across the two
  plans were sized by the deleted 32 — profiling's `LANES` and sample slab, logging's `LOG_LANES`
  and `SAMPLE_CTR` — so the retail figures moved 908.2 → **1 208.2 KiB** (profiler) and
  1 220.26 → **2 012.26 KiB** (logger), joint 2.08 → **3.15 MiB**. ✅ **RESOLVED 2026-08-08 — the
  owner raised the profiler's shipping budget from 1 024 to 1 280 KiB; no source changed and
  G23a/G23b are unblocked.** The number that alarmed everyone measured a *reservation*: the row is
  1 208.2 KiB declared, ≈ 1 142 KiB committed **only when diagnostics are armed**, and **≈ 0
  resident with the flag off** — which is the shipped default. *(`docs/OPEN-QUESTIONS.md`.)*
- ~~**`fold_into`'s lost-update window is not closed by `fetch_sub`**~~ — **RESOLVED: (b), the
  monotone counter.** The cell is never cleared; each consumer folds `cur.wrapping_sub(last_seen)`
  from its own 5 KiB of `last_seen`. Exactness follows from the shape of the datum rather than
  from every future producer remembering `fetch_add`, and `fetch_sub` leaves the design.
  `delta_since` ships at D0; `fold_into` does not exist. *(`substrate/03-LOSS.md`, Q2.)*
- **`llvm-tools` is not installed on this machine** — MEASURED: no `llvm-readobj` / `objdump` /
  `nm` / `llvm-nm` is on PATH, and the active `stable-x86_64-pc-windows-gnu` toolchain ships only
  `rust-objcopy` and `rust-lld`. The whole `.bss` gate family cannot run as written, and the gate
  must treat tool absence as a **RED, never a SKIP**. *(`substrate/04-STORAGE.md`; DG6.)*

Four calls need the **OWNER**, not the architect. They are collected in one place —
`SEAM.md` §*Open — needs the OWNER* (`seam/open-owner-calls`) — so neither plan can bury one in a
disposition table: the ~2 MiB joint shipping budget against the profiling plan's 1 MiB headline
(VALUES); whether `shipping-min` should also disable telemetry (SCOPE); **how the enable flag
arrives**, given that `env::args`/`args_os` appear **zero** times in this workspace (SCOPE, new at
the split); and whether `boyko_demo` keeps a third-party log facade at all (SCOPE).

---

## Capability map

Generated by `python scripts/check_doc_contracts.py --list`. Check 6 reads this table: every id
below must appear here verbatim, and every id declared in a contract must appear below.

| capability | owner | assumed by |
|---|---|---|
| `logging/budgets-and-invariants` | `docs/diagnostics/logging/00-GOAL-TARGETS.md` | `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/logging/03-CODES-REGISTRY.md`, `docs/diagnostics/logging/05-LADDER-GATES.md` |
| `logging/dispositions` | `docs/diagnostics/logging/06-DISPOSITIONS.md` | (exported) |
| `logging/emission-path` | `docs/diagnostics/logging/01-EMISSION-RING.md` | `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/logging/05-LADDER-GATES.md` |
| `logging/game-facing-surface` | `docs/diagnostics/logging/04-GAME-FACING.md` | `docs/diagnostics/logging/05-LADDER-GATES.md` |
| `logging/gates` | `docs/diagnostics/logging/05-LADDER-GATES.md` | `docs/diagnostics/logging/06-DISPOSITIONS.md` |
| `logging/goal-and-audiences` | `docs/diagnostics/logging/00-GOAL-TARGETS.md` | `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/logging/03-CODES-REGISTRY.md`, `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/logging/06-DISPOSITIONS.md` |
| `logging/ladder` | `docs/diagnostics/logging/05-LADDER-GATES.md` | `docs/diagnostics/logging/06-DISPOSITIONS.md` |
| `logging/registry-and-walker` | `docs/diagnostics/logging/03-CODES-REGISTRY.md` | `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/logging/05-LADDER-GATES.md` |
| `logging/ring-and-statics` | `docs/diagnostics/logging/01-EMISSION-RING.md` | `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/logging/05-LADDER-GATES.md` |
| `logging/sink-lifecycle` | `docs/diagnostics/logging/02-SINK-LIFECYCLE.md` | `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/logging/05-LADDER-GATES.md` |
| `profiling/budgets-and-invariants` | `docs/diagnostics/profiling/00-GOAL-TARGETS.md` | `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/profiling/02-GPU.md`, `docs/diagnostics/profiling/03-STATISTICS.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `profiling/contrast-api` | `docs/diagnostics/profiling/03-STATISTICS.md` | `docs/diagnostics/profiling/05-LADDER-GATES.md`, `docs/diagnostics/profiling/06-DISPOSITIONS.md` |
| `profiling/dispositions` | `docs/diagnostics/profiling/06-DISPOSITIONS.md` | (exported) |
| `profiling/emission-abi` | `docs/diagnostics/profiling/01-EMISSION-STORAGE.md` | `docs/diagnostics/profiling/02-GPU.md`, `docs/diagnostics/profiling/04-GAME-FACING.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `profiling/game-facing-surface` | `docs/diagnostics/profiling/04-GAME-FACING.md` | `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `profiling/gates` | `docs/diagnostics/profiling/05-LADDER-GATES.md` | `docs/diagnostics/profiling/06-DISPOSITIONS.md` |
| `profiling/goal-and-audiences` | `docs/diagnostics/profiling/00-GOAL-TARGETS.md` | `docs/diagnostics/profiling/04-GAME-FACING.md`, `docs/diagnostics/profiling/06-DISPOSITIONS.md` |
| `profiling/gpu-zone-seam` | `docs/diagnostics/profiling/02-GPU.md` | `docs/diagnostics/profiling/03-STATISTICS.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `profiling/ladder` | `docs/diagnostics/profiling/05-LADDER-GATES.md` | `docs/diagnostics/profiling/06-DISPOSITIONS.md` |
| `profiling/statistics-discipline` | `docs/diagnostics/profiling/03-STATISTICS.md` | `docs/diagnostics/profiling/04-GAME-FACING.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `profiling/store-and-fold` | `docs/diagnostics/profiling/01-EMISSION-STORAGE.md` | `docs/diagnostics/profiling/03-STATISTICS.md`, `docs/diagnostics/profiling/04-GAME-FACING.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `seam/build-axis` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `seam/decisions-s1-s12` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/logging/06-DISPOSITIONS.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/06-DISPOSITIONS.md` |
| `seam/diagnostic-code-space` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/03-CODES-REGISTRY.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `seam/free-when-off` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `seam/joint-cost` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md` |
| `seam/landing-order` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/03-STATISTICS.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `seam/lifecycle-order` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/profiling/02-GPU.md`, `docs/diagnostics/profiling/04-GAME-FACING.md` |
| `seam/open-owner-calls` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/06-DISPOSITIONS.md`, `docs/diagnostics/profiling/06-DISPOSITIONS.md` |
| `seam/vocabulary` | `docs/diagnostics/SEAM.md` | `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/03-STATISTICS.md` |
| `substrate/clock-source` | `docs/diagnostics/substrate/01-CLOCK.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/profiling/03-STATISTICS.md`, `docs/diagnostics/profiling/04-GAME-FACING.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/crate-graph` | `docs/diagnostics/substrate/00-GOAL.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md`, `docs/diagnostics/substrate/02-LANE.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/dedup-rationale` | `docs/diagnostics/substrate/00-GOAL.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/00-GOAL-TARGETS.md`, `docs/diagnostics/profiling/00-GOAL-TARGETS.md`, `docs/diagnostics/substrate/04-STORAGE.md` |
| `substrate/gates-dg` | `docs/diagnostics/substrate/05-LADDER-GATES.md` | `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `substrate/ladder-d0-d1` | `docs/diagnostics/substrate/05-LADDER-GATES.md` | `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md` |
| `substrate/lane-registry` | `docs/diagnostics/substrate/02-LANE.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/substrate/03-LOSS.md`, `docs/diagnostics/substrate/04-STORAGE.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/lane-write-sites` | `docs/diagnostics/substrate/02-LANE.md` | `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/loss-fold` | `docs/diagnostics/substrate/03-LOSS.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/loss-vocabulary` | `docs/diagnostics/substrate/03-LOSS.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/logging/04-GAME-FACING.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/profiling/04-GAME-FACING.md`, `docs/diagnostics/substrate/01-CLOCK.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/mute-leaf-rule` | `docs/diagnostics/substrate/00-GOAL.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/02-SINK-LIFECYCLE.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/substrate/03-LOSS.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/never-freed-storage` | `docs/diagnostics/substrate/04-STORAGE.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/01-EMISSION-RING.md`, `docs/diagnostics/profiling/01-EMISSION-STORAGE.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/section-report` | `docs/diagnostics/substrate/04-STORAGE.md` | `docs/diagnostics/SEAM.md`, `docs/diagnostics/logging/05-LADDER-GATES.md`, `docs/diagnostics/profiling/05-LADDER-GATES.md`, `docs/diagnostics/substrate/05-LADDER-GATES.md` |
| `substrate/tree-verification` | `docs/diagnostics/substrate/05-LADDER-GATES.md` | (exported) |

**43 capabilities, 21 contract files, 150 assumption edges.**

*These figures are what `--list` and `--graph` report, and they sit **below** the pasted table, so
they are NOT covered by its "cannot drift" property — regenerate them rather than trust them.
Measured at this revision: `--list` prints 43 capability rows; 21 files carry a `CONTRACT` block
(this README's fenced example is excluded, which is why the corpus has 22 files and 21 contracts);
`--graph` prints **150** edge lines, matching an independent count of the `assumes:` declarations
(151 corpus-wide, less this README's illustrative one at §*Why this corpus is split*). The **150**
are capability edges; their projection onto distinct file→file pairs — what check 4 walks for
cycles — is **93**.*

⚠️ *This paragraph previously asserted **131** edges (and **86** pairs) as "re-measured at this
revision", and both were wrong. The diagnosis is worth more than the correction: **131 was the true
count BEFORE the repair that this revision performed.** Check 7 turned roughly eighteen undeclared
cross-area prose references into declared `assumes:` lines — that conversion is exactly what moved
131 to 149 — and the index was updated to the number quoted in the finding document rather than to
the corpus that now existed. So the index re-stated a snapshot of the defect it was repairing,
while calling it a fresh measurement. **Regenerate these from `--list`/`--graph` AFTER the edit
that changes them, never from a number carried in prose.***
