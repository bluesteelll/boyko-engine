# CR — colour passes: CR-F0, the finisher advance as a region policy axis (record)

CR is the lever of ruling 25 (2026-10-06): fewer colour passes, or a cheaper advance between them.
The design's finding (ruling 26(a)) is that J-T already sits at its colouring lower bound (16 colours
= Δ), so a better colouring cannot cut passes on it. CR therefore became three rungs:

- **CR-F0 / CR-F1** — a *finisher advance*: the participant whose completion add makes an item's count
  exact publishes the next item, instead of participant 0 waiting for every count. Value-neutral,
  modelled +0.00..+0.06 ms per step (NOT IDENTIFIED). CR-F0 (this record) is the threadpool axis;
  CR-F1 is the solver's one-type switch, shipped only through a timed bar.
- **CR-SP** — spatial phases with cross-cell lanes: a screen arm only (value-changing).
- **CR-2** — two-item pipelining: gated, after IB and CR-F1.

Sources: the design `D:/tmp/phys-orch/cr/design_r2.md` (§4, §16) and its final critique
`cr/critique_final.md`; the plan `phys-next/LEVERS-R2-PLAN.md` rev 2 (§2.1, §4.1, §6); the lane cut
`cr-f/cut.md` and its review (with the implementation's addendum at its end), and the fix round
`cr-f/fix_r1.md` (after `cr-f/test_r1.md` and `cr-f/triage_r1.md`). Code is cited by symbol at the
CR-F0 code tip `50d66aa0` on `u/phys-cr-f`, and at `08d522cd` for what fix r1 changed. This file is
not in `tests/internal_docs_anchors.rs`'s `GATED_DOCS`; its anchors are not machine-checked.

---

## 1. CR-F0 as built (`crates/boyko_threadpool`, value-neutral, no physics source edit)

### 1.1 The axis

- `RegionPolicy::ADVANCE: Advance`, **defaulted** to `Advance::Orchestrator`, so `V2Policy` and every
  existing policy are unchanged (`V2Policy` is not edited; it stays the loom and ω_b baseline).
- `WithAdvance<P, const FIN: bool>` copies every claim and ladder const of `P` and sets
  `ADVANCE = Finisher` when `FIN`. The claim encoding of `P` and `WithAdvance<P, _>` is therefore the
  same by construction (unit test `with_advance_copies_every_claim_and_ladder_const`, n 0..=128 × P
  1..=16), and `RegionFrame::new`'s Safety contract says so: frames of the two may alternate over one
  claim column (gated by G-CR-F-ALT).
- The finisher runs only in **disarmed** monomorphs with **at least two** participants. An `ARMED`
  region runs the orchestrator twin (the per-item hooks are participant 0's; gated by
  `fin_armed_runs_the_orchestrator_twin`), and one participant keeps `run_orchestrator`.

### 1.2 Two role entries, no runtime index branch

`PoolInner::region` and `region_on_threads` call `run_participant0` on the calling thread and
`run_helper_participant` in every helper task. Each holds `if const { fin::<P, ARMED>() }`, so a
monomorph contains either the finisher or the v2 loop, never both, and no participant-index test
exists (the design's `run_participant` had one, which critique O1 showed may not fold). Loom drives
the same two entries (§3).

### 1.3 The protocol (`run_finisher<const P0: bool>`, `p0_boundary`, `publish_next`, `wait_publish_fin`)

Each item boundary has exactly one advancer:

| boundary after published item i | advancer | action |
|---|---|---|
| i + 1 is published | the completer of i (any participant) | `poison` (Acquire) → reset `done(e_{i+1})` (Relaxed) → publish `g_{i+1}` (Release) |
| i is the last item | the completer of i | `poison` check → the normal END (Relaxed) |
| i + 1 is inline | participant 0 | its own add completed i, or its Acquire load / done-wait read the exact count → the inline items in order → reset + publish, or END |
| region start | participant 0 | the open, then as an inline boundary |

- **The completion add.** `claim_sweep` gained `const FIN: bool`: one sweep body for both paths, so a
  later fix cannot land in one copy only. Under `FIN` the per-block add (or R-a's batched add) is
  `AcqRel` and the sweep returns, and stops, when this participant's add made the count `n_blocks`;
  under `FIN = false` it is the v2 sweep. `sweep_fin` is the `FIN = true` wrapper.
- **Waits.** `wait_publish_fin` loads `poison` (Acquire) on every iteration: after a helper's panic no
  END is certain (only participant 0's guard publishes one), so the poll is every survivor's exit.
  `poison` is word 1 of the sync line whose word 0 the wait spins on, so the poll adds no line
  traffic. **Participant 0 waits on `ORCH_WAIT`, helpers on `HELPER_WAIT`** (critique W-A): SR's
  waiter axes Rd/Rd2 (helpers) and Re (participant 0) keep their meaning, and `Re+fin` / `all+fin`
  mean "participant 0 waits for the publish, made by the completer, on its own ladder". No run can
  see which ladder waited (the finisher's wait is disarmed; loom models PAUSE and yield alike), so
  the choice is the `const fn publish_wait_ladder::<P, P0>()`, gated by value in `region::tests`
  (`a_finisher_waits_for_a_publish_on_its_roles_ladder`, two policies whose ladders differ; fix r1
  F2), and in codegen by the FIN receipt's yield references (§4).
- **Inline items never leave participant 0** (W4): `p0_boundary` is reached only on participant 0's
  paths, and `fin_inline_runs_stay_on_participant_0` / LF2 gate it.
- **Ruling 17 B1 for any advancer.** `publish_next` resets the done line before the Release publish in
  program order; every add of the entry's previous execution happens-before the reset through the
  chain of completer adds, publishes and done-acquires since.
- **END ordering.** The normal END stays Relaxed (its readers read no block data; the caller's reads
  are ordered by the scope's join, which is the edge that replaces participant 0's done-acquire when a
  helper completes the last item). Poisoned ENDs stay Release. A completer that passed its poison
  check can overwrite a poisoned END with an epoch; the survivors' sweep filters and polls end the
  region (the orchestrator has the same check-then-publish window).
- **Counts.** Under the finisher participant 0 no longer sees every item, so the report's
  `published`, `inline` and `max_blocks` are a walk of the table (`table_stats`, no allocation).

### 1.4 The advance receipt (critique O2)

- Receipt word `R_ADVANCES` (`RegionReceipt::advances`; loom `RECEIPT_WORDS` 6): the publishes a
  participant made — every epoch and the normal END, by any participant, participant 0 included;
  poisoned ENDs excluded.
- `RegionReport::{advances, helper_advances}`. **The identity, stated once on
  `RegionReport::advances`:** on every region that returns, `Σ_p receipt(p).advances == advances ==
  published + 1`. `report` debug-asserts it on every region; `check_report` (region tests), loom's
  `assert_receipts` and ω_b's `Frame3::check` assert it in release. Under the finisher it is the
  witness that the schedule ran, since `published` is a table walk.
- The orchestrator's count (`published + 1`) is recorded once per region, after `run_orchestrator`
  returns and only after a normal END: `orchestrate` (every caller of the orchestrator goes through
  it) calls `Participant::record_orchestrated_advances`, which reads participant 0's own exit word and
  stores the advance word. No increment in v2's loop. Fix r1 (triage N1) moved it there from the END
  exit, where the one store re-allocated registers across v2's whole item loop (§4). A poisoned
  orchestrated region therefore counts no advance in any receipt (asserted in `region_panic`'s
  orchestrator cases).

### 1.5 Cost when off

`V2Policy`'s monomorph gains the `R_ADVANCES` store in `store_receipt` (normal exits and the unwind
body), one out-of-line `record_orchestrated_advances` call per region after `run_orchestrator`
returns, `report`'s two sums and the `Participant` / `RegionReport` layout growth (+8 B, +16 B).
`claim_sweep::<_, _, V2Policy, false>` returns a constant `false` its callers ignore. `Participant`
is `repr(C)` with `advances` after `armed` (fix r1 (h)): four adjacent zeroed `u64`s were one ymm
store, after which LLVM put a `vzeroupper` before every call of the v2 helper task, the per-block
stage call among them; every older field keeps its v2 offset. G-CR-F-RCPT (§4) is the codegen
receipt for this list; its fix r1 reading is that v2's per-item and per-block code is v2's.

---

## 2. Native gates (`tests/region_protocol.rs`, `tests/region_panic.rs`, `tests/region_common/`)

- **G-CR-F-PROTO.** T1/T2/T3 under `WithAdvance<X, true>` for the six bounded policies
  (`fin_t1_t2_*`), with a native LF4 (a helper advance per P ≥ 2); T3/T4/T5/T8 twins; T6 and T7
  finisher legs; `fin_armed_runs_the_orchestrator_twin`; `fin_a_helper_advances` (a deterministic
  rendezvous part and a P4 part); `fin_inline_runs_stay_on_participant_0`, whose first part makes a
  helper the completer of an item followed by an inline item in every region (review O3); the SB
  leg's two finisher legs.
- **G-CR-F-ALT** (`fin_alternating_advance_over_one_frame`): 200 regions over one frame alternating `P`
  and `WithAdvance<P, true>` for five policies, P = 4, entries `[2, 6, 3]` — a shape for which the
  harness layout check fails in both directions if `WithAdvance` changed `HOME_LINES` (critique O6).
- **G-CR-F-PANIC.** The finisher twins of cases 1–4, W8, W3; F-FRAME B's `Fin<V2Policy>` round; and
  `fin_helper_panic_while_participant0_waits`: a rendezvous holds participant 0 in block 0 until a
  helper starts block 1, and that helper panics 5 ms later, so participant 0 waits in
  `wait_publish_fin` with nobody left to publish an END; unbounded `Fin<V2Policy>`, W 2/4/8, a 10 s
  watchdog per attempt, a `fired` anti-vacuity. One new ignore site, `miri-unsupported` (cut Q4).
- **Fix r1:** the orchestrator cases of `region_panic` (cases 1–4) assert that no receipt of their
  poisoned region counted an advance (case 4 is red when `record_orchestrated_advances` records on
  any exit; in cases 1–3 participant 0 unwinds and never reaches it);
  `region::tests::a_finisher_waits_for_a_publish_on_its_roles_ladder` gates W-A (§1.3).
- **The bounded red that physics cannot carry** (critique W-B): "the finisher omits its last add" is
  red in every `fin_t1_t2_*` as `RegionWaitBound { participant: 0, g: 2 }`; no physics test carries
  it.

## 3. Loom (`tests/loom_region.rs`; CI's `loom_region` step)

- The 15 models restated through the two role entries, names unchanged, each with a `_fin` twin
  (`twin!`), plus LF1 (reset before publish), LF2 (inline boundary stays on participant 0), LF3 (a
  helper panic while participant 0 waits), LF4 (a helper advances): **34 models**, pinned in CI
  (ruling 17 N5's rule).
- Bounds: M-R1, M-R4, M-R15 and their twins at 1 inside the model; the rest at 2 by default, 3 in CI.
- Measured 2026-10-07 (x86_64-pc-windows-msvc, debug, one model per process): bound 2, 34/34, the
  slowest M-R6 8.2 s; **bound 3, 34/34, the slowest M-R6 52.7 s** (no model near the cut's 600 s
  rule, Q5). The CI step as CI runs it: `running 34 tests`, 34 passed, **336 s** of test time
  (358 s wall with the build), against 241 s at 15.
- The 13 recorded V2 mutations, re-read on the restated harness: every one red as recorded, on both
  twins wherever the site is shared. The finisher's rows (each red): no inline-boundary wait and that
  boundary's load `Relaxed` (M-R1_fin, LF2; critique W2), the leading inline item skipped (M-R2_fin),
  participant 0 never completer (M-R3_fin), the completer test reloading the count (M-R4_fin), the
  live epoch (M-R5_fin), the reset after the publish (M-R13_fin, LF1), no poison poll (M-R15_fin,
  LF3), a helper running the inline item (LF2), the entries ignoring `ADVANCE` (LF4), `Relaxed`
  completion adds (M-R1_fin, M-R5_fin, LF1, LF4). The table is in the file's header.

## 4. G-CR-F-RCPT (the codegen receipt; the lane tester's round)

Method (cut §3 T5): build `omega_b_region` in `[profile.bench]` (`lto = false`, `codegen-units = 1`)
for PARENT = `git merge-base HEAD integ/unified` and the tip, with `-C symbol-mangling-version=v0` on
the final crate (legacy mangling hides generic arguments), extract the `V2Policy` and
`WithAdvance<V2Policy, true>` monomorphs by v0 substrings, and attribute every hunk of the V2 set to
one class of cut Q2 ((a) the receipt store, (b) the END-exit count, (c) `report`'s loads and sums,
(d) layout displacements, (e) `Frame3::check`'s identity asserts, (f) the SUMMARY keys) — plus, by the
implementation addendum, (g) `claim_sweep`'s constant `false` return if the parent's
`claim_sweep::<_, _, V2Policy>` is out of line. Any other hunk is red and goes to the orchestrator
with the asm. The FIN set must read no clock.

**The V2 set is a call graph, not a name filter** (triage r1 N1; fix r1). LLVM merges identical
monomorphs under one representative whose name may be another policy's: V2's orchestrator is
`run_orchestrator<FrameWords, Stages3, V3Rd, false>` in this binary, which the `8V2Policy` filter
never selected. Fix r1's extractor (`cr-f/fix1/rcpt2.py`) walks every function reachable from the
V2-named roots on both sides, pairs a callee with its namesake when its normalised name labels one
function on each side, and otherwise pairs it by call-site position (merged representatives,
compared body against body). A call whose target changed is a visible line change. Funclets pair by
identical body first, then by index.

**Results.**
- **Test r1 (name filter): RED**, two unattributed classes: (h) the v2 helper task's 12 new
  `vzeroupper` (the 32-byte `Participant` zero-init became one ymm store), and (i) callee renames
  under function merging.
- **Triage r1: a third, N1.** V2's merged orchestrator went 263 → 273 instructions: the END-exit
  `published + 1` re-allocated registers across its item loop, spilling the sync line's pointer, so
  all four `poison` polls and both publish stores gained a stack reload. Removing that one line
  restored v2's body.
- **Fix r1, parent `b46cf0ec` against the tip `08d522cd`** (155 pairs, 9680 → 9736 instructions,
  `vzeroupper` 46 → 46, clock sites 14 → 14):
  - the orchestrator is v2's body (263 → 263, no hunk: only `Stages3::run_block`'s impl index
    differs, body-identical);
  - the v2 helper task has no ymm and no `vzeroupper` (324 → 325; frame size and one 8-byte zero
    store, class (d));
  - `store_receipt` is class (a); no finisher code is reachable from a V2 root (the two
    `WithAdvance`-named functions reached are body-identical merged representatives);
  - **left for the orchestrator's ruling, by the letter outside (a)–(g):** (i) nine channel
    functions of ω_b's result transport, monomorphised over a message that grows 384 → 448 B with
    `RegionReport`, and three call sites retargeted to them in the row's `run_detached` task and its
    funclet (all after the region's timed interval); and the v2 row closure (442 → 473), whose
    per-region code carries the two `record_orchestrated_advances` call sites (one runs per region),
    `report`'s sums and the register re-allocation around them — inside ω_b's timed interval, none in
    a per-item or per-block loop.

  The pre-fix tip on the same extractor reads 12 hunks in the orchestrator and `vzeroupper` 0 → 12
  in the helper task (the method's red). The FIN set reads 0 yield references in participant 0's
  closure and 2 in the helper task (W-A in codegen).

## 5. ω_b (`crates/boyko_physics/benches/omega_b_region.rs`) and window SR

- `--policy` takes `<axis>+fin` on all eight SR-OMB3 arms (`v2epoch`, `Ra`, `Rb`, `Rc`, `Rd`, `Rd2`,
  `Re`, `all`): 16 policies. The default stays the eight orchestrator arms.
- `Bounded<P>` forwards `ADVANCE` (critique O2's trap). `Frame3::check` asserts the identity on every
  region, timed rows included. The bare self-check runs all sixteen arms and requires a helper advance
  in some region of every `+fin` arm (measured: 163–181 of each arm's 192 regions).
- SUMMARY gains `"helper_advanced_regions"` and `"helper_advances_median"` (cut Q3).
- Pre-flight (untimed): the 16-policy `--plan` row exits 0 with 512 `PLAN` lines and `PLAN_CELLS
  512`, `entries == stages` on each; SR-B's 8-policy row still reads 256.

**Merge to the trunk before window SR's prep**: the window's ω_b binary is built from the trunk
export that contains this merge. The window commands (prep, the timed SR-OMB3 block with CR's
twins) are the cut's §5, unchanged:

```text
O=<the window's omega_b_region exe, built from the trunk export>
$O --bench --mode region --policy v2epoch,Ra,Rb,Rc,Rd,Rd2,Re,all,v2epoch+fin,Ra+fin,Rb+fin,Rc+fin,Rd+fin,Rd2+fin,Re+fin,all+fin \
   --participants 2,4,8,16 --stages 36,72 --blocks-per-participant 1,2,4,6 --route worker --plan   # prep (c): 512 cells
$O --bench --mode region --policy v2epoch,Ra,Rb,Rc,Rd,Rd2,Re,all,v2epoch+fin,Ra+fin,Rb+fin,Rc+fin,Rd+fin,Rd2+fin,Re+fin,all+fin \
   --participants 2,4,8,16 --stages 36,72 --blocks-per-participant 1,2,4,6 --route worker           # timed, K = 9
```

**Readings (pre-registered in window SR's rows file):**
- M3 is unchanged: `v2epoch`'s w equals v2's within noise.
- The successor rule (plan §4.1, D21) reads only the eight non-fin arms.
- **CR-F1's bar item (1):** ω_b(successor+fin) < ω_b(successor), CLAIMED by the ruling-1 letter, at P8
  and P16, for bpp 1, 2 and 4, on that arm's own twin pair in the same 512-cell grid (`v2epoch+fin`
  against `v2epoch` if the successor is `V2Policy`).
- **Void rule:** a `+fin` row at P ≥ 2 whose SUMMARY reads `helper_advanced_regions == 0` did not run
  the finisher and is void.
- Bar items (2)–(4) (end to end non-inferiority, poses, `helper_advances > 0` per step) are read in
  window IB(+FM)'s POL-AB (plan §4.2). If (1) fails, CR-F stays an unused threadpool axis at zero cost
  (PC-CR-6).
- `report`'s table walk (at most one entry read per item, after the join) is inside the timed region:
  it is the finisher's real per-region cost, not a bias.

## 6. Where we stand against Jolt 5.6 (window 9b, 2026-10-05; unchanged by CR-F0)

J-T, 1,240 boxes, steady state [100,500), Jolt 5.6 in the same block, n = 9 per cell, timed trunk
`c335b4c6` (V2). No timing has run since; CR-F0 adds an axis the solver does not call.

| W | ours, ms | Jolt 5.6, ms | ours / Jolt | verdict (ruling 1 letter) |
|---|---|---|---|---|
| 1 | 8.228 | 9.419 | 0.874 | ours faster, CLAIMED |
| 2 | 5.473 | 5.631 | 0.972 | NOT CLAIMED |
| 4 | 3.774 | 3.617 | 1.043 | NOT CLAIMED; Jolt faster in every pass |
| 8 | 2.976 | 2.626 | 1.133 | NOT CLAIMED; Jolt faster in every pass |
| 16 | 3.274 | 2.402 | 1.363 | NOT CLAIMED; Jolt faster in every pass |

CR-F's modelled increment is +0.00..+0.04 ms at W8 and +0.00..+0.06 ms at W16 (NOT IDENTIFIED), which
is why it ships only through bar item (1), not an end-to-end gain bar.

## 7. The other CR rungs (status)

- **CR-SP** (design §5): spatial phases — 8 phases of a cell grid, cross-cell lanes grouped in
  super-batches, overflow colours — about 138 items per step on J-T against 266. Value-changing, so a
  **screen arm only** in the combined structural screen (rulings 25(c), 26(a); plan P15/P17), with
  pre-registered kill bars; its bit identity across W needs the super-batch invariant that replaces
  "colour = matching" in every cutter (critique W-C), which belongs to its screen, not to CR-F0.
- **CR-2** (design §6): two-item pipelining — a pipe item publishes once item j − 1 is fully claimed
  and j − 2 complete, blocks waiting on per-block stamps of their predecessors. **Gated** (plan D14):
  after IB and CR-F1.

## 8. PC lines (recorded here, not done by CR-F0)

- PC-CRF-1: `levers/scaling/02-SR-DESIGN.md` says CI runs "all fifteen" M-R models; it is 34 now, and
  its "every knob is a compile-time policy" should name `ADVANCE` and point here.
- PC-CRF-2: `levers/00-RULINGS.md` gets CR-F0's entry (the merge sha, window SR's `+fin` rows, CR-F1's
  bar) after the window.
- PC-CRF-3: `CLAUDE.md`'s ignored-test counts go stale by the one new `miri-unsupported` site.
- PC-CRF-4: under `WithAdvance<successor, true>`, SR-B's published/inline replica check compares a
  table walk with a table walk; use `RegionReport::advances == published + 1` and a helper-advance
  getter there (CR-F1).
- PC-CRF-5: CR-F1's physics gates keep FNV identity, the receipt and Miri; the "omits its last add"
  red is carried by the threadpool's bounded tests (§2).
- PC-CRF-6: `docs/SYSTEMS.md`'s `boyko_threadpool` section does not describe the region primitive.
- PC-CRF-7: specs should cite pins by constant name (the CR-F0 spec's literals were the pre-V2 ones).
- PC-CRF-8: the L10 pose-gate scripts live on D: (`D:/tmp/phys-orch/sr-b/gate/`), not in `%TEMP%`.
- PC-CRF-9: native `run_block` implementations must keep ignoring `participant` for the finisher to
  stay value-neutral (true of SR-B's solver at `9b2a7a3e`).
- Carried from the design: PC-CR-1, PC-CR-3, PC-CR-4, PC-CR-5, PC-CR-6 (conditional on bar (1)),
  PC-CR-7.
