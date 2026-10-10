# MEASUREMENT-QUEUE §6–§9: the timings read against each entry's own rules

## TL;DR
- **§6:** no rule fires. The row identity stays as shipped, and stage 2 keeps sort + binary search.
- **§7:** R1 fires on the wording of the rule, on `full_step/4` and `stable/sleeping_off`. The margins are thin (as small as 0.02 pp at n=4) and rest on single runs. R1 asks for the `physics_apply` release listing to be inspected. R4 (the allocation census) was never run, so §7 stays open.
- **§8:** R1 fires on its wording (0.28 % against an A/A spread of 0.26 %), but the source code rules out its stated cause. R2 fires clearly: the awake row is 1.024 slower on B. R2 asks for the `island_of` hoist, then a re-measure. R3 and R4 pass.
- **§9:** S5 costs **1.38× to 1.41×** per step on the default-config pile. R2 does not fire, because the time ratio is below the contact-point ratio on every row. R3 does not fire.

## How I read the numbers
I recomputed every median, spread and ratio directly from the copied `raw/*/estimates.json` files, and all of them match the timing record.

- **"Outside the A/A spread"** means |B/A − 1| > (max − min) / mean of the A medians.
- **Run identity:** all 62 runs used the intended exe (sha256 matches `exes.json`), exited with code 0, and ran exactly the intended row ids.
- **CONTAMINATED:** 0 runs under the before-rule. Two runs waited one 59 s poll first.
- **Discarded:** 1 run, s9 `full_step/1` A2.
  - Its after-receipt was 16.66 % (per-second samples 13–28 %).
  - Its median's 95 % CI, 19.97–21.27 ms, does not overlap the CI of any other A run.
- **Flagged, kept:** s7 `churn_stable` B2 (before 4.1 %, after 5.4 %). It decides one R1 reading (see §7).

**Confound across entries: §7 and §8 have the same arm B.**
- `a56007ab`'s commit message says it fixes both defects: A4 (the wake-on-contact-change key) and A5 (`BodySetFilter`). Its `src` diff against `d552be05` is 1122+/191−.
- So every §7 row and every §8 row prices A4 + A5 together.
- **With sleeping off, A4 cannot run on `a56007ab`:**
  - `physics_solve_colored` calls `solve_colored_sleeping` only when `cfg.sleeping` is true (`systems.rs:1085-1097` at `a56007ab`).
  - `begin_step` is reached only through `if let Some(sleep)` (`solver/colored.rs:3257-3259`).
- So the sleeping-off rows price A5 plus the rest of the diff, and the sleeping-on rows price A4 + A5.

**Instrument noise.** The box is a laptop: Ryzen 9 5900HS, 16 logical CPUs, RTX 3060 Laptop GPU. Across the 12 timed rows, the B/B spread (two runs) runs from 0.08 % (soft_step) to 4.03 %, and some A/A spreads from two runs are implausibly small. For example, `stable/sleeping_on` A/A is 0.07 % at n=2 and 0.99 % at n=4. Several of the firings below rest on margins of 0.02 pp.

## §6 (A = d5782d43, B = b74f7ee8)
| row | B/A | A/A | B/B | n (A+B) | rule |
|---|---|---|---|---|---|
| full_step/1 | 0.9836 (pairings 0.971–0.997) | 2.42 % | 0.22 % | 2+2 | R1: not fired (below 1.005, inside A/A) |
| full_step/4 | 0.9797 (0.954–1.006) | 1.32 % | 4.03 % | 2+2 | no rule covers B < A; inside B/B |

row_identity_churn is B only. Each ratio divides a row by `stable` from the same run, averaged over B1 and B2 (n=2):

| row | off | on | rule |
|---|---|---|---|
| swap_churn | 1.0023 | 1.0083 | R2 (> 2 %): not fired. B1-off alone reads 1.0204; B2-off reads 0.9845 |
| archetype_shift | 0.9899 | 1.0103 | R3 (> 2 % → profile the aligned walk): not fired |
| first_archetype_spawn | 0.9939 | 1.0025 | R3 (> 2 %): not fired |
| burst_despawn | 0.9944 | 0.9983 | R3 (> 2 %): not fired |
| burst_migrate | 0.9971 | 1.0032 | R3 (> 10 % → hash table): not fired |

**Caveat.** B/B on these rows reaches 3.87 %, which is above the 2 % thresholds. At n=2, the 2 % clauses can only say "not seen". The 10 % clause is robust.

## §7 (A = d552be05, B = a56007ab)
| row | B/A | A/A | B/B | n | R1 (> 1.005 and outside A/A) |
|---|---|---|---|---|---|
| full_step/1 | 1.0025 | 1.36 % | 0.22 % | 2+2 | not fired |
| full_step/4 | 1.0110 (0.991–1.031) | 0.39 % | 3.55 % | 2+2 | **fires by 0.71 pp.** No extra runs were taken |
| stable/sleeping_off | 1.0195 (1.005–1.034) | 0.30 % | 2.49 % | 2+2 | **fires** |
| same row | 1.0082 (0.982–1.034) | 0.81 % | 4.36 % | 4+4 | **fires by 0.02 pp.** Without B2 (flagged): 1.0007, does not fire |
| stable/sleeping_on | 1.0079 | 0.07 % | 2.08 % | 2+2 | fires |
| same row | 1.0071 | 0.99 % | 2.87 % | 4+4 | not fired. This row also carries A4 |
| soft_step_sp2/coupled/64 | 0.9950 | 0.17 % | 0.08 % | 2+2 | R3: 0.33 pp outside A/A, under 2 pp, not fired |
| soft_step_sp2/coupled/256 | 0.9984 | 0.52 % | 0.24 % | 2+2 | R3: inside, not fired |

- **R1 prescribes:** inspect the **release** `physics_apply` listing for the filter-fetch initialisation. If it is there, move the selection into the data terms and re-measure. Do not revert to `Query<Mut<RigidBody>>`. The inspection is structural, not load-sensitive.
- **Context against R1.** The same two binaries show +0.25 % on `full_step/1` and +0.28 % on §8's `pyramid_sleeping_off`, both single-worker sleeping-off scenes.
- **R2** (below 0.995): no full_step or stable row is below it.
- **R4: not run.** It is a "stop" rule, so §7 cannot close.

## §8 (A = d552be05 + port, B = a56007ab)
| row | B/A | A/A | B/B | n | rule |
|---|---|---|---|---|---|
| pyramid_sleeping_off | 1.0028 (0.986–1.020) | 0.26 % | 3.10 % | 2+2 | **R1 fires by 0.02 pp → "stop"** |
| pyramid_awake_sleeping_on | 1.0244 (1.0125–1.0365) | 1.80 % | 0.54 % | 2+2 | **R2 fires.** Above 1.01, and outside A/A by 0.63 pp |
| pyramid_frozen_sleeping_on | 0.9777 (0.956–0.999) | 1.19 % | 3.21 % | 2+2 | no timing rule; B is faster |
| full_step/1 cross-check | 1.0025 | 1.36 % | 0.22 % | 2+2 | reused from §7 (same exe sha256) |

- **Frozen-arm receipts:** both trees print `froze at step 61, manifolds 8555, islands 1, contact_wakes 0`. On A that 0 comes from the stub.
  - R3 holds: the two trees are in the same state.
  - R4 holds: 8555 is not 0.
  - The stubbed helper runs only in setup (`sleeping_pipeline.rs:278`), outside `iter_custom`.
- **R1's premise does not hold on these arms.** R1 assumes a move on this row means the key loop leaked into the default path. The code shows the key loop cannot run here, and the only other default-path change is A5.
- **R2 prescribes:** hoist the `island_of` slice in the key sweep and re-measure. R2 cites `resources.rs:3490-3523`; at `a56007ab` the loop is lines 3486-3518, and the citation still lands in it.
  - The awake row prices A4 + A5. The sleeping-off rows put A5's share at about 0.3 %, so A4 accounts for roughly 2.1 %.

## §9 (A = 08fe7b9f, B = 8d656ad8)
| row | B/A | A/A | B/B | n |
|---|---|---|---|---|
| **pyramid_sleeping_off (PRIMARY)** | **1.3790** (1.340–1.419) | 2.14 % | 3.60 % | 2+2 |
| same row | **1.4122** (1.330–1.477) | 2.90 % | 7.54 % | 4+4 |
| full_step/1, A2 discarded | 1.1030 (1.065–1.134) | 4.37 % | 1.92 % | 3+4 |
| full_step/1, A2 kept (not used) | 1.0626 | 9.93 % | 1.00 % | 2+2 |
| full_step/4 | 1.0714 (1.048–1.095) | 1.56 % | 2.76 % | 2+2 |
| awake_sleeping_on (NOT like for like) | 1.2213 / 1.2082 | 4.49 / 6.07 % | 2.64 / 7.61 % | 2+2 / 4+4 |

**R2** fires when the time ratio exceeds the contact-point ratio by more than A/A. It does not fire on any row. Points are live contact points per timed step, over each run's own window:

| row | A pts | B pts | points B/A | time B/A | time ÷ points |
|---|---|---|---|---|---|
| sleeping_off | 14734.0 (286–705) | 22957.6 (158–367) | 1.558 | 1.379 / 1.412 | 0.885 / 0.906 |
| full_step/1 | 15532.7 (d=2) | 17499.4 (d=2) | 1.127 | 1.103 | 0.979 |
| full_step/4 | 14853.5 (d=3) | 17494.6 (B1 d=2, B2 d=3) | 1.178 | 1.071 (B2 alone, same window: 1.086) | 0.910 |
| awake | 13510.2 | 17167.6 | 1.271 | 1.221 / 1.208 | 0.961 / 0.951 |

- **The different timed windows on sleeping_off do not bias the result.** Probe means over both windows:

  | arm | window | manifolds | points |
  |---|---|---|---|
  | B | 158–367 | 6675.0 | 22957.6 |
  | B | 286–705 | 6672.0 | 22972.2 |
  | A | 158–367 | 5527.2 | 14981.1 |
  | A | 286–705 | 5319.0 | 14734.0 |

  B's pile is flat, so its time over A's window would be essentially the same. The measured B/A therefore equals the B/A over the common window 286–705 to within about 0.1 %.
- **The time ratio sits between the manifold ratio and the point ratio.** Manifolds: 6672 / 5319 = 1.254. Time: 1.38–1.41. Points: 1.558.
- **R3** (below 1.0): not fired.
- **The B/B spread on PRIMARY is wide at n=4 (7.54 %).** B3 and B4 read 27.25 and 26.97 ms against 26.19 and 25.26 ms. Load does not explain it: B4's during-run load was 2.79 %. The PRIMARY number therefore rests on 4+4 runs, and it is given with its pairing range.
- **Awake row:** it keeps the "not like for like" label the spec asks for. But the spec's reason (B's pile freezes, A's does not) does not apply to this bench row: with `sleep_threshold = 0`, neither arm ever latches.

---

## Drafted strike blocks

I assumed the orchestrator copies the whole scratch `mq/` tree into `docs/measurements/2026-09-18-physics-s6-s9/` with its layout unchanged: `timing_raw.md`, `runs.jsonl`, `build_report.md`, `exes.json`, `raw/`, `runlogs/`, `logs/`, `probe/`, `s8_armA_port.diff`, `s9_probe.diff`. If it copies less, the paths cited in the blocks need adjusting. The blocks also cite the build report's figures: rustc 1.98.1, the `x86-64-v3` fingerprints, the three-run R1 threshold, and the 21:05–22:01 run window.

### §6
```markdown
## ~~6. Physics — what the defect-A interim row identity costs~~

**RESULT, 2026-09-18. Struck: no rule fired.** Owner's workstation (AMD Ryzen 9 5900HS, 16 logical CPUs; RTX 3060
Laptop GPU, unused by these benches), `stable-x86_64-pc-windows-msvc` rustc 1.98.1, bench profile. Every arm's cargo
fingerprints record `target-cpu=x86-64-v3` from `.cargo/config.toml`; no `RUSTFLAGS`. Prebuilt exes were invoked
directly, each sha256-checked before its run; nothing compiled. §0 receipt: the owner declared the machine quiet at
about 19:00 and no other workflow ran. 10 runs, 21:05–21:17 +03:00. No cargo, rustc, link, lld, dxc or clippy process
existed at any receipt. The 10 s CPU average was 2.09–4.42 % before each run and 2.12–4.22 % after. Background: the
browser and Task Manager. Contaminated runs: 0.

| row | A = d5782d43 (ms) | B = b74f7ee8 (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| full_step/1 | 19.3185 / 18.8567 | 18.7951 / 18.7534 | 2.42 % | 0.22 % | 0.984 | 2+2 |
| full_step/4 | 11.0775 / 11.2249 | 11.1455 / 10.7047 | 1.32 % | 4.03 % | 0.980 | 2+2 |

row_identity_churn (B only, n = 2): each row ÷ the same run's `stable`, mean of B1 and B2.

| sleeping | swap_churn | archetype_shift | first_archetype_spawn | burst_despawn | burst_migrate |
|---|---|---|---|---|---|
| off | 1.002 | 0.990 | 0.994 | 0.994 | 0.997 |
| on | 1.008 | 1.010 | 1.003 | 0.998 | 1.003 |

- **R1 did not fire.** B/A was 0.984, not above 1.005, and inside the A/A spread. The always-on half stays; no gating on
  `has_structural_add_since`.
- **R2 did not fire.** swap_churn / stable was 1.002 (off) and 1.008 (on). Run B1-off alone read 1.020, and B2-off read
  0.985.
- **R3 did not fire.** burst_migrate / stable was at most 1.003, against a 10 % threshold, so stage 2 keeps sort +
  binary search. The aligned-walk rows were at most 1.010, against 2 %.
- **Limit:** the B/B spread on the churn rows reaches 3.87 %, above the 2 % thresholds. At n = 2, those clauses mean
  "not seen", not "absent".
- Structural receipt, identical in all 10 row_identity_churn logs (s6 and s7): stable built 0 maps; the churn rows
  resolved 4 / 5 / 2 / 124 / 2546 rows through stage 2.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md` §1 and §3; `raw/s6/`; `runlogs/s6_*`;
exe hashes in `exes.json`; ISA and no-compile proof in `build_report.md`).
```

### §7 (heading left unstruck: R4 not run, R1 follow-up pending)
```markdown
## 7. Physics — what the shared body-set selection costs (defect A5) — TIMED 2026-09-18; OPEN

**RESULT, timed part, 2026-09-18.** Same host, toolchain, ISA proof and §0 conditions as §6. 20 runs, 21:17–22:01
+03:00. The 10 s CPU average was 1.78–4.10 % before each run and 1.75–5.61 % after. Contaminated runs: 0. Flagged:
`stable` B2 (receipts 4.1 % before, 5.4 % after).

| row | A = d552be05 (ms) | B = a56007ab (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| full_step/1 | 19.0483 / 19.3088 | 19.2060 / 19.2486 | 1.36 % | 0.22 % | 1.0025 | 2+2 |
| full_step/4 | 10.7231 / 10.7650 | 11.0553 / 10.6700 | 0.39 % | 3.55 % | 1.0110 | 2+2 |
| stable/sleeping_off | 17.0243 / 16.9737 / 17.1108 / 16.9773 | 17.1154 / 17.5472 / 17.1838 / 16.7985 | 0.30 % (n=2), 0.81 % (n=4) | 2.49 %, 4.36 % | 1.0195 (n=2), 1.0082 (n=4) | 4+4 |
| stable/sleeping_on | 16.7693 / 16.7577 / 16.6816 / 16.8479 | 17.0705 / 16.7199 / 16.6283 / 17.1120 | 0.07 %, 0.99 % | 2.08 %, 2.87 % | 1.0079, 1.0071 | 4+4 |
| soft_step_sp2/coupled/64 | 0.0873 / 0.0874 | 0.0869 / 0.0869 | 0.17 % | 0.08 % | 0.9950 | 2+2 |
| soft_step_sp2/coupled/256 | 0.3835 / 0.3815 | 0.3814 / 0.3823 | 0.52 % | 0.24 % | 0.9984 | 2+2 |

- **R1 fired by its wording** on full_step/4 (+1.10 % against A/A 0.39 %) and on stable/sleeping_off (+1.95 % at
  n = 2; +0.82 % against 0.81 % at n = 4). The n = 4 firing rests on run B2 alone: without it, B/A is 1.0007. At n = 2,
  stable/sleeping_on fired too; at n = 4 it did not.
- **Against R1:** the B/B spread exceeds each excess. On the same pair of arms, full_step/1 reads +0.25 % and §8's
  `pyramid_sleeping_off` reads +0.28 %.
- **R1's prescription is still open:** inspect the release `physics_apply` listing for the filter-fetch initialisation.
  If it is present, move the selection into the data terms and re-measure. Do NOT revert to `Query<Mut<RigidBody>>`.
- **Attribution limit:** `a56007ab` also carries A4 (wake-on-contact-change). Its key loop is unreachable with sleeping
  off (`systems.rs:1085`), so the sleeping-off rows price A5 plus the rest of the diff. stable/sleeping_on prices A4 + A5.
- **R2 did not fire:** no full_step or stable row is below 0.995.
- **R3 did not fire:** coupled/64 is 0.33 pp outside its A/A spread, under the 2 pp threshold. coupled/256 is inside.
- **R4 NOT RUN.** `alloc_frame_census` S1a/S1b/S1c and `alloc_frame_attribution` are still to be run on both arms and
  compared with each other. This entry is not done until they are.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md`; `raw/s7/`; `runlogs/s7_*`; `runs.jsonl`).
```

### §8
```markdown
## ~~8. Physics — what wake-on-contact-change costs (defect A4)~~

**RESULT, 2026-09-18. Struck, with an R2 follow-up queued.** The "NO TIMING HAS BEEN TAKEN YET" line above is now
history. Same host, toolchain, ISA proof and §0 conditions as §6. 4 runs, 21:30–21:34 +03:00. The 10 s CPU average was
2.02–3.43 % before each run and 1.88–3.85 % after. Contaminated runs: 0.

Arm A is `d552be05` with `a56007ab`'s bench and `[[bench]]` entry ported, and the `contact_wakes` helper body set to
`0` (diff: `s8_armA_port.diff`). Arm B is `a56007ab`.

| row | A (ms) | B (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| pyramid_sleeping_off | 17.7852 / 17.8312 | 18.1349 / 17.5806 | 0.26 % | 3.10 % | 1.0028 | 2+2 |
| pyramid_awake_sleeping_on | 16.7251 / 17.0295 | 17.3349 / 17.2416 | 1.80 % | 0.54 % | 1.0244 | 2+2 |
| pyramid_frozen_sleeping_on | 5.6345 / 5.7020 | 5.4530 / 5.6311 | 1.19 % | 3.21 % | 0.9777 | 2+2 |
| full_step/1 (cross-check; the §7 runs, same exes) | 19.0483 / 19.3088 | 19.2060 / 19.2486 | 1.36 % | 0.22 % | 1.0025 | 2+2 |

Frozen-arm receipts, both trees: `froze at step 61, manifolds 8555, islands 1, contact_wakes 0`. Arm A's 0 is the
stub.

- **R3 holds** and **R4 holds** (8555 manifolds, not 0), so the frozen row is valid.
- **R1 fired by its wording** (+0.28 % against A/A 0.26 %; B/B 3.10 %; the pairings span 0.986–1.020). Its premise does
  not hold on these arms:
  - At `a56007ab`, `begin_step` is reachable only through `solve_colored_sleeping`, which `physics_solve_colored` calls
    only under `cfg.sleeping` (`systems.rs:1085`).
  - `a56007ab` also carries A5, which is on the default path. So a move on this row cannot be the key loop.
- **R2 fired:** +2.44 % against 1.01 and A/A 1.80 %; every pairing is at least 1.0125. Prescribed: hoist the
  `island_of` slice in the key sweep (the loop at `resources.rs:3486-3518` in `a56007ab`), then re-measure this row
  before any behavioural change. The row prices A4 + A5; the sleeping-off rows put A5's share at about 0.3 %.
- **Frozen row:** B is 2.2 % faster. That is inside B/B, and no rule applies; it is not a cost.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md`; `raw/s8/`; `runlogs/s8_*`;
`logs/*.s8receipt.out`; `s8_armA_port.diff`).
```

### §9
```markdown
## ~~9. Physics — what the face-versus-edge rule costs in a default world (defect A7b, S5)~~

**RESULT, 2026-09-18. Struck. Arm B pinned: `8d656ad8`** (only parent `08fe7b9f`). The "NO TIMING HAS BEEN TAKEN" line
above is now history. Same host, toolchain, ISA proof and §0 conditions as §6.
- 28 runs (16 required and 12 supplementary, n = 3 and 4), 21:34–21:57 +03:00.
- The 10 s CPU average was 0.60–3.42 % before each run. Two runs waited one 59 s poll first.
- During-run load not caused by the bench was 1.72–4.10 % of the machine (recorded on 20 runs).
- Contaminated runs: 0. **Discarded:** `full_step/1` A2. Its after-receipt was 16.66 % (a browser burst), and its CI
  overlaps no other A run's.

| row | A = 08fe7b9f (ms) | B = 8d656ad8 (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| **pyramid_sleeping_off (PRIMARY)** | 18.8551 / 18.4564 / 18.9987 / 18.5245 | 26.1900 / 25.2639 / 27.2549 / 26.9695 | 2.14 % (n=2), 2.90 % (n=4) | 3.60 %, 7.54 % | **1.379 (n=2), 1.412 (n=4)**; pairings 1.330–1.477 | 4+4 |
| full_step/1 | 18.6569 / 19.4885 / 18.9712 | 20.7566 / 20.9655 / 21.1604 / 21.1147 | 4.37 % | 1.92 % | 1.103 | 3+4 |
| full_step/4 | 10.8959 / 11.0667 | 11.6027 / 11.9273 | 1.56 % | 2.76 % | 1.071 | 2+2 |
| awake_sleeping_on: NOT like for like, never S5's price | 17.8255 / 17.0423 / 18.1057 / 17.0879 | 21.5734 / 21.0114 / 21.8375 / 20.2263 | 6.07 % | 7.61 % | 1.208 | 4+4 |

**R2: live contact points per timed step, each arm over its own window** (probe: `s9_probe.diff`; series in
`probe/*.csv`).

| row | A pts | B pts | points B/A | time B/A |
|---|---|---|---|---|
| sleeping_off | 14734.0 (steps 286–705) | 22957.6 (steps 158–367) | 1.558 | 1.379 / 1.412 |
| full_step/1 | 15532.7 | 17499.4 | 1.127 | 1.103 |
| full_step/4 | 14853.5 | 17494.6 | 1.178 | 1.071 |
| awake | 13510.2 | 17167.6 | 1.271 | 1.208 |

- **R1:** in a default world, S5 costs **1.38× to 1.41×** per step on the resting height-15 pile over the bench's
  timed steps.
- **R2 did not fire.** Every time ratio is below its point ratio (time ÷ points 0.89–0.98), so no profiling of the
  realized-patch check or the fallback is prescribed.
  - The time ratio sits between the manifold ratio (1.254) and the point ratio (1.558).
  - The two arms timed different windows. This does not bias the result: B's pile is flat across both windows
    (6675 / 6672 manifolds, 22 958 / 22 972 points), so the B/A over the common window 286–705 is the same to about
    0.1 %.
- **R3 did not fire** (B/A is above 1.0).
- **Awake row:** it runs with `sleep_threshold = 0`, so neither arm latches in it. The freeze asymmetry described
  above is not what this row measures.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md` §1 and the R2 table; `raw/s9/`;
`runlogs/s9_*`; `probe/` with `r2_window.py` and `r2_table.txt`; `s9_probe.diff`).
```

---

**Files** (scratch, to be moved into the repository):
- `docs/measurements/2026-09-18-physics-s6-s9/timing_raw.md`
- `.../mq/runs.jsonl`
- `.../mq/raw/`
- `.../mq/probe/`
- `.../mq/build_report.md`

My recompute script: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/a5f69b98-b880-43f1-a079-e964dc80e637/scratchpad/analyst/recompute.py`.

Spec: `D:/wt/joltab/docs/MEASUREMENT-QUEUE.md` (no edits made).