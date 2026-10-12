# Lever SL: the pins

Every pin lane SL moved, and every pin it had to keep. All moves are at C2 = SL-1 `57d6fe62`. C1 (SL-2a),
C2b, C3 (SL-1b) and the fix rounds moved no pin. A pin that moved at C3 would have been a STOP (cut §3.3);
none did.

The readings on the final head are the close run's (`D:/tmp/phys-orch/sl/test_close.md`, HEAD `493be29a`).

## 1. The rule

A7-R1's re-pin rule (`crates/boyko_physics/tests/sleep_settles_box_piles.rs`, at `A7_R1_D_MAX_BITS`):

> **Re-pin rule.** The value moves only with a commit that changes values by design (a value-changing
> lever). That commit re-reads it from A7-R1's own `D_max` line (msvc release), re-pins it here in the same
> commit, and names the lever and the old value in this doc. In a commit that claims bit identity, a change
> is a defect, never a re-pin.

SL-1 is an owner-approved value-changing lever (ruling 25; `docs/physics/perf-campaign/00-RULINGS.md` owner
D2), and its purpose is the stored-velocity defect.

Ruling 39 Q4 extends the rule to the G1 pins in `crates/boyko_physics/src/solver/colored_tests.rs`. Their
block now reads:

> A red pin is a defect, never a re-bless (`00-RULINGS.md`, per-contact parity). The one exception is an
> owner-approved value-changing lever whose purpose is the move (ruling 39, lane SL): its commit re-reads
> the value from this gate's own output, re-pins it and names the lever and the old value (A7-R1's rule,
> `tests/sleep_settles_box_piles.rs`). Any other red is a defect.

What the record carries per moved item (cut §4.1): the old and new value, the command, the
positions-and-rotations hash for P and L, and the first frozen step for P and L.

**The masker.** The positions-and-rotations hash is FNV-1a 64 over the f32 bit patterns of pose fields 0-2
and 6-9 (position, rotation quaternion) of each 13-float pose record, in spawn order. Where it is equal for
P and L, the move is in velocity bytes only.

## 2. Moved on purpose (C2 `57d6fe62`, lever SL-1)

| pin | site | old | new | command (msvc, the lane tree, `--locked`) | masked P = L | first frozen P → L | reason |
|---|---|---|---|---|---|---|---|
| `ANCHOR_REUSE_ON_SLEEP_ON` (JSonT500) | `crates/boyko_physics/tests/v2_speculative_fidelity.rs` | `0x130c_76cb_6b46_3ab8` | `0xc637_a051_7ada_bab7` | `cargo test --release -p boyko-physics --test v2_speculative_fidelity -- --ignored` | `0x832c403cd7b63347` = `0x832c403cd7b63347` | 147 → 147 | velocity bytes only: the step that puts the pile to sleep zeroes its velocities. It equals E3's `v2/w8/JSonT500` |
| `ANCHOR_REUSE_OFF_SLEEP_ON` (JSonToff500) | the same file | `0xc671_8318_9439_4df6` | `0x4aec_2a5f_78f4_88d0` | the same | `0x6d926dea060f48e0` = `0x6d926dea060f48e0` | 142 → 142 | velocity bytes only |
| `PIN_S_B` (G1 scene S-b) | `crates/boyko_physics/src/solver/colored_tests.rs` `mod g1_pins` | `0x40e4_a08c_7eb6_f631` | `0xe9bd_c7f6_80f7_80ca` | `cargo test [--release] -p boyko-physics --lib -- g1_pins` | — (a direct-drive hash, not a pose file) | — | direct drive zeroes a sleeping island's velocities at its settle (ruling 39 Q4), so the frozen steps' body bits and the wake's start moved. All four arms agree, in debug and in release |
| V2-set pose-gate fixtures, the 34 sleeping-on files | E3 `docs/measurements/2026-10-11-sl-sleeping/v2/` (were the V2 record dirs) | section 3 | section 3 | `bash docs/measurements/2026-10-11-sl-sleeping/record_sl_fixtures.sh <exe> <dir> <logdir>` | equal on every one | unchanged (147 / 142 / 86 / 82) | VELOCITY-ONLY, every one |
| d0-set pose-gate fixtures, the 32 sleeping-on files | E3 `d0/` (were the four d0 record dirs) | section 3 | section 3 | the same recorder, through `gate/runner_d0.sh` | moved | 274 → 214, 265 → 205, 250 → 70, 248 → 68 | POSES-MOVED, every one, as cut §4.1 predicted. Under `d = 0` every row rose at its first attempt on the parent; SL re-keys it (1240) and freezes at the first attempt |
| the four d0 hash literals of `gate/l10/pose_gates_sync.sh` | `docs/measurements/2026-09-30-v2-speculative/gate/l10/pose_gates_sync.sh` | `0x3db47fae414b655c`, `0xcc2a5400c66eecce`, `0xb7f1e9e8f91f75ab`, `0x2a2b7926a48aab00` | `0x74c14f3a08e24421`, `0x13d95ddbd8f6907e`, `0xa42b663994f7b36d`, `0xd70e51f6ae4dd2b8` | read from `record_sl_fixtures.sh`'s `logs/record.tsv` | — | — | the d0 rows above |
| the `HS` / `HR`, G4p and G4f literals of the regenerated `pose_gates_sync_v2.sh` | `gate/pose_gates_sync_v2.sh` (written by `gate/make_v2_scripts.py` from E3's `logs/record.tsv`) | J-T reuse on `0x130c76cb6b463ab8`, reuse off `0xc671831894394df6`; rest reuse on `0xfcf3d49356b45a08`, reuse off `0xe99ed04348f5d898` | `0xc637a0517adabab7`, `0x4aec2a5f78f488d0`; `0x9a30f553cdd9d7eb`, `0x413825ded25d21ed` | `python make_v2_scripts.py` | — | — | the V2 rows above. The two negatives compare the reuse-on and reuse-off `R-S` files, which still differ |

The pins carry their old value, the lever and the reason in their doc comments, in the same commit.

## 3. The fixtures (ruling 39 Q1 + Q2: the mirror E3)

The sleeping-on pose-gate fixtures were re-recorded into the new mirror
`docs/measurements/2026-10-11-sl-sleeping/`. No record directory was edited (V2's precedent: the originals
are read, never edited). That directory's `README.md` holds the per-file old/new table: pose hash, masked
hash, first frozen step, and re-keys / breaks / wakes, for every one of the 66 sleeping-on files.

The distinct values (each row stands for every file of that scene and arm, W1 and its W8 copy):

| set | scene, arm | parent pose hash | SL-1 pose hash | masked hash, parent → SL-1 | first frozen | SL-1 re-keys / breaks / contact wakes |
|---|---|---|---|---|---|---|
| V2 | J-T sleeping on, reuse on (`J-Son`, `J-A-on`, `J-D-on`, `Offp-J1000`, `JSonT500`) | `0x130c76cb6b463ab8` (0 contact wakes) | `0xc637a0517adabab7` | `0x832c403cd7b63347`, equal | 147 → 147 | 0 / 0 / 0 |
| V2 | J-T sleeping on, reuse off (`J-Son`, `J-A-on`, `J-D-on`, `Son-J1000`, `JSonToff500`) | `0xc671831894394df6` (0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0`, equal | 142 → 142 | 0 / 0 / 0 |
| V2 | rest, reuse on (`R-S`, `R-on`, `RSOffp800`) | `0xfcf3d49356b45a08` (0) | `0x9a30f553cdd9d7eb` | `0x35be10df71a4ec7b`, equal | 86 → 86 | 0 / 0 / 0 |
| V2 | rest, reuse off (`R-S`, `R-on`, `RS800`) | `0xe99ed04348f5d898` (0) | `0x413825ded25d21ed` | `0x392834e09920966d`, equal | 82 → 82 | 0 / 0 / 0 |
| d0 | J-T sleeping on, reuse on | `0x3db47fae414b655c` (1240) | `0x74c14f3a08e24421` | `0xe64f927f27605db8 → 0x0924f2418b612a51` | 274 → 214 | 1240 / 0 / 0 |
| d0 | J-T sleeping on, reuse off | `0xcc2a5400c66eecce` (1240) | `0x13d95ddbd8f6907e` | `0xcfe7596eeecf89a7 → 0x156318f8c69bc63e` | 265 → 205 | 1240 / 0 / 0 |
| d0 | rest, reuse on | `0xb7f1e9e8f91f75ab` (3720) | `0xa42b663994f7b36d` | `0x0763860bfa825944 → 0xc54cfc6a31fd65ed` | 250 → 70 | 1240 / 0 / 0 |
| d0 | rest, reuse off | `0x2a2b7926a48aab00` (3720) | `0xd70e51f6ae4dd2b8` | `0xaa8d8e0f6118df32 → 0x3c6a731c7784a2c8` | 248 → 68 | 1240 / 0 / 0 |

The receipts:
- **Sleeping off: nothing moved.** The parent's runner (SL-2a, `f1f733f6`), through the same recorder,
  reproduces all 105 record-directory files byte for byte. On C2, the 39 sleeping-off files (23 V2 + 16 d0)
  are byte-equal to them. The session table is `D:/tmp/phys-orch/sl/c2/e3_table.tsv`: `rows 105, problems 0`.
- **The mirror is intact:** `sha256sum --check SHA256SUMS` in E3 reads 105/105 OK (test round 1).
- **The negatives still see a difference.** The reuse-on and reuse-off `R-S` files differ in both sets: V2
  `0x9a30f553cdd9d7eb` / `0x413825ded25d21ed`, d0 `0xa42b663994f7b36d` / `0xd70e51f6ae4dd2b8`. Both
  negatives exit 4 in both sets, on the final head.
- **E3 mirrors the files the gates read.** Window 6's `J500`, `JAon500`, `R1100`, `Ron1100`, `S16-300` and
  `fixtures.json` are not mirrored: no gate reads them.
- **The re-pointing.** `make_v2_scripts.py` asserts that no SL d0 path survives into a `_v2` script. The
  P5 tallies are unchanged: 264 / 26 / 17.

## 4. Read, and unmoved

| item | reading at C2 and C3 | consequence |
|---|---|---|
| G2 / G7 onset-flicker events (`sleep_settles_box_piles.rs`, re-expressed through `attempt_event`) | G2 froze at 61 with 0 attempt events; all 16 G7 draws froze at 61 with 0 events. Budgets stay 0 | ruling 24 (iii) ("a G2/G7 value move is reported to the owner before merge") has nothing to report |
| A7-R2's freeze reading | 86 / 0 / 0 | unchanged |
| the generator (S-SL2) | h4 0 events in 30 draws, h5 0 in 56, all freezing at 61 | `02-SCREENS.md` §3 |

## 5. Must stay byte-identical: the readings on the final head

Command prefix: the lane's (msvc, `CARGO_TARGET_DIR=D:/wt/_targets/sl-msvc`, `--locked`).

| pin | command | value on `493be29a` | status |
|---|---|---|---|
| P1 pyramid, release | `cargo test --release -p boyko-physics --test default_world_pyramid_determinism -- --nocapture` | `0xe5d8a7d5f4694f08`; reuse off `0x78121c47f5c6e8b2`; d0 `0xb583189fa681f3a6`; d0 reuse off `0xa38620b38cbca8d3`. AllPairs, 1w/2w/8w, parallel on/off and the scalar oracle all equal | unmoved |
| P1 pyramid, debug | the same without `--release` | `0x211297c3d16c603c`; `0x0fc5719433dad377`; `0x839d94268d67b09f`; `0xc7eb531b1e1ac19b` | unmoved |
| P2 A7-R1 (sleeping off, release only) | `cargo test --release -p boyko-physics --test sleep_settles_box_piles -- --exact a_resting_jolt_pyramid_does_not_creep_with_sleeping_off --nocapture` | D_max 0.0007244835 / 0.0005266388 / 0.0006670445 m = bits `0x3a3deb44` / `0x3a0a0e22` / `0x3a2edc99` | unmoved ("a change is a defect, never a re-pin": A7-R1 forces sleeping off) |
| P3 GOLDEN | `cargo test [--release] -p boyko-physics --test bodytype_determinism_golden` | 3/3 in both profiles; `0x1575_326A_EB80_3052` | unmoved |
| P4 runner pins (sleeping off) | `bash docs/measurements/2026-09-30-v2-speculative/gate/runner_pins.sh <exe> <out>` | `rows 12, pass 12, fail 0`. W1 and W8: J500 `0x441a568e91a4f9c9`, J500off `0xbc09a1faf7b413a8`, R1100 `0x21bc7ecb642b0090`, d0 `0x30c5438bc6ad9ffa` / `0x32d5e235342b4143` / `0xc8bbe34cf6a8afc6` | unmoved |
| P5 pose gates, V2 set and d0 set | `pose_gates_v2.sh`, `pose_gates_d0.sh` | each: both negatives `exit=4 (want 4)`; `runs 264, pass 264, fail 0`; `checks 26, pass 26, fail 0`; `checks 17, pass 17, fail 0`; `ALL_DONE` (sleeping-on rows on E3) | tallies unchanged |
| P6 alloc census | `cargo test [--release] -p boyko-physics --test alloc_frame_census -- --nocapture` | `GATE: GREEN - 19 scenes, every class inside its pin`, debug and release (S1d and S8 run sleeping on) | unmoved |
| P7 / P8 | `--features bp-query-counts --test bp_query_counts` / `--features narrowphase-counts --test narrowphase_census` | 2/2 and 1/1 (test round 1; the later delta changed no engine line) | unmoved |
| P9 UG-02 | `cargo test -p boyko-engine --test runtime_data_ledger_gate`; `tail -1 docs/memory/ledger/ug02-pins.tsv \| cut -f1,7,8` | 49/49; `SR-B-C6 5 7` (SL adds no pool site) | unmoved |
| P10 V2 fidelity | `cargo test --release -p boyko-physics --test v2_speculative_fidelity -- --ignored` | 4/4. Sleeping-off anchors `0x441a568e91a4f9c9` / `0xbc09a1faf7b413a8` unmoved; sleeping-on anchors at section 2's new values; the red control `red_control_the_velocity_term_capped_at_1_mm_fails_at_gap_1_5` prints `FAILS` | the two sleeping-off anchors and the 26-run gate unmoved |
| P11 scenes | `docs/physics/perf-campaign/levers/scenes/gate/ours_gates.sh`; `docs/measurements/2026-09-30-v2-speculative/gate/s5_runner_gate.sh` | `checks 49, pass 49, fail 0`; `rows 54, pass 54, fail 0`. `pins.json`: none `0x441a568e91a4f9c9`, kick `0x5c27d4ede389f12a`, shoot `0x49e2bf025baa5294`, slide `0xe627f35c282605ba` | unmoved |
| `v2_speculative_d0_identity.rs` | in the workspace run | 10/10. The cut and `pack_r1.md` say 12; the file holds 10 tests and is unchanged since the base | unmoved |
| PIN_S_A, PIN_S_C .. PIN_S_G | `--lib -- g1_pins` | 7/7 with PIN_S_B's new value | unmoved |
| the sleeping-off byte receipt | `existing_unchanged.py`, 10 rows (J500 W1/W8, J500off, J500d0, R1100, S16a, J100allpairs, J100armed W1/W8, selfcheck) | `rows 10, same 10, diff 0`, against the round-1 parent exe and against a parent exe rebuilt from `264e5b2d` (sha256 `19e8faa2903530d11cbc1adc247fea17875a317abca3cf88157a4064cab561b9`). Red control: the sleeping-on row `J500sleep_W1` reads `rows 1, same 0, diff 1` | unchanged |

**Relative gates, green with no edit:** `default_world_sleep_worker_invariance`,
`sr_region_worker_invariance`, `sr_region_store`, `broadphase_tree_scenes`, `sdf_sleep_settles_and_wakes`,
`sleeping_pipeline_o8`, `support_loss_wakes_sleepers`, and the `colored_tests.rs` direct-drive sleeping
tests. `sleep_skip_bit_identity.rs` was edited for LB5 only (ruling 40), and its 58 tests are green.

## 6. Zero cost when sleeping is off

- **The census.** `existing_unchanged.py` shows SAME on every sleeping-off row at each rung: C1 against the
  base, C2 against C1, C3 against C2, and the head against the base. `J100armed_W1/W8` is included (the
  armed CSV columns are equal, and no new zone opens).
- **The codegen receipt** (SR-B's recipe: `cargo rustc --locked -p boyko-physics --lib --release --config
  profile.release.lto=false --config profile.release.codegen-units=1 -- --emit asm`, extracted by
  `D:/tmp/phys-orch/sr-b/cg/fnasm.py`):
  - **C2 against C1.** `ColoredSoftStepSolver::solve_colored` (sleeping off) gains one `movq $0` (the
    `None` seal argument), and its frame grows from 296 to 312 B. The off arm of `physics_solve_colored`
    differs by the same store. Every new call in `solve_colored_inner` is an `IslandSleep` method inside an
    `if let Some(sleep)` block: `begin_step_armed` ×2, `settle_cold`, `mark_settled`.
  - **C3 against C2.** `solve_colored` 13 → 13 instructions; `solve_colored_inner` 6171 → 6171;
    `physics_solve_colored` 134 → 134; all 62 `solve_colored*` symbols differ by 0 lines. Only
    `IslandSleep::begin_step_armed` grows, 520 → 561 (the write test).
