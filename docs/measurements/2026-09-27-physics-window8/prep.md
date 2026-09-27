# Physics window 8: prep (2026-09-27). Ready to launch. Nothing was timed, and no timing number was read.
W = C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8

**Launch:** `bash $W/run_window8.sh --cutoff HH:MM [--resume]`. Preview with `--dry-run --start HH:MM`; stop by creating `$W/STOP`.
Output: raw/ (runs.jsonl, stdout/stderr per process), progress.txt, raw/counts.txt, WINDOW_DONE (exit 0 complete / 2 cut / 3 idle never came or binaries changed / 5 crash).

## Binaries (bin/SHA256SUMS; provenance in bin/COMMIT.txt)
| key | file | sha256 | commit |
|---|---|---|---|
| instr | runner_226bd99e.exe | c3cef91c3de75bc86c227a41a65c192d1a6ccacdc1278b7402fcabb2ac46aba0 | 226bd99e (exported tree, `--profile parity`) |
| omega | omega_b_region_226bd99e.exe | ffcac12f8b1fdded714619c756c98bb375df639de70864ce1d540f2bf1fa96e5 | 226bd99e |
| dmA | dm1_A.exe | 47cb2c9b6f419948cc773d7da392a1d7dc4ebe218875e66180b07fddf3a52c75 | cad5439b (HEAD at build, verified) |
| dmB | dm1_B.exe | 79bba864480255c2e90a1067d91e8bea7ed891e7916f760791854d01ef2b40ad | 97ee830f (HEAD at build; A and B hashes differ) |
| j56 | D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | 918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad | window 7's j56 |
| j56p | jolt56prof8/PerformanceTest.exe | b35986ef2b793735da119d5b465fee1ccc762c08283be4f41297c67d431ef495 | j56p recipe on a build copy, patched |
- rustc 1.98.1 msvc with `--locked`; all three commits carry one lock (LF sha fab5f63f…), unchanged by the builds.
- **DM1 is a dev-profile build** (the cut's `cargo test --no-run`: unoptimized, with debuginfo). The A/B reads GPU zones.
- **D:/wt/_dm1_ab is left at 97ee830f** for you to remove after the window.
- **j56p was patched, not reused** (about 15 minutes, so the "window-7 profile weight" fallback was not needed). tools/patch_jolt_w8.py on the copy D:/wt/_targets/w8-jolt56prof-src:
  - removes the three scopes carrying window 7's 8,486 + 5,390 + 5,390 samples/frame ("Add Constraint From Cached Manifold" and the two per-batch SolveVelocityConstraints scopes);
  - adds the WaitingForBatch counter: rdtsc only at each wait's two edges, summed per job, emitted once per job. `-wfb` writes wfb_<tag>.csv per frame; the tsc rate is traced.

## Rows omitted, and why
- **S4-AB:** S4 is not built.
- **DM1 idle at 2560x1440 and 3840x2160 (both paths):** the display is 2560x1440 physical at 125 % scaling. Both requests came up 2052x1133 and the harness asserted NOT MEASURED (gate/dm1_res.json).
- **omega exe:** no `--help` (it runs its self-check, which passed). Both spec modes exist (`omega-b`, `omega`) on both routes; none missing.

## Orchestrator amendments (applied)
1. **omega_b** runs at `--stages 36` and `72` on both routes (4 rows). The gate confirmed every SUMMARY carries the requested stages.
2. **omega(W, gap):** unchanged. 3. **No per-wave-split rows** added.
4. **Armed/disarmed twins.** J-T-a, J-T-off-a and J-A-a already had theirs at W 1/8/16, same block and round. I added **J-Son (disarmed)**, and gave **both J-Son rows W1** because D(W) needs the W1 term.
   - The W1 pair goes beyond the spec (J-Son-a at W8 only): +3 cells, about 1.2 min. Delete it in tools/mkrows8.py if unwanted.
   - Pass 0 runs each W group as J-T, H-jolt56, J-T-a, J-T-off, J-T-off-a, J-A, J-A-a, J-Son, J-Son-a, so twins share round and W. Pass 1 is reversed.

## Deviations from the spec text
- **W8S-R has its own references:** J-T at W 1/2/4/8/16 and J-T-a at W8 run inside it (as window 7b's Q4), so rungs and zone canaries compare within one block.
- **J-A / J-Son fixtures come from the gate** (`--pose-out` there, `--expect-pose` on every timed process); the spec says pass 1.
- **Priorities 1–5 follow the block order;** a block that does not fit before the cutoff is skipped whole (the 7b rule).

## Minutes (one untimed dry sample per cell; each pass includes the ≥135 s idle rule)
W8S-A 25.6 (150 processes) · W8S-R 17.8 (114) · micro 8.9 (36) · DM1 5.9 (19: ABBA per group, then 3× grow on B) · P-jolt56-prof 7.5 (18). **Total about 66 min** (dryrun.txt).

## Untimed gate (gate/, gate_run.log): everything passed
- **Fixtures:** JT500 0x30c5438bc6ad9ffa and JToff500 0x32d5e235342b4143 (both as the spec); JA500 0x30c5438bc6ad9ffa and JSon500 0x3db47fae414b655c (recorded, JSon at W8).
- **Runner: 105 of 105 pass** (21 rows × W 1/2/4/8/16, `--steps 500`).
  - Every process: exit 0, expected pose, `expect_pose` match, `void_steps 0` armed and disarmed.
  - Armed runs carry `w8s`; ladder/rung runs report `canary_ns`; zone runs report `phys_solve_build` with the right N. J-Son's pose is equal at every W.
  - **`phys_setup_chunks` does not exist at 226bd99e** (0 source hits, no CSV column), so the "reads 0" check does not apply.
- **Red controls:** 501 steps vs JT500, and J-T-off vs JT500, each exit 4. The pose gate can fail.
- **micro:** self-check ok; the smallest configuration (P2, 10 regions; W8, gap 0, 10 reps) exits 0 on both routes; 6/6 full-size dry samples exit 0 with 4/4/8 SUMMARY lines.
- **Jolt:**
  - j56 at `-t=1` (and 8/16) reads hash 0xb8522b4e3fc62cfe.
  - j56p at W 1/8/16: same hash, dumps 0–400, removed scopes absent, wfb 500 frames with W jobs each. Wait episodes appear at every W, W1 included; not interpreted.
- **DM1:**
  - Crash check: A and B at 1280x720 (10 frames) exit 0, "1 passed", frames=31; the zone leg ends by itself and writes the artifact.
  - 220-frame samples (vb idle, deferred idle, vb edit100, B grow150) exit 0; read zones present with n=220 (ids 2 and 14 for vb, 17 for deferred).
- **Driver:** the `--test` rehearsal of all 5 blocks exits 0. The first rehearsal found a rehearsal-only bug (the 10-frame cap never reached the grow frame); fixed, re-run 0 invalid. Hang kill tested (own handle, 3 s, exit 98); idle script smoke-tested.

## Disclosure
The runner has no `--help` either: my first call ran its 3-step s16 self-check, which printed a timing field (not recorded, not used). No commits; no repository worktree edited except the D:/wt/_dm1_ab checkout.
