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
spin; S7 rejected; S1 decided as SR), 2026-09-30 (V2 first; the cut's Q1–Q10). This file is not in
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
| done (≥ entries) | `w[0]` an entry's completion count; 0 between items | `region_done` |
| receipts (≥ participants) | `w[0]` region tag (base), `w[1]` blocks, `w[2]` exit, `w[3]` stalls, `w[4]` max wait ns | `region_receipts` |
| claims | `w[0]` the epoch of the block's last claim (R-b: a participant's ≤ 8 home blocks) | `region_claims` |

- `RegionLine` is `#[repr(C, align(64))] { w: [u64; 8] }`, `Copy`: a `ScratchColumn<T: Copy>` cannot hold atomics
  (cut F3), and loom's atomics cannot overlay raw memory. `RegionFrame::new` takes `&mut` borrows of the four
  groups (`RegionLines`) and projects them as atomic lines for the region's lifetime — the borrow checker, not a
  SAFETY contract, guarantees that nothing else touches them meanwhile. The one `unsafe` block is the projection.
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
  orchestrator waits for exactly `n_blocks` (Acquire) and resets the line to 0 before its next publish.
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

**The companion reset.** A poisoned region can leave the in-flight entry's done line non-zero. The next `open`
sees `poison == 1` and clears **every** done line of the column — not only the new table's entries (critique r1
W3(a): a smaller table B followed by a larger table C would otherwise hand C's entry `e` A's stale count) — then
stores `poison = 0`.

**Gates (Phase A):** `region_panic.rs::w8_poisoned_region_then_same_frame_completes` (P = 2, 8, 16 × participant 0
in an inline item, participant 0 in a claimed block, a helper in a claimed block; B over the same frame must
finish in < 1 s, every block exactly once, END and B's base in every receipt, `B.base == A.base + A.len + 1`);
`w3_poisoned_then_smaller_then_larger_table_completes`; loom M-R13. Mutation **M-W8** ("advance only after a
normal END") is red per profile: debug on the open's claim check, release on the `RegionWaitBound` payload.

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
- `tests/region_panic.rs`: rev 3's four cases at P = 2, 8, 16 with (a) payload, (b) receipts, (c) < 1 s; W8; W3.
- `tests/loom_region.rs`: M-R1…M-R15 (M-R14: a hint that names no earlier item, both the own-epoch and the
  lagging-sweep form; M-R15: participant 0's poisoned END carries a helper's poison to a third participant).
- `src/region.rs` unit tests: `hint_of`, `link_hints` and `SchedItem::new` by value (a clamp one step too tight,
  or a hint never used, is correct and only slower, so no exactly-once gate can see it).
- `tests/region_nested_scope.rs`: the debug guard (`Scope::new` asserts it is not inside a region block — a
  liveness rule: a join inside a block can steal its own region's helper task, which then spins until an END
  that cannot come).
- Miri, Stacked and Tree Borrows, on the region test binaries only.
- Mutations: M-G0, M-G1, M-W8, M-W3, M-OPEN, M-CAS, M-TTAS natively; one per loom model.

---

## 2. The physics half (Phase B — after V2 is on the trunk)

Ruling 2026-09-30 Q1: Phase B starts only after V2 (`u/phys-v2-spec`) reaches the trunk, because V2's diff edits
the same dispatch functions and substep loop and holds files Phase B must edit. Built behind a runtime switch
(Q8); one final commit flips it, deletes the per-colour scope path and re-pins the census once.

- **The table**, per step at W ≥ 2 with a wide colour: `Fill` (S4's ranges) → per substep `[Gravity → Warm c… →
  Biased c… → Integrate → Relax c… × relax_iterations]` → `Store` (only when no row can bounce). A colour below
  `wide_floor` is a one-block inline entry in colour-index order (KD8). Restitution stays serial (it takes
  `&mut CohortColumns`).
- **Grain** (`RegionGrain`, value-neutral, on the solver Resource): colours `clamp(min(max_bpp·P,
  ⌊points / colour_min_points⌋), 1, groups)` — today's cut exactly at the defaults `max_bpp = 6`,
  `colour_min_points = 64`; body stages `min(body_bpp·P, ⌈n_dyn / body_rows⌉)` 8-row aligned; fill entries S4's
  `setup_chunk_count` with a `fill_points` knob. **Every grain term is a knob** (critique r1 W1), so the Miri pile
  and the {1,N} proptest can force multi-block Fill, Gravity, Integrate, colour and Store entries.
- **V2 interplay:** no new serial stage. The speculative branch and K3 live inside the kernels; dp/dq is written by
  the Integrate blocks; V2's per-step Δ reset folds into the first Gravity stage (Q5) and stays in `build_bodies`
  on every non-region path. If V2 lands K3 as its own pass, SR carries it as one parallel per-substep stage (Q10).
- **Merge bar** (Q4): W8 J-T gain ≥ 0.6 × SR's predicted Δ on the contact set of the trunk it lands on, computed
  at window prep from that trunk's census; no W slower, W1 and W16 included.

## 3. Pins

Moved on purpose at the flip, each by `01-DESIGN.md` §6.10's counter rule (formulas in participants, substeps and
colours, never constants): `alloc_frame_census` S1c/S1e (per-colour scopes → 1 install + 1 np + 1 region), its
structural assertion, `alloc_frame_attribution` row D, `profiling_zone_counts` (the colour-scope rows, the
`PHYS_REGION_*` rows, every span and W8S wave row the region path emits), the runner's structure checks,
`default_world_worker_invariance`, `frozen_island_warm_start`, `colored_tests`' dispatch counters. Byte-identical:
every pose pin of the trunk SR lands on (pyramid `PINNED_FINAL_HASH`, A7-R1, GOLDEN, the parity runner's fixtures,
the L10 pose gates), the {1,N} oracle, `simd_solve` on/off.

## 4. Window 9b (the orchestrator runs it; nothing here is timed)

SR-AB (J-T, J-A, J-D at W 1/2/4/8/16, parent/tip adjacent, a Jolt 5.6 row in-block), SR-ARMED (the post-SR stage
map; `region_opens == 1` per step, `steps_with_zero_helper_blocks == 0`), SR-GRAIN (`max_bpp` {2, 4, 6} ×
`colour_min_points` {32, 64, 128, 256}; narrow floor {128, 256}; `--scene pairs`, Q7), SR-OMB3 (ω_b v3:
`omega_b_region --bench --mode region` over `V2Policy` and one policy per axis, next to v2's own arm — including
the helper PAUSE-only vs PAUSE+yield axis of ruling Q6), SR-JOLT. Rows: `scratchpad/sr/window9b_sr_rows.json`.
