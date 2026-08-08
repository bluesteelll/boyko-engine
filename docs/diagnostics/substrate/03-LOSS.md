# Substrate A3 — `loss`

<!-- CONTRACT
provides: substrate/loss-vocabulary     # the ONE loss vocabulary + the DiagFlag sticky-bit mechanism
provides: substrate/loss-fold           # the monotone counter + delta_since; Q2 RESOLVED (b)
assumes:  substrate/mute-leaf-rule      # raise/take_raised IS the mute leaf's report-above mechanism
assumes:  substrate/lane-registry       # cells are indexed [lane][class]; single-writer BY the topology
-->

> **Carved from** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` §3 A3 (in full), §10 Q2 / Q3 / Q4 and
> §11 F5, plus the first paragraph of the profiling plan's D24 (the sentence establishing whose
> vocabulary this is). Diff against those files until the monoliths are retired.

**The consequence of not sharing this, in one line:** the profiler reports its own drops
*through* the logger, so under load — precisely when profiler drops occur — the report of the
loss is dropped and counted as a *logger* loss; two counters double-count one event and no rule
says which is authoritative.

**Whose vocabulary this is.** The loss vocabulary is **`boyko_diag`'s**, not the profiling
plan's. The profiler's 18 drop classes map *onto* `LossClass`; they do not define it. What
sharing removes **by construction** is the second-order defect above: a profiler drop reported
through the logger is a log record, and a log record under load is a droppable thing — so the
report of a loss is itself lost, and the two subsystems' counters then disagree about one event
with no rule naming the authority. Sharing makes the report of a profiler drop **a counter read,
not a log record.**

---

## Types

```rust
#[repr(u8)]
pub enum LossClass { Overflow, Unclaimed, Late, Refused, Device, Sink, Rotation, Budget }
impl LossClass { pub const COUNT: usize = 8; }

#[repr(C, align(64))] pub struct LossCell  { count: AtomicU64, bytes: AtomicU64, _pad: [u8; 48] }
#[repr(C, align(64))] pub struct LossTotal { count: AtomicU64, bytes: AtomicU64, _pad: [u8; 48] }

pub enum LossStatus { Measured, Unproven, UnprovenLossy, UnprovenSampled, UnprovenUnsunk }

pub fn delta_since(cell: &LossCell, last: &mut LossSeen) -> LossDelta; // Q2(b): monotone,
                                                        // never clears the cell

#[repr(u32)] pub enum DiagFlag { /* sticky bits */ }
pub fn raise(f: DiagFlag);
pub fn take_raised() -> u32;
```

`LossStatus`'s five tokens are the **one status vocabulary shared with the profiler**, so a
reader who learnt the tokens in one artifact has learnt them in the other.

---

## Accumulate in `u64`, never saturate

Logging's saturating `u32` was justified by *"an 8-byte RMW is more expensive"*. **On x86-64
`lock xadd` costs the same at 4 and 8 bytes**, so the rejection does not survive.

With `u64` the **`SATURATED(>=4294967295)` census token disappears** — a token a census reader
could never compare against anything, because two saturated counters are equal at the token
level and unequal in fact. Removing the saturation removes the token, and removing the token
removes a reader's only way to be silently wrong about a magnitude.

---

## The cells are `AtomicU64`, accessed `Relaxed` by the owner (F5)

The decision record spells the per-lane cell as a plain `u64`, justified as "single-writer, no
lock prefix".

**A plain `u64` written by one thread and read by another is a data race and therefore UB in the
Rust abstract machine, irrespective of what x86-64 does. Miri reports it.**

`AtomicU64` with `Relaxed` load/store **lowers to exactly the same `mov` pair with no `lock`
prefix**, so the performance argument is preserved *verbatim* and the UB is removed. There is no
trade here; the record's spelling is strictly worse at equal cost.

`Relaxed` is the correct strength: **the cell is a pure counter and no data is published through
it** — the flag in `raise`/`take_raised` carries the ordering (below).

**Flagged onward to the logging plan:** the record cites this same argument as one logging
"already makes for `SAMPLE_CTR`". If `SAMPLE_CTR` is also a plain integer read across threads,
it has the identical defect. Out of scope for this file; named so it is not lost.

---

## Who writes what

Every producer lane writes **its own** cell — **single-writer by the lane topology**
([`02-LANE.md`](02-LANE.md)), not by convention and not by a lock. Each subsystem's consumer
folds. Cells are indexed `[lane][class]`.

---

## Layout arithmetic

`LANE_COUNT × LossClass::COUNT × 64 B` = 80 × 8 × 64 = **40 KiB** in the dev profile.

---

## `DiagFlag` — sticky bits

```rust
static DIAG_FLAGS: AtomicU32 = AtomicU32::new(0);   // own cache line, NOT in ClockGlobals
pub fn raise(f: DiagFlag)   { DIAG_FLAGS.fetch_or(f as u32, Ordering::Release); }
pub fn take_raised() -> u32 { DIAG_FLAGS.swap(0, Ordering::Acquire) }
```

**`Release` on `raise` pairs with `Acquire` on `take_raised`.** `Relaxed` would be enough *for
the bit alone*, but every `raise` is paired with a counter increment whose value the emitter
prints; **the pairing is what guarantees that a consumer which observes the bit also observes
the counter.** The cost is zero on x86-64 and the pairing is the reason it is written this way,
not the ISA.

**`swap(0)` — not `load` then `store(0)` — is what makes the take exact:** a `raise` landing
between a load and a separate clear would be dropped, which is **the same defect class as Q2**.

`DIAG_FLAGS` is deliberately **not** inside `ClockGlobals` ([`01-CLOCK.md`](01-CLOCK.md)):
`raise` dirties it, and a dirtied line shared with the clock would invalidate a line every hot
reader touches.

---

## `.bss` residency

| Item | Dev |
|---|---|
| cells (`LANE_COUNT × 8 × 64 B`) | 40 KiB |
| `DIAG_FLAGS`, padded to a line | 64 B |
| per-subsystem `LossTotal` arrays (2 × 8 × 64 B) | 1 KiB — **owned by the subsystems, not by this crate** |

**`boyko_diag`'s own total: ≈ 42 KiB in dev.** That figure is attributed to its row exactly once
(see [`00-GOAL.md`](00-GOAL.md)); the joint table already counts those bytes inside the two
subsystems' rows, and double-counting them would manufacture a footprint regression out of a
move.

---

## Q2 — `fold_into`'s lost-update window is NOT closed by `fetch_sub`. **BLOCKER for A3.**

S8 replaces `store(0)` with `fetch_sub(observed)` so that an increment landing between the
consumer's read and its clear is not lost. **That closes the *consumer* side. The *producer*
side stays open.**

An owner increment is `load; add; store`, and a consumer `fetch_sub` landing between that load
and that store is **overwritten by the store**:

```
owner:     r = cell.load()          ...                       cell.store(r + 1)
consumer:                  obs = cell.load(); cell.fetch_sub(obs)
```

The owner's `store` overwrites the consumer's subtraction; `obs` was **already folded into the
total**, so that one loss event is **counted twice** — the exact double-count S8 exists to
remove.

Two closures, with the cost of each:

**(a) Owner-side `fetch_add(1, Relaxed)`** — one `lock xadd` per **loss event**. Losses are by
definition rare (a loss is the thing being counted), so the RMW lands on the **already-degraded
path, never on the hot path**. Costs the "single-writer, no lock prefix" sentence — which was
never about the hot path in the first place.

**(b) Consumer-side monotone delta, no clear at all** — the cell is write-only-increasing; the
consumer keeps a per-`(lane, class)` `last_seen` and folds `cur.wrapping_sub(last_seen)`. The
owner keeps its plain `Relaxed` load/store pair (one `mov` each, no lock prefix), **exactness is
unconditional**, and `fetch_sub` disappears from the design entirely. Costs
`LANE_COUNT × 8 × 8 B` of consumer-side `last_seen` state (**5 KiB dev**) — owned by each
consumer, not by `boyko_diag`.

(b) is the standard monotone-counter fold and preserves the record's own performance argument.

### RESOLVED — (b), the monotone counter. `fetch_sub` and the clear leave the design entirely.

The cell is **write-only-increasing and is never cleared**. Each consumer keeps its own
`last_seen: [[u64; 8]; LANE_COUNT]` and folds `cur.wrapping_sub(last_seen)`.

**Why (b) and not (a) — the decisive reason is not performance.** Under (a), exactness holds only
while *every* producer, at every future call site, remembers to write `fetch_add`. One plain
`store` silently restores the double-count, and nothing fails until someone reads a loss report
that is wrong in the direction that hides work. Under (b) exactness follows from the **shape of
the datum** — a counter that only ever goes up — and there is no discipline left for a caller to
forget. This campaign's whole recent history is mechanisms that cannot be applied wrongly beating
rules that must be remembered.

Two supporting reasons:

- **(a)'s "rare by definition" premise is not proved for every class.** It holds for `Overflow`,
  where a loss IS the counted event. It does not hold for `Late` or `Refused`, which can be
  *systematically* high under a misconfigured window — precisely when an RMW per event is least
  welcome.
- **(b) keeps the leaf smaller**, which is this crate's stated entire value. The
  `LANE_COUNT × 8 × 8 B` = **5 KiB** of `last_seen` is owned by each consumer, not by
  `boyko_diag`, and it is per-consumer state that never crosses a thread.

**Ordering, and the one thing that must not be "optimised" later.** The fields stay `AtomicU64`
with `Relaxed` on both sides. A plain `u64` written by one thread and read by another is a data
race in Rust **regardless** of x86-64 making an aligned 8-byte access atomic in hardware;
`AtomicU64` + `Relaxed` lowers to the same `mov` pair with **no `lock` prefix**, so the
single-writer performance argument survives intact and the UB does not exist. Producer:
`c.count.store(c.count.load(Relaxed) + 1, Relaxed)`. Consumer:
`let cur = c.count.load(Relaxed); let delta = cur.wrapping_sub(last); last = cur;`

`wrapping_sub` is exact for as long as fewer than 2⁶⁴ increments separate two folds, and the
counter never resets, so there is no ABA to reason about.

**Consequence for D0: `fold_into` can now ship**, but its signature changes — it is no longer a
clear-and-add over a shared cell. The consumer-side reduction belongs to each consumer, so
`boyko_diag` exposes the cell and the delta helper, not a folding verb that mutates the cell.
DG5's RED is rewritten against the monotone form: **preset a cell, drop N with a live producer
thread running, assert the folded total advanced by exactly N and THE CELL WAS NEVER DECREASED.**
Replacing the delta with a clear-and-add reintroduces the window and reds. One gate still serves
both subsystems: profiling **G4b** = logging **G11** = substrate **DG5**.

---

## Q3 — `LossCell` padding

48 B of padding per `(lane, class)` costs **40 KiB in dev** and puts one lane's eight classes in
**eight** cache lines.

The padding exists to stop **cross-thread** false sharing, and **one lane's eight classes share
a single writer** — so between them it buys nothing. A per-lane `#[repr(C, align(64))]` block of
eight 16-B pairs is **10 KiB and two lines**, and the fold touches 4× fewer lines.

**Not taken here** because it changes a type the record declares. Cheap to take. Recorded for
the architect.

---

## Q4 — the paired-counter table for `DiagFlag` is UNASSIGNED

The mute-leaf rule ([`00-GOAL.md`](00-GOAL.md)) requires **every flag to have exactly one paired
counter**, so an emitter can print "N times" rather than "it happened". **The record specifies
neither the flag set nor the pairing.**

The **18** `W92xx` rows (`W9201`..`W9218` — consecutive, no gaps, so the count is the range) fix
the *profiling* half at logging L2. The **logging half** and **the
substrate's own flags** — `ClockEpochBreak`, `ClockUncalibrated`, `LaneExhausted` — are
unassigned.

**Needs a table before any emitter is written; owed by whichever plan lands its emitter first.**
