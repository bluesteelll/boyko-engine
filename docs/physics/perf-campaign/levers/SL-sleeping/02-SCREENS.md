# Lever SL: the screens S-SL1, S-SL2, S-SL3 (untimed)

Run 2026-10-12 by the lane's tester, on `u/phys-sl` at `493be29a`. The lane tree was clean before and
after; nothing was committed by the run.

**Verdict: PASS on all three.**
- **S-SL1 (blocking):** every pre-registered bar (i)-(ix) holds for both L and L+b, and 0 runs are
  SUB-ISLAND.
- **S-SL2:** 0 events in 30 draws and in 56 draws; every draw freezes at step 61.
- **S-SL3:** all four rows pass. (ii-dyn) blocks only on the structural prefix, as written before the run.
- **Bar (iii):** passes (S-SL2, G2 and G7).

Nothing was relaxed or re-defined after a run, and no timing was read.

Where the data are:
- The session report: `D:/tmp/phys-orch/sl/screens.md` (sha256
  `9f1eb651d69f0f6268f2c59384f1bf12299df2d0f8fce6a7420e027d71ea45f6`).
- Raw runs, scripts and logs: `D:/tmp/phys-orch/sl/screens/` (about 93 MB), with the manifest
  `SHA256SUMS.screens`.
- The two pre-registrations are quoted in full in Appendix A and Appendix B.

The definitions of the screens are design rev 2 §10.1-§10.3 (`01-DESIGN.md` Part B) and the cut's §3.4,
§3.0 (route 2's exit) and Addendum 1 W1 (the structural prefix).

## 1. Provenance

| item | value |
|---|---|
| arm P (parent = SL-2a, C1) | `f1f733f6` + `patch/perturb.diff`; `bin/runner_P_f1f733f6_pert.exe`, sha256 `d6bc8e01c4c20c3f8c6a13eeeb78d6b86a22d854f66dc4d870d06c3703179253` |
| arm L (SL-1, C2) | `57d6fe62` + perturb.diff; `bin/runner_L_57d6fe62_pert.exe`, `e6e9469d7201033b4ae6f3d283a363deebbaccdf9ea68f5e4fdfd0f671ee82ec` |
| arm L+b (SL-1b, the final head) | `493be29a` + perturb.diff; `bin/runner_Lb_493be29a_pert.exe`, `0cb47ae9ac0256df1524bf4241ce6f0dd2e689a82e3db39408077d4f327562e9` |
| arm D (the seal discarded) | L+b's export + `let sealed = self.take_seal(bodies, arm) && false;` in `begin_step_armed`; `bin/runner_D_493be29a_pert.exe`, `bd833e14561c175339c362b028b812c45cea02ae67262d801d16d6909a0d31c3`. The accounting still runs, so D's `seal_held` counts checks that held but were not used |
| the perturbation (ruling 39 Q3 = no) | `patch/perturb.diff`, sha256 `7f38218ea270fcf355179c7a72ad134c49ace9769b88585755e19d7981cdb236`: a port of `cs_proto.diff:632-664`. Env `BOYKO_F0_PERTURB="<box>:<axis>:<+-1>"`; the f32 bits of the spawn component step by dir; inert when unset. Scratch only, never committed |
| S-SL3 arms (unperturbed lane exes) | P `bin/runner_C1_f1f733f6.exe` (`235ed5b5…`); L `bin/runner_C2_57d6fe62.exe` (`7b13dfd4…`); L+b `tc/bin/runner_head_493be29a.exe` (`003f33e62d28cba49c27436d2cac13c68c643b1139557c08a6935d488759ad42`); control `bin/runner_C3_3bbd9334.exe` (`aaaf9347…`) |
| pre-registrations, pinned before the runs | `screens/s-sl1/PREREG.md` `b2adfe478e9948481dcefa9387ae0c8ea9647d8a3e5c64ad16bfcb417a30627b`; `screens/s-sl3/PREREG.md` (S-SL2 + S-SL3, with amendment A1) `e8d20b9f69bafca1ef1a12eb5628b33710dcc0af69cb315e562510435c97d085` (first pin `8a811b6b…`) |
| the analysis script | `sl_screens.py` `86c4808b6ce2e4fbebf207268a740f74067d1e82811ec127dd4ff459f24322e0`. Every run's `job.json` carries `prereg_sha256`; the driver refuses a changed PREREG or a changed analysis script |

Paths are relative to `D:/tmp/phys-orch/sl/screens/` (and `D:/tmp/phys-orch/sl/` for `bin/` and `tc/`).

What the reader needs to know:
1. **L+b is built at `493be29a`, not at the cut's `3bbd9334` (`runner_C3`).**
   - Between them the non-test source changed only in comments.
   - The unperturbed C3 exe and the unperturbed head exe have byte-identical `.text`, `.data`, `.pdata`
     and `.reloc` sections (62 differing bytes: 59 in `.rdata`, 3 in the PE header).
   - Both were also run as controls. They match the perturbed L+b build on the `none` rows, and C3
     equals the head byte for byte on every S-SL3 row.
2. **The first build pass was wrong and was discarded.**
   - The L and L+b builds reported `Finished in 0.2s`: the exports' sources had older mtimes than P's
     dep-info. So the script linked P's exe three times, under one sha256 `d6bc8e01…`.
   - Pass 2 touched every `.rs` of `crates/boyko_physics`, required a `Compiling boyko-physics` line, and
     refused an exe whose hash equals an earlier arm's.
   - The four arm exes are distinct and behave as their commits. A verification `none` row reads: P
     `0x130c76cb6b463ab8` with C1's keys; L and L+b `0xc637a0517adabab7`; first frozen 147.
   - Arm D differs from L+b's export in exactly the one patched line.
3. **A code-path dry run** of the reader (3 seeds, scratch root, deleted) found one transcription error in
   the S-SL3 PREREG: "`frozen_v_max == 0` on L+b (and on L)".
   - The cut binds the bar to L+b. L has no write test, so a kick writes a velocity into a frozen row and L
     reads > 0 on S-KICK.
   - Amendment A1 was recorded before any real S-SL3 run, and the file was re-pinned. It brings the PREREG
     to the cut's text; it is not a relaxation.
   - The S-SL1 PREREG was not amended.
4. The scratch exports and the scratch target were deleted after the runs.

## 2. S-SL1 (BLOCKING): the J-T sleeping arm

Four arms × (16 one-ulp seeds + `none`) × contact reuse {on, off}.

**Rows**, exactly:

    --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping on --contact-reuse on|off --workers 1
    --steps 500 --arm-profiler --window 0..500 --csv .. --pose-out ..

with `BOYKO_F0_PERTURB` set for the seeded rows.

**The runs:** 136 W1 runs, 8 W8 replays (`b581zm` reuse off and `none` reuse on, per arm), and 8
unperturbed-exe controls: 152 runner processes. Plus 121 C3b motion-dump runs (P 66, D 55; L and L+b
have no re-wake to dump).

**Validity:** every run exits 0 and is valid. The checks: the config; `void_steps` 0; `drops_total` 0;
500 rows and 1240 poses; SUMMARY `pose_hash` equals the FNV-1a of `pose.bin`; the stderr perturbation
receipt names exactly the seed's box, axis and dir, with the after bits equal to the before bits + dir.
Invalid runs: 0.

**Validity of the instrument (before any bar):**
- **P against the CS prototype's V2 sweep** (`cs/proto/sleep_sweep/sweep.json`): the pose hash, first
  frozen step, wakes, rise steps and W_sleep are equal on 34 of 34 rows. So the perturbation port, the
  seeds and the W_sleep reader are the CS ones.
- **The patch is inert when unset.** The `none` rows of the perturbed builds equal the unperturbed lane
  exes (P = C1, L = C2, L+b = C3, L+b = HEAD, both reuse arms) on pose bytes, the untimed CSV columns and
  the `sleep` object: 8 of 8.
- **No perturbation was absorbed:** the final pose hashes are distinct in every arm and reuse cell, 17 of
  17.
- **P's re-wakes are exactly the predicted six,** all with reuse off: b38xp [146], b581zm [146],
  b658xp [150], b813xm [148], b968xp [145], b1046zp [147]. None with reuse on. This matches the CS sweep.

**The bars** (the threshold is the pre-registered text; both X = L and X = L+b were evaluated):

| bar | pre-registered threshold | L | L+b | verdict |
|---|---|---|---|---|
| (i) | 0 visible re-wakes (C3b bands: visible = max corner displacement ≥ 16 mm over [r-1, r+59]) | 0 re-wakes in 34 runs: **vacuously true** | same | PASS (vacuous, stated) |
| (ii) | per seed W_sleep(X) ≤ W_sleep(P), both reuse arms (34 pairs); mean ratio reuse on ≤ 1.00 | 0 violations; 28 pairs equal, 6 strictly less; mean ratio reuse on 1.0000 (17- and 16-seed); reuse off 0.8895 (reported) | identical | PASS |
| (iii) | G2/G7 re-expressed events = 0; generator 0/30 and 0/56 counting re-keys | section 3 | section 3 | PASS |
| (iv) | holds 34/34 | 34/34; max drift 106.6 mm; 0 boxes > 0.5 m | same | PASS |
| (v) | `seal_breaks == 0 && seal_held >= 1 && settles - seal_held in {0,1}` on every run | 34/34: settles 1, held 1, breaks 0 (difference 0) | same | PASS |
| (vi) | `frozen_v_max == 0` | 34/34 read 0 (P: 34/34 non-zero, max 4.80e-6) | same | PASS |
| (vii) | where P does not rise, the positions-and-rotations hash equals P's | 28 of 28 equal | 28 of 28 | PASS |
| (viii) | first frozen step == P's first attempt (P's first frozen if no rise, else P's first rise) | 34 of 34 equal | 34 of 34 | PASS |
| (ix) | L and L+b byte-identical on all 34 runs (pose, untimed CSV, every L `sleep` counter), `write_wakes == 0` on L+b | 34 of 34 identical; `write_wakes` 0, `write_wake_rises` [] | | PASS |
| W8 replays | pose bytes, `sleep` object and core CSV columns equal to W1; exit 0 | 8 of 8 (P, L, L+b, D × 2 rows) | | PASS |
| route 2 | SUB-ISLAND runs, expected 0 | 0 | 0 | none to route |

**Per seed** (the first frozen step; `rk` = `settle_rekeys`; the full table is
`screens/s-sl1/RESULTS.md`):

| seed | P on | P off (rises) | L = L+b on | L = L+b off | rk off | D off (rises) |
|---|---|---|---|---|---|---|
| none | 147 | 142 | 147 | 142 | 0 | 142 () |
| b38xp | 144 | 206 [146] | 144 | 146 | 0 | 146 () |
| b116zp | 146 | 143 | 146 | 143 | 0 | 143 () |
| b193xm | 144 | 144 | 144 | 144 | 0 | 144 () |
| b271zm | 145 | 144 | 145 | 144 | 0 | 144 () |
| b348xp | 148 | 143 | 148 | 143 | 0 | 143 () |
| b426zp | 147 | 144 | 147 | 144 | 0 | 144 () |
| b503xm | 148 | 143 | 148 | 143 | 0 | 143 () |
| b581zm | 144 | 206 [146] | 144 | 146 | 1240 | 206 [146] |
| b658xp | 144 | 210 [150] | 144 | 150 | 1240 | 210 [150] |
| b736zp | 144 | 146 | 144 | 146 | 0 | 146 () |
| b813xm | 144 | 208 [148] | 144 | 148 | 1240 | 208 [148] |
| b891zm | 148 | 144 | 148 | 144 | 0 | 144 () |
| b968xp | 144 | 205 [145] | 144 | 145 | 1240 | 205 [145] |
| b1046zp | 144 | 207 [147] | 144 | 147 | 1240 | 207 [147] |
| b1123xm | 144 | 152 | 144 | 152 | 0 | 152 () |
| b1201zm | 145 | 144 | 145 | 144 | 0 | 144 () |

**Against the predictions recorded in the PREREG,** all of these are confirmed:
- L and L+b freeze at P's first attempt on 34/34, with 0 contact wakes and 0 breaks.
- `settles == seal_held == 1` everywhere.
- `settle_rekeys` is 1240 on exactly the five risers other than b38xp, and 0 elsewhere.
- W_sleep is equal on the 28 non-rising runs, and 30.8-31.6 % lower on the six rising ones (L/P
  0.6836-0.6918; 1.855-1.865 M solved slots saved each).
- L and L+b are byte-identical.

**One recorded prediction was falsified, on arm D only.** The design expected `rises(D) ≥ rises(P)`, paired
by seed.
- D re-wakes on the same five seeds as P (all but b38xp). b38xp does not re-wake on D (P 1 rise, D 0): the
  zero alone removes that flicker.
- D never rises where P does not (0 of 28).
- Totals: P 6 re-waking runs, D 5, L and L+b 0.
- So the unsealed settle (C1's raw size, which the seal exists for) re-creates P's flicker on 5 of 34
  trajectories and creates no new one. The seal turns those 5 into first-attempt freezes (1240 re-keys
  each).
- D's mean W ratio against P: 0.9815 with reuse off, 1.0000 with reuse on. On its 5 re-waking runs, D/P
  is 0.99997-1.00004.

**Motion** (the C3b bands, for context; only P and D have re-wakes):
- P's 6 intervals are all `sub-pixel`: max corner displacement 1.12-1.90 mm, sampled corner speed
  3.07-3.78 mm/s.
- D's 5 are `sub-pixel`: 1.22-1.62 mm, 2.59-4.81 mm/s.
- None is `visible` and none is `micro`. The classifier can separate them: `classify(20 mm, 3 mm/s)` is
  `visible`.

**Red controls: the bars can fail** (`screens/s-sl1/red_controls.txt`, script `red_controls.py`, sha256
`6789dee0cb7d1984e86d0a8407a8ec5c8428238987830af80653423257baeed3`):

| bar | fed with | reads |
|---|---|---|
| (viii) | arm D (the seal discarded) | 5 of 34 runs violate it: b581zm, b658xp, b813xm, b968xp, b1046zp, reuse off |
| (ii) | the roles of X and P swapped | 6 of 34 pairs violate it |
| (vi) | arm P (no zero at sleep) | 34 of 34 read `frozen_v_max != 0` |
| (ix) | L against P | 34 of 34 differ |
| (vii) | another seed's P hash | 28 of 28 violate it. With the full pose hash in place of the positions-and-rotations record, 28 of 28 differ (the velocity bytes: why the masker exists) |
| (v) | arm P (no counters) | 34 of 34 violate it |
| (iv) | the scorer on a pose with one box moved 0.6 m | `hold=False` |

Bar (i) is vacuous on L and L+b, which have no re-wake to classify. Its classifier is exercised on P's and
D's re-wakes only.

**Verdict S-SL1: PASS,** by the rule fixed in the PREREG.
- For both L and L+b, bars (i), (ii), (iv), (v), (vi), (vii), (viii) and (ix) hold.
- The SUB-ISLAND count is 0, the W8 replays are equal, and every run is valid.
- Bar (iii) is read in section 3.

## 3. S-SL2 and bar (iii)

Pre-registered thresholds: "h4: 0 events in 30 draws, freeze at step 61. h5: 0 events in 56 draws, freeze
at step 61." Events are the steps on which `contact_wakes` or `settle_rekeys` rose (the test's
`attempt_event`).

| arm | command / tree | result |
|---|---|---|
| L+b | `cargo test --release -p boyko-physics --test sleep_settles_box_piles --locked -- --ignored --exact flicker_redraw_distribution --nocapture`, in `D:/wt/sl` at `493be29a` (clean; the release test binary was newer than every source) | `running 1 test`, 1 passed. **h4: 30 draws, 30 froze, 0 events, freeze steps {61: 30}. h5: 56 draws, 56 froze, 0 events, {61: 56}.** Σ K 0 |
| D (reported beside) | the same test in the export carrying arm D's patch (`Compiling boyko-physics` present) | `running 1 test`, 1 passed. Identical: h4 0/30 {61: 30}; h5 0/56 {61: 56} |

Bar (iii) also reads G2 and G7 on L+b, in release:
- The command: `cargo test --release ... -- --exact a_height_5_box_pyramid_freezes_within_the_onset_flicker_budget sixteen_height_4_draws_freeze_within_the_onset_flicker_budget --nocapture`.
- `running 2 tests`, 2 passed.
- G2: "froze at step 61 (contact_wakes 0, 0 rise events, 0 attempt events)".
- G7: all sixteen draws freeze at 61 with 0 events; Σ events 0.

**Bar (iii): PASS.**

The honest scope of S-SL2: arm D reads the same 0/30 and 0/56, so this scene cannot see the seal. It is a
regression screen for the settle (no new attempt event), not a discriminator for the seal. The
discriminator is S-SL1's arm D.

## 4. S-SL3: dynamic scenes, sleeping on

The arms: P = C1, L = C2, L+b = the head. Per arm and row: a W1 armed run, a W8 armed replay, and a W1
`--sanity` run.

The pre-registered thresholds (`screens/s-sl3/PREREG.md`, Appendix B):
- `dyn_sanity.py` B1-B3: no failure, and `--final` exits 0, on every arm.
- (ii-dyn): blocking only on the structural prefix, steps 1..s*-1. `s*` = the minimum of: the program's
  first external event (KICK 201, SHOOT 201); the first counted wake in either arm; the first awake rise
  in either CSV on LAND and SLIDE. X's sum of solved slots must be ≤ P's there.
- `frozen_v_max == 0` on L+b.
- `write_wake_rises` is a subset of {201 + 25k}.
- W8 identity, and the C3 exe equals the head exe.
- The per-kick-event split is reported.

| row | s* (from) | prefix sum P / L / L+b (blocking) | whole run P / L / L+b (reported) | frozen_v_max P / L / L+b | settles / write wakes / contact wakes (L+b) | first CSV divergence, P vs L+b | verdict |
|---|---|---|---|---|---|---|---|
| S-LAND (100 steps) | infinite (no wake, no event; the first freeze would be 146) | 2,650,755 / = / = | 2,650,755 / = / = | 0 / 0 / 0 | 0 / 0 / 0 | none | PASS |
| S-KICK | 201 (the program's event; the first counted wake is also 201) | 4,073,080 / = / = | 20,083,479 / 20,055,196 / 20,056,407 (−0.135 %) | 21.43 / **21.43** / 0 | 14 / 65 rows (4 steps) / 1182 rows (5 steps) | 201 | PASS |
| S-SHOOT | 201 (the program's event; the first counted wake 260) | 4,082,680 / = / = | 20,668,039 / 20,636,181 / 20,636,181 (−0.154 %) | 3.2e-7 / 0 / 0 | 8 / 0 / 1242 rows (260, 648, 781) | 261 | PASS |
| S-SLIDE | infinite (freezes at 379; no wake) | 9,486,233 / = / = | 9,486,233 / = / = | 1.0e-7 / 0 / 0 | 1 / 0 / 0 | none | PASS |

- **Exit codes.** LAND and SLIDE exit 0 on all arms.
  - **KICK and SHOOT exit 3 (void) on all arms, the parent P included** (F1 below).
  - `drops_total` is 0.
  - They are accepted by the pre-registered rule: every printed `VOID` line names `phys_bp_queried`, and
    `phys_bp_queried + phys_bp_members` is conserved on every void step of every arm.
  - The void steps: KICK P [455, 542, 666, 692], L and L+b [454, 543]; SHOOT P [258], L and L+b
    [258, 633, 771].
  - The `--sanity` runs (no profiler) exit 0 on all 12.
- **`dyn_sanity.py`:** 0 B1-B3 failures on the CSV, and `--final` exits 0, on all 12 (arm × row). No
  B4-B6 failure and no premise failure either. The premises: S-KICK's 24 events inject energy; S-SHOOT
  adds 60 bodies; S-SLIDE's centroid moves ≥ 5 m downhill; S-LAND holds 1240 throughout.
- **W8 replays:** pose bytes, the `sleep` object and the core CSV columns equal W1's on 12 of 12. The C3
  exe equals the head exe byte for byte (pose, untimed CSV, `sleep` object) on 4 of 4 rows.
- **The S-KICK event split (the O1 claim), on L+b:**
  - 24 kick events, rising at steps 201 + 25k.
  - Write wakes on 4 steps {201, 626, 726, 751}, 65 rows in total. All 4 are inside {kick + 1}, as
    required.
  - Event 201 is both a write and a contact wake. Events 626, 726 and 751 are write only.
  - **20 of the 24 events produce neither:** the kicked boxes were already awake after the first event,
    so nothing was asleep to wake.
  - Contact wakes away from the event steps: 458, 556, 707, 714 (islands that fell asleep after 201 and
    woke by contact). P's analogues: 459, 557, 646, 715, 743.
  - L (no write test) has 0 write wakes and reads event 201 as contact only. P also reads 201 as contact
    only.
- **S-SHOOT:** `seal_breaks` 1 (the projectile's spawn step), `seal_held` 6, `settle_rekeys` 0. This is the
  design's expected spawn break; S-SL1's bar (v) does not apply to dyn rows.
- **Beyond `s*`, where (ii-dyn) is reported, L+b ≤ P everywhere:** −27,072 slots on S-KICK (−0.135 % of
  the run) and −31,858 on S-SHOOT (−0.154 %); equal on LAND and SLIDE. There is no excess to price.

The S-SL3 pose hashes, W1 (each equal at W8 and in the `--sanity` run):

| row | P (C1 `f1f733f6`) | L (C2 `57d6fe62`) | L+b (head `493be29a`; the C3 exe equal) |
|---|---|---|---|
| S-LAND, 100 steps | `0x8e1cd101ae19a264` | `0x8e1cd101ae19a264` | `0x8e1cd101ae19a264` |
| S-KICK, 800 steps | `0x1dcb064143de7501` | `0x30052a6805b1a209` | `0xca3161f209b56e41` |
| S-SHOOT, 800 steps | `0x7a20a407e3eebdc3` | `0x0a3bbe0a97f62147` | `0x0a3bbe0a97f62147` |
| S-SLIDE, 500 steps | `0x54319f083eea058e` | `0x39656286b0c2c3cc` | `0x39656286b0c2c3cc` |

**Verdict S-SL3: PASS** (all four rows, every blocking item).

## 5. What a red would have been

- **S-SL1:** the red controls of section 2. Arm D is a real engine mutation (the seal discarded), and bar
  (viii) reds on 5 of its runs. Arm P is the engine without the zero, and bar (vi) reds on all 34 of its
  runs.
- **S-SL3, `frozen_v_max`:** L's value on S-KICK (21.43) is the mutation "no write test", and it reads
  non-zero. So the L+b bar cannot be met by an engine without SL-1b.
- **S-SL3, (ii-dyn):** its prefix cannot go red by construction, because the arms are bit-identical before
  the first event. It is the structural bar the W1 addendum makes it. The reported part beyond `s*` is
  where the comparison has content.

## 6. Findings (no scope growth; each is a PC line or a note in `04-LANE-RECORD.md`)

- **F1 → PC-SL-17 (the runner; pre-existing since SL-2a).**
  - An armed (`--arm-profiler`) `--dyn kick|shoot --sleeping on` row exits 3 on C1, C2 and the head.
  - KICK reads `VOID ... counter phys_bp_queried recorded (1, 1239), the step has (1, 1240)`, at step 454
    or 455. SHOOT reads `(1, 6)` against `(1, 1246)` at step 258.
  - The runner's anti-vacuity recount expects `rows - members` rows queried, and a sleeping row is
    neither. The data are still written, and `phys_bp_queried + phys_bp_members` is conserved on every
    void step.
  - So every armed sleeping-on dyn row is "void" for any gate that reads the exit code. S-SL3 had to
    accept it by a rule written before the run.
  - SL-3's flip and the 9c rows generator should know. The fix belongs in the runner's recount, not in
    the engine.
- **F2 (a design prediction):** arm D re-wakes on 5 of P's 6 flickering trajectories and on none of the
  other 28. The zero alone removes b38xp's flicker. The design predicted `rises(D) ≥ rises(P)`; the
  reading is D 5, P 6. The seal's measured job is the 5 first-attempt freezes (1240 re-keys each).
- **F3:** SL-1 alone (C2) is not safe on S-KICK. L's `frozen_v_max` is 21.43 (a kick writes into a frozen
  row); L+b's is 0. The rungs ship together, and the record states the pair, not L.
- **F4 → PC-SL-22:** S-KICK's write wakes fire on 4 of its 24 events (65 rows); 20 events hit an
  already-awake pile. So the write-wake claim of design §3.5 (O1) is exercised by 4 events, plus the unit
  and integration tests. A window that wants the write path under load needs a kick schedule that lets
  the pile fall asleep between events.
- **F5 (process):** exports built into one shared target share fingerprints across trees. An export with
  older mtimes is "Finished in 0.2s" and silently reuses the previous arm's exe. Hash-compare every arm
  exe against the earlier ones, and require a compile line.
- **Bar (i) was vacuous on the shipped arms** (0 re-wakes in 34 runs) → PC-SL-23. If bar (i) is to carry
  weight, it needs a seed set that makes L re-wake: that is a new screen, not this one.

## 7. Reproduction

- **Arms and patches:** `screens/patch/{perturb.diff,make_perturb.py,make_d.py,d.diff.txt}`. The builds are
  `screens/build_arms2.body` (pass 2); pass 1's `build_arms.body` was discarded as stale.
- **Runs:** `python sl_screens.py s1-run | s1-motion | s1-report | s3-run | s3-report | s2-report <log-LB> <log-D>`,
  from `screens/` (numpy needed).
  - The bodies `run_screens.body`, `s2_lb.body` and `s2_d.body` run under `screens/heavy.sh`.
  - The kit run dirs are `D:/tmp/phys-orch/sl/k/screens_{build,build2,runs,s2}`.
  - Their `VERDICT GREEN exit_code 0` is the lock wrapper's, not the screens' verdict.
- **The verdicts:** `s-sl1/results.json` `verdict_s_sl1`, `s-sl3/results.json` `verdict_s_sl3`,
  `s-sl2/results.json` `bars`.
- **Reports:** `screens/s-sl1/RESULTS.md` (with `results.json`, `red_controls.txt` and the per-run table),
  `screens/s-sl3/RESULTS.md` (with `results.json`), `screens/s-sl2/results.json`, and the logs in
  `screens/logs/`.

Nothing was vacuous:
- The `running N` lines: S-SL2 `running 1 test` on both arms; G2 + G7 `running 2 tests`; 0 skips; no
  `running 0 tests`.
- The S-SL1 bars count what they claim:
  - (ii): 34 pairs, of which 6 strict;
  - (vii): 28 runs checked (17 with reuse on and 11 with reuse off, as predicted);
  - (viii), (v) and (iv): 34 each.
- Bar (i) is vacuous on L and L+b and is labelled so.
- The instrument was validated against an independent reference (the CS V2 sweep, 34/34 including the
  W_sleep integers) and against the unperturbed exes (8/8).

## Appendix A: the S-SL1 pre-registration, verbatim

`D:/tmp/phys-orch/sl/screens/s-sl1/PREREG.md`, sha256
`b2adfe478e9948481dcefa9387ae0c8ea9647d8a3e5c64ad16bfcb417a30627b`, pinned before the first S-SL1 run.
Every run's `job.json` carries that sha256. The text below is quoted line for line.

> # S-SL1 PRE-REGISTRATION (lane SL, J-T sleeping arm, blocking for SL-1)
>
> Written 2026-10-12 by the lane's tester, BEFORE the first S-SL1 run. Its sha256 is in `PREREG.md.sha256` and in every
> run's `job.json` (`prereg_sha256`). The runner refuses to start if the file does not match the sidecar.
> Sources of the definitions: `D:/tmp/phys-orch/sl/cut.md` section 3.4 and section 3.0 (route 2 exit), `design_r2.md`
> section 10.1 (bars (i)-(viii), routes), `cut.md` Addendum 1 W1 (S-SL3 only), `cs/proto/sleep_sweep/PREREG.md` (seeds,
> sha256 `96fdfd28e2c15a4440881b7049bfed12238d523a6a5dc2513dc9bb30cc13b4aa`) and `PREREG-C3b.md` (W_sleep, motion bands, sha256
> `1e45a63edb5239398d4589cf87bb6314bd206615b56b617cf23b32076ee5a3ee`). UNTIMED: no wall-time field is read or recorded; the
> reader takes from SUMMARY only the keys `SUMMARY_KEYS` of `sl_screens.py` (pose_hash, config, void_steps, drops_total,
> first_frozen_step, steps, workers, scene, gap, bodies, sleep, dyn, pose_bytes, final_manifolds, final_pairs) and from the armed
> CSV only count columns (by header name). The runner still writes its `*_ns` columns; nothing here opens them.
>
> ## What happened before this file was written (stated, not hidden)
>
> - The brief names `D:/tmp/phys-orch/sl/screens/` (cut section 3.4 says `screen/`): the brief's directory is used.
> - Q3 = no (ruling 39): the runner has no `--perturb`. Every arm is therefore a scratch export of its commit carrying the shared
>   perturbation patch `patch/perturb.diff` (sha256 `7f38218ea270fcf355179c7a72ad134c49ace9769b88585755e19d7981cdb236`; a port of
>   `cs_proto.diff:632-664`, env `BOYKO_F0_PERTURB="<box>:<axis>:<+1|-1>"`, the f32 bits of the spawn component step by dir; the hook
>   is inert when the variable is unset and prints a `F0_PERTURB` receipt on stderr when set). Nothing was committed.
> - Exports (`git archive`, CRLF as the lane checkout) at `D:/wt/_scratch/sl-screens/{C1,C2,HEAD}`; one shared scratch target
>   `D:/wt/_targets/sl-screens` (deleted at the end of the screens). Build prefix of the brief, `--profile parity`, `--locked`,
>   `CARGO_BUILD_JOBS=2`, under the machine-wide heavy lock, through the kit.
> - **The first build pass was wrong and was discarded:** the L and L+b builds reported `Finished in 0.2s` (the exports' sources had
>   older mtimes than P's dep-info: the mtime-guard trap) and linked P's exe three times (identical sha256 `d6bc8e01...`). Those copies
>   were deleted. Pass 2 touches every `.rs` of `crates/boyko_physics` in the export first, requires a `Compiling boyko-physics` line
>   in the log and refuses an exe whose sha256 equals an earlier arm's. The four arm exes are now distinct, and a `none` row of each
>   (a verification probe, outside this screen's data, kept under `screens/probe/`) read: P first frozen 147, pose
>   `0x130c76cb6b463ab8`, C1 `sleep` keys; L and L+b `0xc637a0517adabab7`, C2 / C3 keys; D the C3 keys.
>
> ## Arms (exes pinned by sha256 in `bin/SHA256SUMS`)
>
> | arm | source | exe |
> |---|---|---|
> | P | `f1f733f6` (C1 = SL-2a, the cut's parent) + perturb.diff | `bin/runner_P_f1f733f6_pert.exe` `d6bc8e01c4c20c3f8c6a13eeeb78d6b86a22d854f66dc4d870d06c3703179253` |
> | L | `57d6fe62` (C2 = SL-1) + perturb.diff | `bin/runner_L_57d6fe62_pert.exe` `e6e9469d7201033b4ae6f3d283a363deebbaccdf9ea68f5e4fdfd0f671ee82ec` |
> | L+b | `493be29a` (final head; C3 `3bbd9334` plus comment-only source edits and test edits) + perturb.diff | `bin/runner_Lb_493be29a_pert.exe` `0cb47ae9ac0256df1524bf4241ce6f0dd2e689a82e3db39408077d4f327562e9` |
> | D | L+b's export + `patch/d.diff.txt`: `let sealed = self.take_seal(bodies, arm) && false;` (the seal's verdict is discarded, the accounting still runs, so D's `seal_held` counts checks that held but were not used) | `bin/runner_D_493be29a_pert.exe` `bd833e14561c175339c362b028b812c45cea02ae67262d801d16d6909a0d31c3` |
>
> Why L+b is built at `493be29a` and not at `3bbd9334` (the cut's `runner_C3`): the final head is what merges. Between them the
> non-test source changed only in comments (`resources.rs` two doc blocks, `sleep_sets.rs` one doc block: `git diff 3bbd9334 493be29a`).
> Evidence: the unperturbed `runner_C3_3bbd9334.exe` and the unperturbed head exe (`tc/bin/runner_head_493be29a.exe`, sha256
> `003f33e62d28cba49c27436d2cac13c68c643b1139557c08a6935d488759ad42`) have byte-identical `.text`, `.data`, `.pdata`, `.reloc`
> (`t3/cmp_pe.py`: 62 differing bytes, 59 in `.rdata`, 3 in the PE header). Controls below run both against the perturbed L+b build.
>
> ## Rows
>
> All runs W1 unless stated, one process at a time, argv exactly (per arm and cell):
>
>     --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping on --contact-reuse <on|off> --workers 1 --steps 500
>     --arm-profiler --window 0..500 --csv <armed.csv> --pose-out <pose.bin>        env: BOYKO_F0_PERTURB=<seed> (absent for `none`)
>
> - Seeds (`cs/proto/sleep_sweep/PREREG.md` lines 29-54; recomputed by the rule `b_n = floor((2n+1)*1240/32)`, axis x for even n else z,
>   dir +1 if floor(n/2) even else -1): none, b38xp `38:x:1`, b116zp `116:z:1`, b193xm `193:x:-1`, b271zm `271:z:-1`, b348xp `348:x:1`,
>   b426zp `426:z:1`, b503xm `503:x:-1`, b581zm `581:z:-1`, b658xp `658:x:1`, b736zp `736:z:1`, b813xm `813:x:-1`, b891zm `891:z:-1`,
>   b968xp `968:x:1`, b1046zp `1046:z:1`, b1123xm `1123:x:-1`, b1201zm `1201:z:-1`.
> - 17 trajectories x reuse {on, off} = 34 runs per arm; 4 arms = 136 runs.
> - W8 replays per arm (identity spot check, `--workers 8`): `b581zm` reuse off (one of P's six reuse-off risers; the cut asks for one per
>   arm) and `none` reuse on (extra). Compared with the arm's own W1 run on: pose.bin bytes, the `sleep` object, and the core CSV
>   columns (`CORE_COLS` of `sl_screens.py`: step, manifolds, pairs, top_y, awake, void, phys_slots_wide/narrow, phys_np_pairs/manifolds/points,
>   phys_bp_pairs, phys_np_reused, phys_np_sep_hits, phys_np_full, phys_sleep_held, the five phys_hist_slots_*).
> - Controls (the perturbation patch is inert when the variable is unset): the `none` rows (both reuse arms) of the unperturbed
>   lane exes C1, C2, C3, HEAD, compared with P, L, L+b, L+b respectively on pose.bin bytes, untimed CSV columns and the `sleep` object.
>
> ## Readings per run
>
> - Conventions: rise and first-frozen steps are `step + 1` (1-based, the runner's). `first_frozen` = SUMMARY `first_frozen_step`.
>   `rises` = `sleep.contact_wake_rises`; `wakes` = `sleep.contact_wakes`.
> - **W_sleep** = sum over CSV rows 0..499 of `36 * (phys_hist_slots_lt32 + _lt64 + _lt128 + _lt256 + _ge256)`, the 36 being
>   `3 * substeps * (1 + relax_iterations)` read from the run's SUMMARY config (4 * 3 = 12 sweeps) and asserted equal to 36. The
>   cross-check `slots_t == phys_slots_wide + phys_slots_narrow` is asserted on every step (an assertion failure stops the report).
> - Positions-and-rotations hash (bars (vii)): FNV-1a 64 over, per body in spawn order, the f32 bit patterns of pose fields 0-2 and 6-9
>   (position, rotation quaternion) of `pose.bin` (13 LE f32 per body, 1240 bodies); the full-pose hash is the SUMMARY `pose_hash`.
> - Holds (bar (iv)): the F0g scorer's `met_pos` (`score/f0g_scorer.py`): drift = horizontal distance to the spawn lattice; holds iff
>   0 boxes > 0.5 m and max drift < 500 mm, and exit code 0.
> - Validity receipts (a run failing one is INVALID and the screen stops, it is not excluded): exit 0; config sleeping on, contact_reuse =
>   the cell, substeps 4, relax 2, broadphase Tree; `void_steps == 0` and `drops_total == 0`; 500 CSV rows and 1240 poses;
>   SUMMARY `pose_hash` = FNV-1a of pose.bin; the stderr `F0_PERTURB` receipt names exactly the seed's box / axis / dir with
>   after bits = before bits + dir (absent on `none`).
> - Continuity (reported; a mismatch is an anomaly, not repaired): P's 34 rows against the CS prototype's V2 cells
>   (`sweep.json` sha256 `b9a3da12d8a3e41a2109d459559e82a9f3e436e4785c1125a23d69d0cffd399f`: pose hash, first frozen, wakes, rise steps,
>   W_sleep). The CS runner at V2 is the trunk's behaviour; P is SL-2a on the same trunk.
>
> ## Bars (quoted from the design and the cut BEFORE any number; each is evaluated for X = L and for X = L+b; blocking)
>
> - **(i)** "0 visible re-wakes (C3b bands)". Operational: for every X run with at least one `contact_wake_rises` entry r, the C3b motion
>   protocol (dump runs `--steps k --window 0..k`, prefix-proved against the full run; states after r-1 and r+59 steps; sampled states at
>   r+{0,1,2,4,8,16,30,45,59}) classifies the interval by max corner displacement (8 corners of the 2 m box) and sampled max corner
>   speed `|v| + |omega| sqrt(3)`: `micro` if corner < 1 mm and speed < 10 mm/s; `sub-pixel` if corner < 16 mm; `visible` otherwise.
>   The bar holds iff the number of `visible` intervals is 0 and no interval is unreadable. An X with no re-wake satisfies (i) vacuously;
>   that is printed as such.
> - **(ii)** "Per seed: W_sleep(L) <= W_sleep(P) in both reuse arms. The mean ratio is <= 1.00 with reuse on (ruling 24(ii)'s letter) and
>   reported with reuse off." Operational: all 34 pairs hold; mean over the 17 reuse-on ratios W(X)/W(P) <= 1.00 (the 16-seed mean
>   printed beside; the reuse-off means reported).
> - **(iii)** "G2/G7 re-expressed events = 0. Generator 0/30 and 0/56 counting re-keys (S-SL2)." Evaluated in `s-sl3/PREREG.md` (S-SL2) and
>   reported in `screens.md`.
> - **(iv)** "Holds 34/34."
> - **(v)** (cut 3.4, replacing the design's) "`seal_breaks == 0 && seal_held >= 1 && settles - seal_held in {0, 1}`" on every run.
> - **(vi)** "`frozen_v_max == 0`" on every run.
> - **(vii)** "On every run where P does not rise, L's positions-and-rotations hash equals P's." P does not rise = P's `contact_wake_rises`
>   is empty in that cell. (Design prediction: 28 runs, 17 reuse-on and 11 reuse-off.)
> - **(viii)** "On every run, L's first frozen step equals P's first attempt: P's first frozen step if P does not rise, else P's first rise step."
> - **(ix)** (cut 3.4) "L and L+b are byte-identical on all 34 runs (pose, CSV non-timing columns, every `sleep` counter,
>   `write_wakes == 0`)". Operational: pose.bin bytes equal; every column of the armed CSV whose name does not end in `_ns` and is not
>   `disp_lane` / `worker_lane_max` equal; every key of L's `sleep` object equal in L+b's; L+b's `write_wakes == 0` and
>   `write_wake_rises == []`.
> - **W8 replays** (identity, part of the verdict): all 8 equal as defined above, exit 0.
>
> Arm D is reported paired by seed (rises, first frozen, W ratio, re-keys), never a bar. Design prediction: `rises(D) >= rises(P)` per seed.
>
> ## Route if a bar fails (design_r2 10.1, written before the first run; quoted)
>
> 1. **(v) fails** (`seal_breaks > 0` on J-T). The seal compares a field the engine itself changes. STOP. The developer finds the field.
>    It may be excluded only if it is **derived** from compared fields, as `inv_inertia` and `bp_margin` are. A field a user can write stays
>    compared. Re-run S-SL1 from scratch.
> 2. **(vii) or (viii) fails while (v) holds.** L's trajectory diverged from P's before P's first attempt, e.g. a small island settled
>    earlier and later merged, or a defect. STOP; diagnose run by run.
> 3. **(ii) fails while (v), (vii) and (viii) hold.** By construction this means a frozen L pile later woke with no external input: a count
>    change at fixed poses, the defect class of `resources.rs:4392-4396`. STOP; diagnose as a defect, not a value.
> 4. **(iii) fails.** The generator's count changes at attempts now include the settle's shift. Ruling 24(iii) applies: to the owner with
>    the numbers and arm D's, no re-pin.
> 5. **(i) or (iv) fails.** STOP.
>
> "No branch lands SL-1 with a failed bar." If SL-1 cannot pass, the orchestrator receives the per-run receipts and arm D.
>
> **Route 2's exit (cut 3.0, quoted):** "If (vii) or (viii) fails while (v) holds, read the run's `sleep.settle_steps`. SUB-ISLAND case: a
> settle of fewer rows than the pile came before the first step where P's and L's untimed CSV columns differ, and that sub-island later
> woke. The run is classed SUB-ISLAND. This is G1's intended behaviour: it resumes from 0 instead of v_N. Such a run is excluded from
> (vii) and (viii) but must still pass (i), (ii) per seed, (iv), (v) and (vi). Every SUB-ISLAND run goes to the orchestrator with its
> receipts. Otherwise route 2 stands: STOP. Expected count: 0."
> Operational: for a run failing (vii) or (viii): `first_div` = the first 1-based step on which P's and X's armed CSVs differ on any untimed
> column (all columns not ending in `_ns`, minus `disp_lane`, `worker_lane_max`); `subisland` = the `settle_steps` entries `[step, rows]` of
> X with `rows < 1240` and `step < first_div`; "later woke" = a rise of P or X after that settle step, or an awake-count increase in X's CSV
> after it; SUB-ISLAND iff `subisland` is non-empty, "later woke" holds and X's `seal_breaks == 0`. Anything else is a bar failure.
>
> ## Predictions (recorded now, not bars)
>
> From `impl_c2.md:101-104`, `design_r2.md` 10.1 and the CS sweep (V2 cells): P rises on exactly the six reuse-off seeds b38xp, b581zm,
> b658xp, b813xm, b968xp, b1046zp and nowhere with reuse on; P reproduces the CS V2 cells on 34/34. L and L+b: 34/34 freeze at P's first
> attempt, 0 contact wakes, 0 breaks, `settles == seal_held == 1` on every run, `settle_rekeys` 1240 on the five risers other than b38xp
> and 0 elsewhere, `frozen_v_max == 0`, W_sleep equal to P's on the 28 non-rising runs and smaller on the six rising ones, positions and
> rotations equal to P's on the 28, L and L+b byte-identical. D: re-wakes where P re-wakes (and possibly more).
>
> ## Verdict rule (fixed now)
>
> S-SL1 PASS iff, for both L and L+b, bars (i) (ii) (iv) (v) (vi) (vii) (viii) hold, bar (ix) holds, the SUB-ISLAND count is 0, the W8
> replays are equal and every run is valid. Bar (iii) is read from S-SL2 and the G2/G7 tests and enters the lane verdict in
> `screens.md`. A failure is reported as FAIL together with the route above; nothing is relaxed or re-defined after the first run.
>
> ## Files pinned with this file
>
> `sl_screens.py` sha256 is written in `PREREG.md.sha256`'s sibling `analysis.sha256` (the definitions above are implemented there);
> `bin/SHA256SUMS`; `patch/SHA256SUMS`.

## Appendix B: the S-SL2 and S-SL3 pre-registration, verbatim

`D:/tmp/phys-orch/sl/screens/s-sl3/PREREG.md`, sha256
`e8d20b9f69bafca1ef1a12eb5628b33710dcc0af69cb315e562510435c97d085` (amendment A1 included, recorded before the
first real S-SL3 run; the first pin was `8a811b6bec15a62e78e3f33e66b0761c2cbb58f75107281a9f420846df980e34`). The text below
is quoted line for line.

> # S-SL2 and S-SL3 PRE-REGISTRATION (lane SL; also reads bar (iii) of S-SL1)
>
> Written 2026-10-12 by the lane's tester, BEFORE the first S-SL3 run and before the first S-SL2 / G2 / G7 run. sha256 in
> `PREREG.md.sha256`; every S-SL3 run's `job.json` carries it. Sources: `D:/tmp/phys-orch/sl/cut.md` section 3.4 (S-SL2, S-SL3), section 3.0
> (O2, W4a-c), Addendum 1 W1 (the structural prefix), `design_r2.md` sections 10.2, 10.3. Untimed: same reading rules as
> `s-sl1/PREREG.md` (SUMMARY whitelist, count columns only).
>
> ## What is known before this file (stated, not hidden)
>
> Format-discovery probes (outside the data, `screens/probe/`) showed that the armed `--dyn kick|shoot --sleeping on` rows exit **3**
> (void) on ALL of C1 (the parent), C2 and C3: `VOID: step 454 (kick) / 258 (shoot): counter phys_bp_queried recorded (1, 1239) /
> (1, 6), the step has (1, 1240) / (1, 1246)`; `void_steps` 2 on kick. The runner's anti-vacuity recount expects every row queried
> (`rows - members`) and a sleeping row is not. It is present at SL-2a (the parent), so it is not an SL regression; the SUMMARY, CSV and
> pose are still written. Rule fixed now: exit 3 whose every `VOID` line names `phys_bp_queried` is accepted and RECORDED as a runner
> finding for a PC line (the armed sleeping dyn row cannot be a clean parity row; the cut already calls S-SL3 "a structural screen");
> any other exit code (2, 101, ...), a `drops_total` > 0, or a void naming another counter is a STOP.
>
> ## S-SL2 (cut 3.4; design 10.2)
>
> - Command (release, from the arm's tree, `--nocapture`, log kept):
>   `cargo test --release -p boyko-physics --test sleep_settles_box_piles --locked -- --ignored --exact flicker_redraw_distribution --nocapture`.
>   Arm **L+b**: in `D:/wt/sl` at HEAD `493be29a`, lane build prefix (`CARGO_TARGET_DIR=D:/wt/_targets/sl-msvc`). Arm **D**: in the
>   `HEAD` export that carries arm D's one-line patch (`bin/runner_D...` source), own target `D:/wt/_targets/sl-screens`.
> - Events = steps on which `contact_wakes` or `settle_rekeys` rose (the test's `attempt_event`).
> - Bars, quoted: **h4: 0 events in 30 draws, freeze at step 61. h5: 0 events in 56 draws, freeze at step 61.** Operational: for L+b, h4 has
>   30 draws, 30 froze, 0 events summed and the freeze-step histogram is `{61: 30}`; h5 has 56 draws, 56 froze, 0 events, `{61: 56}`. Arm D
>   is reported beside, not a bar.
> - Bar (iii) of S-SL1 also reads, on L+b, the release tests G2 `a_height_5_box_pyramid_freezes_within_the_onset_flicker_budget` and G7
>   `sixteen_height_4_draws_freeze_within_the_onset_flicker_budget` (budgets `G2_MAX_EVENTS = G7_MAX_EVENTS = 0`; the tests count the same
>   attempt events): both must pass and print 0 events. Bar (iii) holds iff S-SL2's L+b bars hold and both pass.
>
> ## S-SL3 (cut 3.4; design 10.3; Addendum 1 W1)
>
> - **Arms** (unperturbed lane exes, sha256 pinned): P = `bin/runner_C1_f1f733f6.exe` (`235ed5b5...`), L = `bin/runner_C2_57d6fe62.exe`
>   (`7b13dfd4...`), L+b = `tc/bin/runner_head_493be29a.exe` (`003f33e6...`, the final head). Control: `bin/runner_C3_3bbd9334.exe`
>   (`aaaf9347...`, code-identical to the head exe: equal `.text`) runs each row at W1 and must equal L+b byte for byte (pose, untimed CSV,
>   `sleep` object).
> - **Rows** (cut): S-LAND `--scene jolt --gap 0.5 --cfg default --sleeping on --steps 100 --window 0..100`; S-KICK / S-SHOOT / S-SLIDE
>   `--scene jolt --gap 0.5 --cfg default --dyn <kick|shoot|slide> --sleeping on`. Per arm and row: an armed W1 run
>   (`--arm-profiler --csv --pose-out`), an armed W8 replay, and a `--sanity` W1 run (no profiler).
> - **Bars, quoted from the cut / design**
>   - "`dyn_sanity.py` B1-B3: a failure for ours is a STOP." Operational: `dyn_sanity.py --program <none|kick|shoot|slide> [--steps 100]
>     <sanity.csv>` reports no `B1`/`B2`/`B3` failure, and `--final <program> <pose.bin>` exits 0, on every arm. B4-B6 and the premise
>     checks are printed and reported, not blocking.
>   - "(ii-dyn) per O2 ... Addendum 1 W1": **blocking only on each row's structural prefix `[0, s*)`, reported after.** Quantity: sum of
>     solved slots (`phys_hist_slots_lt32 + _lt64 + _lt128 + _lt256 + _ge256`, CSV rows). Blocking: sum over steps 1..s*-1 (CSV rows
>     0..s*-2) of X <= the same sum of P, for X = L and X = L+b. `s*` (first step, 1-based, on which a row woke whose island had settled
>     or latched earlier, in either arm; operationalised, fixed now) = min of: the program's first external event (S-KICK 201, S-SHOOT 201;
>     none for S-LAND and S-SLIDE); the first element of the union of X's `contact_wake_rises`, X's `write_wake_rises` and P's
>     `contact_wake_rises`; on S-LAND and S-SLIDE (no spawns) the first step whose awake count in X's or P's CSV exceeds the step before.
>     If none exists `s*` is infinite and the prefix is the whole run. The whole-run sums, the scene's metric-window sums
>     (LAND [0,100), KICK/SHOOT [200,800), SLIDE [0,500)) and the sums from `s*` on are reported beside, with the first step on which
>     P's and X's untimed CSV columns differ (needed by the failure route).
>   - "`frozen_v_max == 0` on L+b" (and on L; P's value is reported).
>   - "per kick event, the write vs contact split from `write_wake_rises` / `contact_wake_rises`", with the cut's O2 condition
>     `write_wake_rises` is a subset of {kick steps + 1} = {201 + 25k : k = 0..23} (blocking on L+b; L must have no write wake).
>   - W8 replay identity per arm and row: pose bytes, `sleep` object and the core CSV columns equal to the W1 run's (exit 0 or the accepted 3).
> - **Failure route (Addendum 1 W1, quoted):** "a prefix red is a STOP with, per row, both arms' per-step solved-slot columns, `s*`, and the
>   first step on which the arms' untimed CSV columns differ; it goes to the orchestrator, never relaxed." S-SL3's FAIL (any blocking item) is
>   reported with that route; "(ii-dyn) is reported, not blocking" from `s*` on.
> - **Predictions (not bars):** S-LAND: nothing settles within 100 steps (first freeze 146), `s*` infinite, sums equal. S-KICK: `s*` = 201,
>   the pile frozen since about step 146 wakes on the first kick through a write wake (L+b) and stays awake. S-SHOOT: `s*` = 201. S-SLIDE:
>   `s*` read from the run. `frozen_v_max` 0 on L and L+b.
>
> ## Files pinned with this file
>
> `sl_screens.py` (sha256 in `analysis.sha256`), `bin/SHA256SUMS` and `D:/tmp/phys-orch/sl/bin/SHA256SUMS`.
>
> ## Amendment A1 (2026-10-12, before the first REAL S-SL3 run; the first real S-SL1 run had not started either)
>
> The original text above (sha256 `b2adfe478e9948481dcefa9387ae0c8ea9647d8a3e5c64ad16bfcb417a30627b` is the S-SL1 file; this file's first pin was
> `8a811b6bec15a62e78e3f33e66b0761c2cbb58f75107281a9f420846df980e34`) said "`frozen_v_max == 0` on L+b (and on L; P's value is reported)".
> A code-path dry run of the reader in a scratch root (`screens/dry/`, three seeds for S-SL1 and the S-SL3 rows, results discarded as
> screens) showed that the clause "and on L" is stricter than the cut and contradicts the cut's own mechanism: L is SL-1 without SL-1b, it
> has no write test, so S-KICK writes a velocity into a frozen row and L's `frozen_v_max` is expected to be above 0 there (the parent's
> too). The cut (3.4) and O2 (3.0) bind `frozen_v_max == 0` to **L+b** ("with SL-1b a written row wakes before the fold, so it must read 0
> everywhere"). Corrected text: **`frozen_v_max == 0` on L+b (blocking); L's and P's values are reported.** No other bar changed. The same
> dry run showed that only the FIRST void step's counter is printed by the runner, so the accepted-exit rule above is supplemented by a
> recorded (non-blocking) check that `phys_bp_queried + phys_bp_members` is conserved on every void step.
