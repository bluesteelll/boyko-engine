# SR — the solve region: design record (Jolt-gap Lever 1)

SR is Lever 1 of the Jolt-gap plan: **one region per solve step** in place of one `pool.scope` per colour wave.
It runs stage 0 (the P-c fill, KD5), gravity, the per-colour warm start on the existing AVX2 cohort kernel
(KD3, S2 not S2′), the biased and relax sweeps per colour, integrate + inertia, the narrow colours inline in
colour-index order (KD8) and the warm store, all inside one region on a new kernel primitive,
`boyko_threadpool::region`. It is **value-neutral**: poses bit-identical at W 1/2/3/4/5/8/16, the {1,N} oracle,
the no-FMA census; the kernels' arithmetic is untouched.

Sources, in the order they patch each other: the Jolt-gap plan rev 1 (`architect_plan.md`: §2, KD2–KD5, KD8,
KD10, §5–§8 Lever 1, §11, §12), rev 2 (`architect_r2.md`, patches P1–P19), rev 3 (`PLAN.md`, patches
R3-1…R3-18), their three critiques, the SR cut (`scratchpad/sr/cut.md`) and its critique (`critique_r1.md`, six
Important remarks, resolved in the cut's addendum). The rulings: 2026-09-26 (scaling plan), 2026-09-29 (helpers
spin; S7 rejected; S1 decided as SR), 2026-09-30 (V2 first; the cut's Q1–Q10). Phase B: its cut (`sr-b/cut.md` in
the campaign's scratch, re-located on the V2 trunk `f277269d`), its critique's W1–W4 (resolved in the cut's
developer addendum) and rulings 17, 18 and 28 of 2026-10-01. This file is not in
`tests/internal_docs_anchors.rs`'s `GATED_DOCS`; its anchors are not machine-checked.

**Why.** At W8 our serial chain is 1.07 ms (58 % of the step) against Jolt's 0.62 (24 %). Warm-start apply runs
serially four times per step (0.31 ms); every colour wave is its own `pool.scope` (96 per step) with helpers
parking in every serial gap; tasks are capped at ~23.4 per wave; integrate is serial (0.12 ms). Priced
(arith., `scratchpad/sr/pricing.txt`): today's contact set W8 −0.243…−0.343 ms, W16 −0.225…−0.387 ms; V2's
contact set W8 −0.398…−0.599 ms, W16 −0.378…−0.673 ms.

---

## 1. The primitive: `boyko_threadpool::region` (Phase A)

### 1.1 Storage is the caller's (Principle 0)

The frame is plain data the caller owns — the physics solver keeps it in its Resource's `ScratchColumn`s:

| lines | words | owner of the column (Phase B) |
|---|---|---|
| sync (1) | `w[0]` publish (OPEN = base, an epoch `g`, `END_BIT \| base`), `w[1]` poison | `region_sync` |
| done (≥ entries) | `w[0]` an entry's completion count; reset to 0 just before each publish of the entry, its last count between items | `region_done` |
| receipts (≥ participants) | `w[0]` region tag (base), `w[1]` blocks, `w[2]` exit, `w[3]` stalls, `w[4]` max wait ns | `region_receipts` |
| claims | `w[0]` the epoch of the block's last claim (R-b: a participant's ≤ 8 home blocks) | `region_claims` |

- `RegionLine` is `#[repr(C, align(64))] { w: [u64; 8] }`, `Copy`: a `ScratchColumn<T: Copy>` cannot hold atomics
  (cut F3), and loom's atomics cannot overlay raw memory. `RegionFrame::new` takes `&mut` borrows of the four
  groups (`RegionLines`) and projects them as atomic lines for the region's lifetime — the borrow checker
  guarantees that nothing else touches them meanwhile. The one `unsafe` block inside is the projection.
- **`RegionFrame::new` is an `unsafe fn`** (ruling 17 A3, 2026-10-05, as the cut first specified; Phase A had made
  it safe). Its `# Safety` contract is the storage precondition the borrow checker cannot see: the sync, done,
  receipt and claim lines are exclusively the caller's for the frame's lifetime and not shared with another live
  frame (no other frame runs over them with a different epoch counter, now or between the owner's regions); they
  are sized for the schedule (a done line per entry, a receipt line per participant, `claim_lines` per published
  entry); the counter is the claim column's own (it only grows and is at or above every claim epoch). A breach a
  caller can make within the borrow rules (the lines' contents, a counter that is not the claim column's own, a
  column handed to two owners in turn, short columns) may hang the region or panic; with B1 (§1.3) it can no longer
  make two items overlap or run a block twice. Two frames live over the same lines at once would need two `&mut`
  borrows of them, which the borrow checker refuses. Every caller carries a `// SAFETY:` naming why the contract holds — the test harness's `Frame` (one value
  owning its columns and counter; `region_frame::<P>` is an `unsafe fn` that asserts the table's claim layout for the
  region's policy `P` and forwards the rest of the contract — ownership, the counter, and that the region runs `P`),
  the ω_b v3 bench's `Frame3` — and Phase B's caller derives it from the solver Resource's exclusively owned
  `ScratchColumn`s.
- **Four separate groups, never one merged column** (critique r1 W3(b)): a merged column would let a done or
  receipt value be read as a claim epoch when the table or P changes size.
- The stage table (`StageEntry`, 16 B: `n_blocks`, `first_claim`, and the caller's opaque `kind`, `color`,
  `first_cut`) and the schedule (`SchedItem`, 8 B: `entry`, `prev_off`) are read-only while the region is open.

### 1.2 The protocol (ω_b v2 as benched, refined by rev 3)

- **Epoch claims.** Item `i` runs at epoch `g = base + 1 + i`. A claim is `compare_exchange(hint, g, AcqRel,
  Relaxed)`; on `Err(w)` with `w < g` it retries once with `w`, on `w ≥ g` it skips the block. `hint` is the
  entry's previous epoch in this region (`base + prev_off`), 0 on its first execution — and 0 whenever `prev_off`
  names no earlier item (`prev_off > i`), so every expected value is below `g`, every successful claim raises its
  word, and any `prev_off` costs at most one retry. (Review r3 W1: unclamped, an expected value equal to `g`
  re-won blocks already claimed at `g`, and one above `g` let a lagging sweep re-win, and lower, a later item's
  words — both reachable through `SchedItem::new`.) Claim words are never reset (rev 2's ≈ 800-line serial
  reset is gone).
- **Tagged END and the open reset.** At open, participant 0 stores `publish = base` (OPEN) before any spawn; END
  is `END_BIT | base`; a helper that reads END debug-asserts the tag. `base ≥ 1` always (critique r1 O1), so a
  zero-initialised publish word or receipt can never read as a region's OPEN or tag. The normal END is a Relaxed
  store: a helper reads no data after END, and what the caller reads is ordered by the done counts and the scope's
  join (tester r3 N4: weakening it to Relaxed was invisible to every loom model and to Miri, because nothing
  consumes the edge). The poisoned ENDs stay Release: a helper's POISONED receipt depends on seeing `poison = 1`.
- **Exact completion.** `done.fetch_add(1, Release)` per block (R-a: one batched add per participant); the
  orchestrator waits for exactly `n_blocks` (Acquire). It resets the line to 0 **just before the item's publish**
  (ruling 17 B1, §1.3), not after the done-wait, so the count an item waits on starts at 0 whatever the line held.
- **Poison on unwind.** Each participant's tallies live in a guard. Dropped while armed it stores its receipt
  (PANICKED, or BOUND if the wait bound fired), `poison = 1` (Release), and on participant 0 the tagged END. The
  orchestrator's guard is created inside the scope closure before the first spawn, so an unwind drops it — and
  publishes END — before `Scope::drop` joins the helpers. The orchestrator checks poison before every item and on
  every done-wait iteration; helpers before every claim.
- **Waits.** Helpers never park inside a region (owner value 2026-09-29): `PauseThenYield { pauses: 5 }` is a
  burst of five PAUSEs then one `yield_now`, repeated (ruling 2026-09-30 Q6). The orchestrator pure-PAUSEs.
- **The wait bound** (tests only, `RegionPolicy::BOUND_NS`) is checked on every yield and every 64 PAUSEs of every
  ladder; past it the waiter records BOUND and panics with `RegionWaitBound`.
- **Work conservation.** Participant 0 alone completes every item; helpers that start after END read the tag and
  leave. A one-block item runs inline on participant 0 and is never published.

### 1.3 The W8 resolution: the epoch range is reserved at entry

Rev 3's critique (W8): `base` advanced only on the normal exit, so the region after a caught panic reused epochs,
skipped every block whose word already held them, and hung in the pure-spin done-wait.

**Fix:** `open` — the first thing `PoolInner::region` does — advances the caller's counter by `len + 1` before
anything can write an epoch: `base = max(*epoch, 1); *epoch = base + len + 1`. Every exit (END, poison, an unwind,
a bound panic, even an unwind out of `open`'s own debug checks) leaves the counter past every epoch the region
can write. One site, no exit path to miss, a kernel property every consumer gets.

**The done reset (ruling 17 B1, 2026-10-05; it replaces the companion reset).** A poisoned region can leave the
in-flight entry's done line at a partial count. Rev 3 and the cut cleared done lines in two places: after each
item's done-wait, and — when `open` saw `poison == 1` — the whole column (critique r1 W3(a): a smaller table B
followed by a larger table C would otherwise hand C's entry `e` A's stale count). Review r4 W1 (F-FRAME failure B)
showed that this left the safety of the barrier to the lines' history: a caller that re-creates its sync line after
a caught panic hides the poison from the next `open`, the stale count survives, and it completes an item after
fewer than `n_blocks` blocks — two items overlapped in 5 of 5 release runs through the safe API under `V2Policy`
(fix r4, tester r5). **Now participant 0 stores `done = 0` immediately before each item's Release publish.**
Every add of an earlier execution of the entry precedes that store in the line's modification order (in this
region its done-wait read exactly `n_blocks` of them, and exactly-once claims leave none to come; in an earlier
region the scope's join ordered them first), and the publish orders the store before every add of this item. So
an item completes only on its own `n_blocks` adds, and **two items can never overlap, whatever the lines hold**.
Same cost: one Relaxed store per published item, moved, not added. `open` now clears only `poison`, its
O(column) cold loop is gone, and so is the "pass the WHOLE done column" precondition and the debug "every done
line is 0 at open" check (a line now holds its entry's last count between items).

| word / line | reset to | by whom, when |
|---|---|---|
| sync line's publish word | OPEN | every open (participant 0, before any spawn) |
| sync line's poison word | 0 | the open that follows a poisoned region |
| done lines | 0 | participant 0, just before each publish of the line's entry (B1); never at open |
| receipts | never reset | tagged by `base` |
| claim words | never reset | epochs |

(The cut addendum's table, "Critique open question 1", read "done lines: 0 — participant 0 after each item's
done-wait; the whole column by the open that follows a poisoned region". Ruling 17 B1 supersedes that row.)

**Gates (Phase A):** `region_panic.rs::w8_poisoned_region_then_same_frame_completes` (P = 2, 8, 16 × participant 0
in an inline item, participant 0 in a claimed block, a helper in a claimed block; B over the same frame must
finish in < 1 s, every block exactly once, END and B's base in every receipt, `B.base == A.base + A.len + 1`);
`w3_poisoned_then_smaller_then_larger_table_completes`; `frame_b_a_stale_done_line_never_completes_an_item_early`;
loom M-R13. Mutation **M-W8** ("advance only after a normal END") is red per profile: debug on the open's claim
check, release on the `RegionWaitBound` payload.

**W3 and M-W3, re-expressed for B1.** The intent — a stale done line cannot complete an item early — is kept as a
gate that can fail. A is poisoned with entry 2's line partial; B (two entries) leaves it untouched, which the test
asserts, so C's first entry-2 item opens over A's stale count. C holds a helper in that item's block 1 for 50 ms
after participant 0's block 0, and the next item reads every slot: a count that started stale either completes the
item while block 1 is held (a generation mismatch) or overshoots and never completes (the test bound). C must
complete exactly-once, and every line C published must end at exactly its entry's block count. The old assertion,
"every done line is 0 after B and after C", tested the mechanism B1 deletes. **M-W3** is now "the done line is reset
after the item's done-wait (the old order), not before its publish" — with the open's column loop gone, that is
the old protocol minus its open reset, the state in which a stale line reaches a later item. It is also the
mutation the F-FRAME failure-B regression test (`region_panic.rs`) must be red on. M-R13's last assertion moved
from `done == 0` to `done == 2` (B's count from its own reset).

### 1.4 Every knob is a compile-time policy

`RegionPolicy` consts: `HELPER_WAIT`, `ORCH_WAIT` (`Ladder::{PauseThenYield, PureSpin, BudgetThenYield}`),
`BOUND_NS`, `DONE_BATCHED` (R-a), `HOME_LINES` (R-b), `TTAS` (R-c). The shipped `V2Policy` is
`{ PauseThenYield{5}, PureSpin, 0, false, false, false }`. `region::<S, P, ARMED>`: `ARMED` compiles in the stall
census (`REGION_STALL_NS` = 20 µs, helpers' recruitment wait excluded) and `RegionStages::item_begin/item_end`.
**No clock is read in the disarmed `V2Policy` monomorph** (`needs_clock` folds per monomorph).

### 1.5 Phase A gates

- `tests/region_protocol.rs`: T1/T2 exactly-once and visibility (a plain-memory generation instrument: every block
  of an item reads the previous item's writes, from other participants whenever the block counts differ) over
  seeded random tables at P ∈ {1, 2, 3, 4, 5, 8, 16}, both routes, every policy axis, armed and disarmed, with an
  anti-vacuity guard (helpers ran blocks); T3 report formulas; T4 work conservation; T5 two tables over one frame
  (the retry path); T6 the bound in every ladder and role; T7 the census; T8 hints that name no earlier item (the
  item's own epoch, the next item's, past the region, `u32::MAX`) under `TestPolicy` and, on a watchdog thread,
  the shipped unbounded `V2Policy`.
- `tests/region_panic.rs`: rev 3's four cases at P = 2, 8, 16 with (a) payload, (b) receipts, (c) < 1 s; W8; W3
  (re-expressed for B1); F-FRAME failure B (a poisoned region's count under a re-created sync line, `V2Policy`, a
  rendezvous that holds block 1 while item 1 may run, under a watchdog: red before B1 in release and debug).
- `tests/loom_region.rs`: M-R1…M-R15 (M-R14: a hint that names no earlier item, both the own-epoch and the
  lagging-sweep form; M-R15: participant 0's poisoned END carries a helper's poison to a third participant). CI's
  `loom` job runs all fifteen (ruling 17 N5): debug, `LOOM_MAX_PREEMPTIONS=3`, names listed and count pinned, with
  `--cfg loom` through a `target."cfg(any(unix, windows))".rustflags` key and `RUSTFLAGS` unset.
- `src/region.rs` unit tests: `hint_of`, `link_hints` and `SchedItem::new` by value (a clamp one step too tight,
  or a hint never used, is correct and only slower, so no exactly-once gate can see it).
- `tests/region_nested_scope.rs`: the debug guard (`Scope::new` asserts it is not inside a region block — a
  liveness rule: a join inside a block can steal its own region's helper task, which then spins until an END
  that cannot come).
- Miri, Stacked and Tree Borrows, on the region test binaries only.
- Mutations: M-G0, M-G1, M-W8, M-W3 (re-expressed for B1), M-OPEN, M-CAS, M-TTAS natively; one per loom model.

---

## 2. The physics half (Phase B) — as built on `u/phys-sr-b`

Ruling 2026-09-30 Q1 started Phase B only after V2 (speculative contacts, `SolveStep`, the tracked integrate)
reached the trunk; it was cut on `f277269d` (Phase A + V2 + C4-final) and written against V2's `SolveStep` from
the start. Seven commits: the warm kernels' cohort range (C1), the region behind a runtime switch (C2), the fill
as stage 0 (C3), the store as the last stage (C4), V2's per-step Δ reset folded into the first gravity stage
(C5), a trunk sync onto S5 (`775ea8c1`), and the flip (C6), which made the region the only parallel path, retired
the per-colour and setup scopes and re-pinned by the counter rule. Value-neutral throughout: every pose pin of the
trunk, both V2 and `d = 0` arm sets, the {1,N} oracle and the no-FMA census are unmoved.

### 2.1 When a step runs as a region

`build_columns` decides once per step, right after P-b: **P2** = `parallel_solve` ∧ a pool of at least two
workers ∧ the widest colour at least the grain's `wide_floor`, plus a laid-out cohort and a table whose entry
indices fit the region's `u16` (`region_fits`). A step that passes stops the build after the serial sources and
hands the fill over as a `FillPlan` (the cohort cuts, `fused`, the sources' plan counts, the tail); `solve_region`
(`#[inline(never)]`, one call per step) runs the fill and every substep as ONE `pool.region::<_, V2Policy, ARMED>`.
Every other step — one worker, `parallel_solve` off, no wide colour, the fast path — runs the serial loop, which
is byte-for-byte the per-wave path's serial arm (O5's 0 %-gate) and the {1,N} oracle's reference.
P = `pool.num_threads()`, no physical-core cap (ruling W8B-2).

### 2.2 The frame and the table

- **The frame lives in the Resource** (`RegionColumns` on `ColoredSoftStepSolver`): the four line groups of §1.1
  as four `ScratchColumn<RegionLine>` (sync, done, receipts, claims), and `entries: StageEntry`,
  `schedule: SchedItem`, `cuts: u32` (block boundaries) and `hints: u32` (`link_hints`' scratch). Every column is
  grown serially before the region, with zeros, and never truncated below its high-water length (Phase A's carried
  W3 obligation; ruling 17 B1 makes it defence in depth). `region_epoch` is the claim column's own counter,
  created with it and never reset. `RegionFrame::new`'s `// SAFETY:` derives ruling 17 A3's three clauses from
  that ownership.
- **Four scratch ids for eight columns** (cut Q1): only 352..=355 were free under `SCRATCH_REGION_MIN_ID`, so the
  frame takes one id per element type — one shared by the four line columns, one each for the entries and the
  schedule, one shared by the two `u32` columns. The registry keys on (slot, `TypeId`) and treats a
  re-registration as idempotent (the precedent is `mesh_draw.rs`' six `u32` columns on one id); columns that share
  an id share a stagger slot, harmless because frame lines are indexed by block or entry, never swept row by row
  beside each other.
- **Entries** (`SK_*` kinds; the fixed ones at `E_*` indices, debug-asserted): `Gravity`, `GravityFirst` (the
  first substep's gravity, its own entry, done line and claim lines — critique W1: a block is not told which item
  runs it, so one gravity entry could not tell substep 1 from 2..S), `Integrate`, `Fill`, `Store`, a test-only
  `Digest` (one inline block after the fill: the setup digest of the seeds before the first sweep), then per
  colour `Warm c`, `Biased c`, `Relax c`.
- **The schedule** is the serial loop's order: `[Fill, (Digest)]`, then per substep `[Gravity(First), Warm 0..C,
  (Biased 0..C) × B, Integrate, (Relax 0..C) × X]`, then `[Store]` when the store runs in the region. The builder
  takes S, B and X as counted loops (B = 1, X = `relax_iterations`, S = `substeps` today), so IB's presets
  (ruling 15) change only those three numbers, with no runtime branch; an entry is reused by each of its passes,
  each pass one item.
- **Blocks per entry**, every term a `RegionGrain` knob (critique r1 W1; the defaults reproduce the retired cuts):
  a colour's sweeps `clamp(min(max_bpp·P, ⌊points / colour_min_points⌋), 1, groups)` by the point-quota walk,
  cohort-snapped under `simd_solve`; a colour under `wide_floor` is one inline block, in colour order (KD8); its
  warm start the same count of cohort ranges (any cohort partition keeps each body's add order, L11 D7); the body
  entries `min(body_bpp·P, ⌈rows / body_rows⌉)` over every row, on 8-row boundaries (the Δ reset needs every
  row); the fill S4's terms `min(max_bpp·P, ⌊points / fill_points⌋, cohorts, SETUP_MAX_TASKS)` cut by
  `setup_cuts`, one inline block under two ranges; the store the fill's cohort ranges beside as many equal
  manifold ranges. `RegionGrain::is_valid` refuses a zero floor and a factor outside `1..=8` (the region asserts
  `n_blocks ≤ 8·P` at publish; critique O5).

### 2.3 `SolveStages` and the protector rule (the W5 write-up)

`SolveStages<'a>` is what every participant calls `run_block` through, by a shared reference. It holds only `Copy`
views and raw parts: the `CohortSolveView`, the body, snapshot and delta `ScratchSolveView`s, `&CohortColumns`
(whose heads the warm kernels read and whose struct bytes no block writes), the step's `SolveStep`, the fill's
and the store's inputs as raw bases, and the table. **It holds no `&[BodyState]` and no `&[BodyEffective]`.** That
is the W5 rule: a shared slice of a body column alive across the region would be a protected reference under both
Stacked and Tree Borrows for as long as it lives, and a gravity or integrate block's raw row write under it would
be UB even though no other block reads the row. So every block forms its slices from the raw views at block
entry, over its own rows, and drops them at block end:

- gravity / integrate: rows `[lo, hi)` of the body, snapshot and delta columns; the cuts partition `[0, rows)`;
- a warm start: the dynamic rows of cohorts `[lo, hi)` of one colour, pairwise disjoint (O4); the static rows a
  colour's lanes share are only read (the movability guard);
- a sweep: the impulse lanes of groups `[lo, hi)` of one colour and their dynamic rows, disjoint the same way;
- a fill: a block-local `FillCtx` (its two body slices formed at block entry, read-only: no block of the fill's
  item writes a body row), cohorts `[lo, hi)`, their rank rows, their lanes' `plan` / `tags` / record entries and
  its own out slot;
- a store: pass 1 the records of its cohort range's lanes (solved manifolds), pass 2 its manifold range's
  unsolved manifolds' carry — two disjoint write sets, each partitioned by the ranges — and its own carry slot.

`unsafe impl Sync for SolveStages` states exactly that, kind by kind. The protector rule is gated both ways:
red-first m8 (C2) gave `solve_region` a `&[BodyEffective]` parameter and Miri reported a protector violation at
the first gravity write in both legs. `run_block` is `#[inline(never)]` (one copy serves both `ARMED` monomorphs'
orchestrator and helper loops; critique O4). The hooks (`ARMED` only, participant 0 only, through a slot struct
reached by a raw pointer, since `ZoneGuard` is RAII and `ZoneCanary::at` takes `&mut self`) keep every span's
count: gravity and integrate around the first entry of the kind per substep, `PHYS_WARM_APPLY` from a substep's
first warm item to its last, the passes, the colour spans by the inline floor's predicate, `PHYS_SB_PC` (with its
W8S canary) around the fill, `PHYS_STORE` around the store.

### 2.4 The fill (stage 0, KD5) and the store

- **Fill.** On a region step the serial sources still run in `build_columns` (outside `PHYS_SB_PC`, critique O7:
  one span per step), and the ranges run as the region's first item. The ranges' reports reduce after END through
  the same `#[inline]` tail the serial fill calls (`fill_tail`), so the restore counts L10 reads are added after
  the region (critique O6) and `WarmSeedStats` is the serial fill's to the bit.
- **Store.** In the region only on a warm step where no row can bounce: `restitution_possible` is an O(rows) scan
  over every row, statics included, written as the restitution pass's skip rule's complement (NaN included),
  because restitution takes `&mut CohortColumns` and cannot be a stage. J-T's boxes are restitution 0, so its store
  is in-region; default components bounce (0.5) and keep the serial store. On an in-region store the
  `PHYS_RESTITUTION` span still opens with an empty body (the runner's header pin), the carry hits reduce in block
  order, and `capture_moved_in` runs before the swap (it reads the read side only).

### 2.5 V2's Δ reset — the corrected invariant (B-F2)

The earlier cut's addendum W4(f) said no Δ read precedes the first integrate. On the V2 trunk that is false:
substep 1's biased sweep reads Δ through K3 (`delta_copy`). **The invariant is that the reset precedes the step's
first sweep.** `build_bodies` lost the reset; a non-region step resets serially right after the build
(`reset_deltas`); a region step only sizes the column (grown with zeros or cut, never re-filled serially) and its
`GravityFirst` blocks write `Δ = ZERO` row-parallel before the first warm start. Red-first (C5): the reset moved to
the second substep's gravity reds the invariance gate on frame 0 at W2; the reset on the first gravity block's
rows only reds it on frame 1.

### 2.6 Counters, and the wave counters retired (cut Q2)

Seven counters, one sample each per solving step from the region's `RegionReport`, zeros on a step that opened
none (so `profiling_bit_identity`, which needs a sample on every counter, stays green untouched):
`PHYS_REGION_{OPENS, PUBLISHED, INLINE, HELPER_BLOCKS, BLOCKS_MAX, STALLS, MAX_WAIT}`. `PHYS_SETUP_CHUNKS` is the
fill's block count. At the flip the fifteen solve-side W8S wave counters (`PHYS_WAVE_*`, `PHYS_COLOR_SCOPES`,
`PHYS_COLOR_TASKS`, `PHYS_ROUTE_*`, `PHYS_SETUP_STAMPED`) retired with the scopes they read — an always-zero
counter is a check that cannot fail — and `WaveTally` with them; the narrowphase's wave counters stay
(`COUNTER_ZONE_COUNT` 56 → 41). The solver also exposes `region_dispatches()` (the census and the tests subtract
it) and a doc-hidden `last_region_report()` (critique W3: the counters test reads a region unarmed, so no second
bound profiler world is needed).

### 2.7 The gates

- `tests/sr_region_worker_invariance.rs`: the per-frame FNV of every body bit and of the step's `WarmSeedStats`
  equal to W1's at W 2/3/4/5/8/16, on the default arm (V2), scalar, reuse off, sleeping, `d = 0`, a bouncy arm
  (a static-floor drop row with a rebound witness) and every grain term lowered; a region on every frame.
- `tests/sr_region_counters.rs`: published / inline / widest item equal to a test-side replica of the builder at
  W 2/3/4/8/16 × four grains × `simd_solve`; helper blocks on every step at W8/W16 (load-sensitive, solo command
  in its header, critique O8); two allocations per warmed region step (the scope's frame and first chunk) under a
  thread-local counting allocator.
- `tests/sr_region_store.rs`: next-step seeds and warm statistics equal to W1's with the store in the region, on
  the serial store (bouncy) and with a frozen carry (sliders beside the stacks).
- The Miri legs, named targets only: SB through a `cfg(all(test, miri))` route that runs the region on
  `region_on_threads` with no pool (crossbeam-epoch's list is not SB-clean, so a pool cannot run under SB);
  TB through the same unit test and `tests/sr_region_pile_miri.rs` on a two-worker pool. Both lower every grain
  term and require, per kind, an entry of at least two blocks AND a helper block (a `cfg(miri)` per-kind tally,
  natively absent; critique W2), so a green says something about every kind's writes. Since the flip
  `colored_rigid_scratch_miri`'s pool arm drives the region under TB too.
- `colored_tests`: the random {1,N} gate covers the region on every multi-worker case; a lowered-grain {1,N}
  gate over random piles (hubs of 64+ colours, one-colour worlds); G4-A drives the fill through a Fill-only
  region (`fill_in_region`) with exact task counts forced by `SetupMode::Pool(t)`; G4-F keeps the std-thread
  twin with a `thread_fills` witness.

### 2.8 The census ledger (C6, counter rule)

| pin | before the flip | after |
|---|---|---|
| census S1c / S1e (release, debug) | 195..=195 / 195..=195 / dispatch MAX 391 | **3..=3 / 3..=3 / 7** on all 4,352 long-run frames, both profiles |
| census S1f (`d = 0`) | release 123..=135 / 271, debug 99..=111 / 223 | **3..=3 / 3..=3 / 7** |
| the census's per-frame structure | `scope − 1 − np − setup` a multiple of the pass count | `scope == 1 + np + region`, `region` read from `region_dispatches` |
| S8b exact | `scope == 1 + np` | unmoved; `region` asserted 0 (every colour under the floor) |
| attribution row D | a colour-scope model | `scope == chunk == 1 + region` per frame at W 2/4/8 and over the pass sweep; the lanes buy blocks per region item (widest item 24 at W4, 42 at W8, release scalar), never a chunk |
| `profiling_zone_counts`, the runner's structure checks | colour waves, setup wave | `PHYS_REGION_*` = the replica; `PHYS_SETUP_CHUNKS` = the fill's blocks |
| `default_world_worker_invariance`, `frozen_island_warm_start` | `setup_dispatches` | `region_dispatches` (frozen island: the premise flipped, B-F5 — the region opens where S4's setup never did) |
| UG-02 `ug02-pins.tsv` | `physics_pool_scope` 7, `physics_active_pool` 9 | **5 / 7** (row `SR-B-C6`: `fill_scope` and `solve_color_parallel`'s `pool.scope`, `fill_parallel` and `solve_color_parallel`'s `try_with_active_pool` deleted) |

The fan-out regression S1c/S1e were the gate of (+12 scope frames a frame) cannot occur any more; its successor,
a scope opened inside a region block (`01-DESIGN.md` §6.10), reds the census's structural assertion (4 against
`1 + 1 + 1` on S1c's first steady frame), row D (3 scope frames against 2) and, in debug, the threadpool's own
assertion. In release that red is not guaranteed to be printed (triage r1 F1): the scope's join can run the
enclosing region's own queued helper task, which then waits for a publish the participant inside the block cannot
make — the debug assertion's own text (`Scope::new`, `boyko_threadpool/src/scope.rs`) — so the release outcome is
the reds above or a deadlock, by scheduling. Measured both ways with one mutation spec (on the tip's `colored.rs`
it gives md5 `61c3f3ae…`): the lane's two release runs on its pre-flip scratch tree printed the reds, the tester's
three on the tip hung and were killed. Neither is a green; a hang is the CI's timeout.

### 2.9 The codegen receipts and the tool

Two receipts at the flip, from `cargo rustc --locked -p boyko-physics --lib --release --config
profile.release.lto=false --config profile.release.codegen-units=1 -- --emit asm` on two exported trees (PARENT =
`git merge-base HEAD integ/unified`, TIP = the flip), each with its own target dir:

- **W1**: the serial step's functions, PARENT against TIP, every normalised hunk attributed to a named change;
- **disarmed**: no clock read (`rdtsc`, `QueryPerformanceCounter`, `Instant::now`) reachable by direct calls from
  any `ARMED = false` function of `PoolInner::region::<SolveStages, ..>` (the monomorph and its closures, the
  helpers' task bodies included), and some clock reachable from the `ARMED = true` one; the solver calls exactly
  one of each. Red-first: `ARMED = true` passed on the disarmed route, in a scratch copy — RED.

The tool (`D:/tmp/phys-orch/sr-b/cg/`: `asmlib.py`, `fnasm.py`, `disarmed.py`), so the next receipt does not
re-write it (PC-SR-B14): rustc for `x86_64-pc-windows-msvc` emits COFF asm in AT&T syntax with v0 symbols by
default. A function is announced by `.def NAME;` and runs from its label `NAME:` to the next `.def` — not from a
label to `.seh_endproc`: `.Lfunc_begin` exists only for functions with EH tables, and the `$cppxdata$` /
`$ip2state$` data labels after a function open false functions under "any label starts one". Two exported trees
build every workspace crate with a different crate disambiguator (`Cs<base62>_`, a hash of the metadata, which
includes the source path) and so different back-references (`B<base62>_`); the extractor pairs functions by a
name with those, impl indices (`Ms<N>_`) and legacy hashes normalised, and normalises the same in the
instructions, plus local labels, constant-pool labels and every memory displacement, so a struct field that moved
does not hide a real change and a crate built from another directory does not invent one. It writes a raw and a
normalised diff per function and attributes nothing.

## 3. Pins

Moved on purpose, all at the flip, each by `01-DESIGN.md` §6.10's counter rule: §2.8's table. Byte-identical on
every commit of the lane (each commit's gates): every pose pin of the trunk SR landed on — V2's defaults (pyramid
`PINNED_FINAL_HASH` and `_REUSE_OFF` in both profiles, A7-R1, the parity runner's J500, J500 reuse-off and R1100 at
W 1 and 8, `pose_gates_v2.sh` 264/264 with both negatives exiting 4, 26/26, 17/17, V2's fidelity gate 4/4 with its
red control failing) and the `d = 0` arm set (pyramid `_D0` and `_REUSE_OFF_D0`, A7-R1 at `d = 0`, GOLDEN, the
runner's `d = 0` rows, `pose_gates_d0.sh`) — and the census's sixteen other scenes, `bp_query_counts` and
`narrowphase_census` (features), the {1,N} oracle, `simd_solve` on and off, W 1/2/3/4/5/8/16, and the no-FMA census.

## 4. Window 9b (the orchestrator runs it; nothing here is timed)

Window 9b ran no SR row (the phase B cut's B-F15), so SR ships `V2Policy` and today's grain unmeasured, and its
A/B is a window of its own ("window SR", the phase B cut's §5.2: SR-AB, SR-DYN, SR-ARMED, SR-GRAIN, SR-OMB3,
SR-JOLT). The rows below are what the plan first named for 9b.

SR-AB (J-T, J-A, J-D at W 1/2/4/8/16, parent/tip adjacent, a Jolt 5.6 row in-block), SR-ARMED (the post-SR stage
map; `region_opens == 1` per step, `steps_with_zero_helper_blocks == 0`), SR-GRAIN (`max_bpp` {2, 4, 6} ×
`colour_min_points` {32, 64, 128, 256}; narrow floor {128, 256}; `--scene pairs`, Q7), SR-OMB3 (ω_b v3:
`omega_b_region --bench --mode region` over `V2Policy` and one policy per axis, next to v2's own arm — including
the helper PAUSE-only vs PAUSE+yield axis of ruling Q6), SR-JOLT. Rows: `scratchpad/sr/window9b_sr_rows.json`.

## 5. Window SR — the commands (the orchestrator runs them; nothing in the lane was timed)

Written for the tip of `u/phys-sr-b` after the flip; no row passes `--sr` (the flag is retired). TIP = the
`u/phys-sr-b` tip, PARENT = `git merge-base u/phys-sr-b integ/unified` at prep (the trunk SR last synced to: S5's
`775ea8c1`, then dyn-scenes' `b46cf0ec`). The poses must also equal the trunk's fixtures, which the pre-flight checks.

**Pre-flight, untimed** (the lane ran every row at C6; the window prep re-runs them on its binaries):
1. The gate scripts are copies of the committed `docs/measurements/2026-09-30-v2-speculative/gate/**` with
   `D:/wt/merge` replaced by the tree the fixtures are read from; record every script's md5 before and after.
2. The runner from an export of each side, each with its own target dir:
   `git archive <sha> | tar -x -C D:/wt/_scratch/sr-run/<sha>`, then from there
   `cargo bench --no-run --locked --profile parity -p boyko-physics --bench jolt_parity_pyramid` (and
   `--bench omega_b_region` for the tip), with the lane prefix (`stable-x86_64-pc-windows-msvc`,
   `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`, no `RUSTFLAGS`).
3. The checks on the tip's runner: `runner_pins.sh <exe> <out>` 12/12; `pose_gates_v2.sh` and `pose_gates_d0.sh`,
   each 264/264, both negatives exit 4, 26/26, 17/17; J-T (`--scene jolt --gap 0.5 --cfg default --broadphase tree
   --sleeping off --steps 500`) at W 1/2/3/4/5/8/16 with `--expect-pose
   docs/measurements/2026-09-30-v2-speculative/w8/JT500.pose`, and with `--speculative-distance 0
   --speculative-velocity-cap 0` against `docs/measurements/2026-09-30-physics-window9a/gate/fixtures/JT500.pose`,
   every row exit 0; J-T `--arm-profiler` at W 8 and 16: `void_steps 0`, a region opened on every step, no step
   with zero helper blocks; the dynamic scenes' own gate, `docs/physics/perf-campaign/levers/scenes/gate/ours_gates.sh
   <exe> <out>` (its W-identity across W 1/2/4/8/16 runs through the region since the flip); ω_b v3
   `--bench --mode region --policy v2epoch,Ra,Rb,Rc,Rd,Rd2,Re,all --participants 2,4,8,16 --stages 36,72
   --blocks-per-participant 1,2,4,6 --route worker --plan` exit 0 with 256 `PLAN` lines and `PLAN_CELLS 256`; v2's
   arm `--bench --mode omega-b2 --participants 2,4,8,16 --stages 36,72 --blocks-per-participant 1,2,4,6 --helper
   spin --route worker --regions 0` exit 2 with its message; the bare runner and the bare omega bench exit 0.
4. The `u` attribution rows (triage r1 W1, below) on the tip's runner: J-T with `--steps 60 --arm-profiler
   --canary-zone phys_sb_pc --canary-ns 50000000` at W 1/2/8, each exit 0 — the canary's spin counted exactly once
   by `u`'s subtracted set, at W 1 inside the build span and at W ≥ 2 as the region's fill span. Untimed: both of
   the runner's bounds are load-robust (module docs, "The in-zone canary").

**Reading `u` and the build span on a tip row** (triage r1 W1, PC-SR-B6). On a region step the fill's span
`phys_sb_pc` is opened by the region's stage-0 hook after `phys_solve_build` closed, so it is a sibling of the build
span, not a part of it (`boyko_physics::profiling`'s module docs): a tip row's `phys_solve_build` lacks the fill a
PARENT row's held, and the two must not be compared. The runner counts `phys_sb_pc` among `u`'s subtracted spans on
exactly the steps whose `phys_region_opens` is 1 — until fix r1 it did not, and a region step's `u` carried the
fill. What stays in a region step's `u` is the region's own unzoned fixed cost: the `restitution_possible` scan,
the stage table's build, `pool.region`'s open, recruitment and join, the fill's tail and the store's carry
reduction. So a tip row's `u` is not the residue window 9b's closure rule (`u` above 3 % of the solve span stops the
analysis) was set on; on PARENT rows the rule reads what it always read. Whether the window's driver reads a tip
row's `u` as a report or re-derives that ceiling at prep is the orchestrator's call — the lane timed nothing, `u`
included.

**The timed window** (`W = D:/tmp/phys-orch/win-sr`; the driver is a renamed copy of window 9b's with SR's rows; run
through `Start-Process` in its own console — background shells are reaped after about 30 minutes — with no agent
session alive, `progress.txt` the only file polled, `--dry-run` first for TOTAL, the cutoff at launch + ⌈1.2 ×
TOTAL⌉, `--resume` always). Binaries `runner_parent_<PARENT>.exe`, `runner_sr_tip_<TIP>.exe`,
`omega_b_region_v3_<TIP>.exe`, Jolt 5.6 `PerformanceTest.exe` (the window 9b build), every sha256 in `SHA256SUMS`.
Claim cells are K = 9 (3 passes × 3 rounds, p0/p2 reversed).

| block | rows | reading |
|---|---|---|
| SR-AB | SR-JT (J-T as above), SR-JA (`--scene jolt --gap 0.5 --cfg a`), SR-JD (`--scene jolt --gap 0.5 --cfg default`); W 1/2/4/8/16; parent and tip adjacent; windows [0,100) (S-LAND) and [100,500); canaries SR-rung8 (`--canary-frac 1 --canary-ref-ns 60000`, W8) and SR-rung16 (`105000`, W16) on the tip, which must be SEEN | SR merges iff the W8 J-T [100,500) gain ≥ 0.6 × SR's predicted Δ low end on the synced trunk's census (0.237 ms on 9b's census; recomputed at prep by `s2_levers.py`'s rule) and no W is slower on J-T, J-A or J-D; poses equal across W and to PARENT |
| SR-DYN | `--scene jolt --dyn kick --steps 800` [200,800); `--dyn shoot --steps 800` [200,800); `--dyn slide --steps 500` [0,500); W 1/8/16, parent and tip; the pose reference is PARENT's W1 `--pose-out`, checked by `--expect-pose` on every tip W | no W slower on any program (ruling 21(c), the cut's Q4) |
| SR-ARMED | SR-JT with `--arm-profiler`, W 8/16, parent and tip | the tip: `region_opens` 1 per step, `region_steps_with_zero_helper_blocks` 0, `void_steps` 0; stalls, max wait and blocks max reported |
| SR-GRAIN | the tip, K = 3: `--sr-bpp {2,4,6} × --sr-min-points {32,64,128,256}` at W 8/16 on SR-JT [100,500); `--sr-floor 128`; `--scene pairs` over the same grid | `RegionGrain`'s defaults; a change is a later value-neutral commit under the same window rule |
| SR-OMB3 | ω_b v3 (the pre-flight row without `--plan`) next to v2's arm (without `--regions 0`) | M3: v2epoch's w equals v2's within noise; the axis winners become `V2Policy`'s successor, a one-const commit |
| SR-JOLT | Jolt 5.6 `-s=Pyramid -q=Discrete -f` at W 1/8/16, and `-dyn=kick|shoot|slide` (the parity patch v2) | standing, in-block |
