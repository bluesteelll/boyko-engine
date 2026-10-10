# Physics perf campaign, window 6 — the L10 pre-C0 refutation, L9's C0 reading and the C4 decision, the tree query after C3b, and L11 G9's remainder (2026-09-24)

Receipts for `docs/MEASUREMENT-QUEUE.md` section 14, the `RESULT, 2026-09-24, window 6` block (one sub-block per
lever: L10, L9, the tree broadphase, L11), and for the four dated lines of 2026-09-24 in
`docs/physics/perf-campaign/levers/00-RULINGS.md` (the L10 block: the pre-C0 refutation does not fire; the L9 block:
C4 is built; the broadphase block: C3b ships and F3 is taken up; the L11 block: G9's remainder passes, G9 not formally
closed). Read `analysis.md` first: it is the results-analyst's reduction, recomputed from `raw/` with its own script
(`tools/analyze6.py`), not from the tester's `tools/reduce6.py`. The plan the window executed, with every item's
source, rows and bars, is `plan.md`.

`analysis.md` is verbatim from the text the analyst returned. The analyst's role could not write files, so the text
was not on disk in the scratch directory `win6/`; the recording step wrote it here unchanged, apart from dropping two
things that were not part of the document: the analyst's leading note to the orchestrator and its trailing one-line
`result:` summary. Its closing "Files are in …" paragraph (including "`analysis.md` — not written") describes the
scratch directory at the time of writing; wherever it writes `win6/<path>`, read `<path>` under this directory. Its
other inputs are committed:
- window 5 (`d9f1cf70…/win5/`) is `docs/measurements/2026-09-23-combined-tree-l11/`;
- window 4b (`win4b/`) is `docs/measurements/2026-09-22-l11-solve-setup/`;
- window 4 (`win4/`) is `docs/measurements/2026-09-22-broadphase-tree/`;
- window 3 (`D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/`) is `docs/measurements/2026-09-21-physics-window3/`.

The design notes it cites by scratch path — `c3b/design.md`, `treebp/g4_g5_recipe.md`, `l9/h_tau.md`, `l9/c3.md`,
`l10/plan.md` — are the orchestrator's working files under
`C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/`.
They are not in the tree. The lever designs it cites by `NN-*.md` name are under `docs/physics/perf-campaign/levers/`.

There is no `window_report.md`: the tester's reduction is `raw/reduction.json`, rendered as `raw/tables.md`, and
`analysis.md` § 0 states that every cell of it reproduces (and lists the five places where the analyst reads the
rules differently).

## What was measured

Five items, run as twelve blocks in priority order (`plan.md` § 4; `rows6.json` holds the rows):

| item | question | blocks | binaries |
|---|---|---|---|
| P1 | L10's pre-C0 refutation (`levers/L10-sleeping/08-DESIGN-REV2.3.md:221`): stop L10 if the armed Off′ span on J-Son at W=1 is below 0.584 ms | `P1-L10` | b4db |
| P2 | L9's C0 refutation (`levers/L9-contact-reuse/02-DESIGN-REV1.md:380`) and the realized-gain rule (`:407-410`), which decide C4 | `P2a-L9classes`, `P2b-L9reuse` | c4db, b4db |
| P3 | the tree query cost after C3b (RowWalk against LeafList), and the action table of `c3b/design.md:365-376` | `P3-C3b` | par, tip |
| P4 | the rest of L11's G9 on C3 (`levers/L11-solve-setup/02-DESIGN-REV1.md:359-371`; window 5 left J-A, R, R-S, S16, W 2/4/16 and the canary open) | `P4-G9` | g9p, g9t |
| P5 | extras: bridge 2 (`P5a`), L10's armed Off′ spans at W 8 and on R-S (`P5b`), G5 on the C3b tip (`P5g`), L9 reuse off/on on J-D, R and J-A at W 2/4/16 (`P5d`, `P5e`, `P5f`), the G4 criterion on the C3b tip (`P5c`) | `P5a-bridge2`, `P5b-L10spans`, `P5g-C3bG5`, `P5d-L9reuseJD`, `P5e-L9reuseR`, `P5f-L9reuseJAmid`, `P5c-C3bG4` | w4bt, g9p, b4db, tip, bpb |

| row | block | binaries | args (+ `--workers W --steps N --window a..b --csv --pose-out --expect-pose gate/fixtures/<ref>.pose`) | W | steps [window] | armed | pose ref |
|---|---|---|---|---|---|---|---|
| `L10-Offp` | P1-L10 | b4db | `--scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on` | 1 | 1000 [264,1000) | yes | Offp-J1000 |
| `L10-Son` | P1-L10 | b4db | `--scene jolt --gap 0.5 --cfg a --sleeping` | 1 | 1000 [264,1000) | yes | Son-J1000 |
| `L9-classes` | P2a-L9classes | c4db | `--bench` | — | — | — | — |
| `L9-JA-off` / `L9-JA-on` | P2b-L9reuse | b4db | `--scene jolt --gap 0.5 --cfg a --contact-reuse off` / `on` | 1, 8 | 500 [0,500) | no | J500 / JAon500 |
| `L9-JA-a-off` / `L9-JA-a-on` | P2b-L9reuse | b4db | the same, armed | 1 | 500 [0,500) | yes | J500 / JAon500 |
| `C3b-TA-armed` | P3-C3b | par, tip | `--scene jolt --gap 0.5 --cfg a --broadphase tree` | 1, 8 | 500 [0,500) | yes | J500 |
| `C3b-TD-tree` | P3-C3b | par, tip | `--scene jolt --gap 0.5 --cfg default --broadphase tree` | 1, 8 | 500 [0,500) | no | J500 |
| `G9-JA` | P4-G9 | g9p, g9t | `--scene jolt --gap 0.5 --cfg a` | 1, 8 | 500 [0,500) | no | J500 |
| `G9-R` | P4-G9 | g9p, g9t | `--scene rest --solver colored` | 1, 8 | 1100 [600,1100) | yes | R1100 |
| `G9-RS` | P4-G9 | g9p, g9t | `--scene rest --sleeping` | 1, 8 | 800 [300,800) | yes | RS800 |
| `G9-S16` | P4-G9 | g9p, g9t | `--scene s16 --cfg a` | 1 | 300 [0,300) | yes | S16-300 |
| `G9-JAs-mid` | P4-G9 | g9p, g9t | `--scene jolt --gap 0.5 --cfg as` | 2, 4, 16 | 500 [0,500) | no | J500 |
| `G9-JAs-a` | P4-G9 | g9p, g9t | `--scene jolt --gap 0.5 --cfg as` | 1 | 500 [0,500) | yes | J500 |
| `G9-JC` (canary) | P4-G9 | g9p, g9t | the same plus `--canary-frac 0.05 --canary-ref-ns <the latest valid G9-JAs-a mean of the same binary>` (P0's recipe) | 1 | 500 [0,500) | yes | J500 |
| `B2-JAs` | P5a-bridge2 | w4bt, g9p | `--scene jolt --gap 0.5 --cfg as` | 1, 8 | 500 [0,500) | no | J500 |
| `L10-Offp8` / `L10-Son8` | P5b-L10spans | b4db | as `L10-Offp` / `L10-Son` | 8 | 1000 [264,1000) | yes | Offp-J1000 / Son-J1000 |
| `L10-RS-Offp` | P5b-L10spans | b4db | `--scene rest --sleeping --broadphase tree --contact-reuse on` | 1, 8 | 800 [300,800) | yes | RSOffp800 |
| `L10-RS` | P5b-L10spans | b4db | `--scene rest --sleeping` | 1, 8 | 800 [300,800) | yes | RS800 |
| `G5-TA-tree-a` / `G5-TA-ap-a` | P5g-C3bG5 | tip | `--scene jolt --gap 0.5 --cfg a --broadphase tree` / `allpairs` | 1, 8 | 500 [0,500) | yes | J500 |
| `G5-TD-tree` / `G5-TD-ap` | P5g-C3bG5 | tip | `--scene jolt --gap 0.5 --cfg default --broadphase tree` / `allpairs` | 1, 8 | 500 [0,500) | no | J500 |
| `L9-JD-off` / `L9-JD-on` | P5d-L9reuseJD | b4db | `--scene jolt --gap 0.5 --cfg default --contact-reuse off` / `on` | 1, 8 | 500 [0,500) | no | J500 / JAon500 |
| `L9-R-off` / `L9-R-on` | P5e-L9reuseR | b4db | `--scene rest --solver colored --contact-reuse off` / `on` | 1, 8 | 1100 [600,1100) | no | R1100 / Ron1100 |
| `L9-JA-off-mid` / `L9-JA-on-mid` | P5f-L9reuseJAmid | b4db | `--scene jolt --gap 0.5 --cfg a --contact-reuse off` / `on` | 2, 4, 16 | 500 [0,500) | no | J500 / JAon500 |
| `C3b-G4` | P5c-C3bG4 | bpb | `--bench --noplot` filtered to `bp_g4_scene/{tree,tree_rowwalk}/{j100,1240}` and `bp_g4_{uniform,disparity}/{tree,tree_rowwalk}/1000` | — | — | — | — |

**Protocol** (`plan.md` § 1, `rows6.json` → `protocol`), the one ruled after P0 and used by windows 3, 4, 4b and 5:
- **Cell and claim rule.** A cell is the MEDIAN over K separate processes of each process's statistic (the mean of
  `wall_ns` over the row's window; armed rows also keep every zone column's per-process mean and median over the
  window, over [100,500) and over the all-frozen tail). Spread: r = min–max and i = IQR, with the median's SE
  s = 1.2533·SD/√K beside them, each relative to the median. B against A is claimed iff
  |B/A − 1| > 2·√(sA² + sB²) under BOTH r and s; i is printed as well.
- **K and ordering.** **K = 6**: every block is two passes × three rounds. Pass 0 orders the cells by W ascending,
  row by row inside a W group with each row's binaries in their listed order, so the arms alternate inside the W
  group; pass 1 is the whole list reversed; each pass opens with one untimed warm-up process.
- **Idle rule before EVERY pass** (`tools/wait_idle6.ps1`, window 5's `wait_idle5.ps1` with `MaxPolls` 30): three
  consecutive 60-s polls with no build process (cargo, rustc, link, lld-link, dxc, clippy-driver, cl, msbuild, miri,
  cargo-miri, and the three bench exe names), no process whose image is under `D:/wt/_targets` or `D:/wt/mq-*`, and
  a 10-s CPU below 5 %. A 30-minute timeout stops the window.
- **Receipts.** A 10-s receipt opens each pass; a 5-s receipt sits between processes (the receipt after process i
  is the receipt before process i+1). A process whose receipt before or after reads above 5 % busy, or that is
  invalid, is re-run ONCE at the end of its pass.
- **Void rule.** A build or lane process seen at any receipt, or started during a process, voids the whole pass;
  it is re-run from its start after the idle rule.
- **Launch** P-none: suspended, the affinity mask read back, resumed; no affinity call.
- **Validity** on every process: exit 0 and `void_steps` 0; `expect_pose: match` against the row's fixture and the
  hash equal to it; `workers` = W, `target_env` msvc, the armed flag the row expects; `drops_total` 0 and no
  disarmed ring traffic; the row's broadphase kind; TreeDiag `static_rebuilds 1, members 1, evictions 0` on tree
  rows and all zero on AllPairs rows; the class bench prints its band and SUMMARY; criterion prints both J kernels.
- **Hard stop** at 12:45: no process starts whose estimated end passes it. It was not reached.

**When and where.** The owner declared the machine quiet for 08:10–13:10 +03:00 on 2026-09-24 (as relayed in the
recording brief; `plan.md` opens with "The quiet window runs from 08:10 MSK"). Inside it, before any timed pass:
the builds 08:15–08:21, the untimed pose gate 08:24–08:45, the untimed rehearsals until 09:08. The timed window ran
09:09:49–12:27:54 (`raw/window_state.json`: `"status": "complete"`, `"exit": 0`, `"voided_processes": 0`,
`"binaries_after": "all match"`), on the owner's workstation (Ryzen 9 5900HS, 8C/16T; 16 logical CPUs in every
receipt; power scheme High performance at the start, `raw/window_state.json`). The blocks ended at (`progress.txt`):
P1 09:17:59, P2a 09:24:12, P2b 09:36:55, P3 09:56:05, P4 10:50:54, P5a 11:06:36, P5b 11:17:35, P5g 11:30:52,
P5d 11:38:52, P5e 11:49:00, P5f 11:59:20, P5c 12:27:54. `WINDOW_DONE` reads `exit 0` / `complete` / the reduction's
exit 0.

**Jolt was not run.** Jolt v5.6.0 is the only reference (owner ruling of 2026-09-21), and `analysis.md` § 5 reads
its window-3 cells (recomputed: 9.8282 / 2.5693 ms at W = 1 / 8), never re-run.

## Binaries

| key | exe (sha256 prefix, `bin/SHA256SUMS`) | commit | what | built from / provenance | `Compiling boyko-physics` path | log |
|---|---|---|---|---|---|---|
| b4db | `runner_4db26681.exe` `da674e39` | `4db26681` | trunk `integ/unified`: L9 C0–C3 merged, reuse dormant and switchable (`--contact-reuse`); the L10 lane base | exported tree `trees/4db26681/`, target `D:/wt/_targets/win6-4db26681`, cold, `Finished parity … in 1m 23s` | `(C:\Users\flint\AppData\Local\Temp\claude\D--claude-BoykoEngine\7dc37fc3-7b1e-46f1-89da-ccd0dba7c788\scratchpad\win6\trees\4db26681\crates\boyko_physics)` | `logs/build_4db26681.log` |
| c4db | `classes_4db26681.exe` `8f084c48` | `4db26681` | the L9 class bench `benches/narrowphase_classes.rs` (`--bench`) | the same tree and target dir, `--bench narrowphase_classes`, 17.13 s | the same path | `logs/build_4db26681_classes.log` |
| par | `runner_6dd1f916.exe` `228f3514` | `6dd1f916` | C3b's parent (tree query = RowWalk) | `trees/6dd1f916/`, target `D:/wt/_targets/win6-6dd1f916`, cold, 1m 22s | `(…\scratchpad\win6\trees\6dd1f916\crates\boyko_physics)` | `logs/build_6dd1f916.log` |
| tip | `runner_983480a9.exe` `f93e5fec` | `983480a9` | C3b's tip (tree query = LeafList); the trunk before L9 | `trees/983480a9/`, target `D:/wt/_targets/win6-983480a9`, cold, 1m 22s | `(…\scratchpad\win6\trees\983480a9\crates\boyko_physics)` | `logs/build_983480a9.log` |
| bpb | `bpbench_983480a9.exe` `e601fd46` | `983480a9` | `benches/broadphase.rs` (criterion), cargo profile `bench` | the same tree and target dir, `cargo bench --no-run -p boyko-physics --bench broadphase`, 56.67 s | the same path as tip | `logs/build_983480a9_broadphase.log` |
| g9p | `runner_0ca312bd.exe` `3993684b` | `0ca312bd` | window 5's `runner_parent.exe`: L11 C2 + the integration-line merge | copied from window 5, byte-identical (its `bin/SHA256SUMS`) | window 5's: `(…\d9f1cf70-4eed-44f3-9e33-2efbf030fc1a\scratchpad\win5\parent_tree\crates\boyko_physics)` | window 5's `logs/build_parent.log` |
| g9t | `runner_cbd86a65.exe` `9c7caff1` | `cbd86a65` | window 5's `runner_tip.exe`: the tree broadphase + L11 C0–C3 | copied from window 5, byte-identical | window 5's: `(D:\wt\lighttable\crates\boyko_physics)` | window 5's `logs/build_tip.log` |
| w4bt | `runner_w4btip.exe` `29dbd993` | `f8873aae` | window 4b's `runner_tip.exe`: L11 C2 | copied from window 4b, byte-identical (its `bin/SHA256SUMS`) | window 4b's: `(D:\wt\lighttable\crates\boyko_physics)` | window 4b's `logs/03_build_tip.log` |

**How the five new binaries were built** (`bin/COMMIT.txt`, `tools/build_one.sh`):
- Each commit was exported with `git -C D:/wt/lighttable archive <sha> | tar -x` into the scratch `win6/trees/<sha>/`,
  never built in a worktree. The runners: `cargo bench --no-run --profile parity -p boyko-physics --bench
  jolt_parity_pyramid` (`parity` inherits `release`: fat LTO, default CGU); one `CARGO_TARGET_DIR` per tree, cold,
  three builds in parallel.
- Toolchain rustc 1.98.1 (`48a229cea` 2026-09-01), `stable-x86_64-pc-windows-msvc`; `CARGO_INCREMENTAL=0`,
  `CARGO_BUILD_JOBS=5`, `TMP`/`TEMP` under `D:/wt/_targets/tmp`; `RUSTFLAGS` unset (`x86-64-v3` comes from each
  tree's `.cargo/config.toml`). Each log's `Compiling boyko-physics` line names its exported tree, so each binary
  compiled the commit it is named for.
- **One pruned lock.** `4db26681` does not track `Cargo.lock`. Each tree got `D:/wt/lighttable`'s lock, LF sha256
  `5f8de754e85aeebd7838ff01156cbfc5f815e89b8052a1f50dd8a7e090191bcb` — the lock the trunk tracks since `44d9ce22`
  ("build: track Cargo.lock", after `4db26681` on `integ/unified`; `git show 44d9ce22:Cargo.lock` has the same LF
  sha256). Cargo pruned the `boyko-symcensus` package, which these three trees do not have, and the lock after each
  build reads LF sha256 **`530cc386380c3b1315bd0158713ed33eec639a5d188a28493b976e3b6c6d9af0`**, identical in all three
  (the `lock copied` / `lock after build` lines of `logs/build_<sha>.log`). The difference is the `boyko-symcensus`
  package entry and its one line in a dependency list. `logs/pruned_Cargo.lock.txt` is that lock (renamed because the
  root `.gitignore` ignores `*.lock`).
- The three copied runners were re-checked against window 5's and window 4b's `SHA256SUMS` before the copy, and every
  binary was re-checked against `bin/SHA256SUMS` after the window (`"binaries_after": "all match"`).

## The idle rule, the receipts and the counts

**The idle-rule log** (`wait_log.txt`, the P0 idle rule as `tools/wait_idle6.ps1` implements it): 24 waits, one
before each of the 24 passes, 78 polls. Every wait reached idle. 22 did so in the minimum three polls; two took six,
because one poll each read a 10-s CPU of 5.51 % (the wait opened 09:56:06, before `P4-G9-p0`) and 5.27 % (10:56:07,
before `P5a-bridge2-p1`), both with `claude` processes on top, which reset the streak. **No poll saw a build process
or a process under `D:/wt/_targets` or `D:/wt/mq-*`.** The 10-s CPU over the 78 polls: median 1.11 %, range
0.35–5.51 %; process count 239–252. Before the window, one untimed probe at 08:53 read 5.88 % from two `claude`
processes and failed the rule (`plan.md` § 0; the probe's log is in the untimed `test/`, not kept).

**Counts** (`analysis.md` § 0). `raw/runs.jsonl` holds 553 records: 529 processes (24 warm-ups, 444 originals, 61
re-runs) and 24 pass markers. 0 passes were voided. 436 of the 444 slots are used, 53 of them by their re-run. 8
slots are dropped because both attempts were hot after the process, so these cells are K = 5: `C3b-TA-armed` par W1,
`C3b-TD-tree` tip W1, `G9-S16` g9p W1, `G9-RS` g9p W8, `G9-JAs-mid` g9p W2, `G9-R` g9p W1, `G9-JAs-mid` g9t W16,
`B2-JAs` w4bt W8. Every one of the 436 used processes passes every validity check above.

**Receipts.** 620 receipts: median 0.85 %, p90 5.13 %, max 17.21 %, 69 over 5 %. All 69 contaminations were flagged
by the receipt after the process; `claude.exe` topped 67 of them, and no build or lane process was present at any.
The during-process witness (`others_busy_pct`) over the used set: median 0.26 %, p95 3.87 %, max 5.07 %.

**The pose gate** (untimed, 08:24–08:45; `gate/gate_run.log`, `gate/gate_all.json`, `gate/gate_newcells.json`,
`tools/gate6.py`): 126 processes, 0 failing.
- Base, per runner (b4db, par, tip, g9p, g9t, w4bt): J `--cfg default` with `--broadphase tree` and `allpairs` at W
  1/8 reads `0x32d5e235342b4143`, rest `--cfg default` at W 1/8 reads `0xee2a67a98434919a` — 36 of 36 (w4bt has no
  `--broadphase` flag and ran its flagless default).
- L9's C0 fixtures (`docs/measurements/2026-09-23-l9-contact-reuse/fixtures/`, 600 steps) on b4db — 10 of 10 `match`.
- Every timed cell once with `--expect-pose` — 72 of 72 (50 in `gate_all.json`, 22 in `gate_newcells.json`); these
  runs recorded the four new fixtures and are the per-process time estimates (`gate/estimates_*.json`).
- Red controls: a 501-step J-A run against the 500-step J fixture exits 4 (`expect mismatch`, pose
  `0x9336b30a06a7d8af`) on all six runners — the gate can fail.
- The class bench `--bench` and the filtered criterion run exited 0 (`plan.md` § 3).

## What was reused from windows 3, 4, 4b and 5, and why

- **Window 5's two runners, g9p and g9t** (`0ca312bd`, `cbd86a65`), copied byte-identical. Why: G9's remainder is
  C3's gate, so it runs on the exact pair of binaries window 5 passed warm_apply on; the rerun warm_apply cell is also
  a same-binary bridge to window 5 (−0.2325 → −0.2299 ms, `analysis.md` § 6).
- **Window 4b's tip, w4bt** (`f8873aae`), copied byte-identical. Why: window 5's open "bridge 2"
  (`docs/measurements/2026-09-23-combined-tree-l11/analysis.md:234`) asks for window 4b's `runner_tip.exe`
  (`29dbd993`) interleaved with window 5's parent on `J-As` at W=8, to tell whether window 5's +11.5 % at W=8 was code
  or machine state.
- **Row spellings.** The G9 rows are window 4b's (`docs/measurements/2026-09-22-l11-solve-setup/rows.json`), the
  canary is P0's recipe (`tools/physics_parity/driver.py`), `C3b-TA-armed` is window 4's `T-A-tree-armed`, and
  `C3b-TD-tree` and the `G5-TD-*` rows spell window 5's `HL-D-*`.
- **Fixtures.** Five of the nine are byte-identical to files already committed, so their hashes tie this window to
  the earlier ones: `J500` = the P0 J pose (window 3's `pose.bin`, `eff361e1…`); `R1100`, `RS800` and `S16-300` =
  window 4b's gate poses (`gate/R_W1_parent.pose`, `R-S_W1_parent.pose`, `S16_W1_parent.pose`); `Son-J1000` = L9 C0's
  `fixtures/J-Son_W1.pose`, and to the L10 lane base's `b984a2af:docs/measurements/2026-09-23-l10-sleeping/fixtures/J-Son_W1.pose`
  (sha256 `eb8cc644…` for all three). The four new ones —
  `JAon500`, `Offp-J1000`, `RSOffp800`, `Ron1100` — were recorded by this window's gate from its first process and
  required of every other process and W.
- **The protocol block** of windows 3–5, with this window's additions (the claim required under both min–max and SE,
  the 30-poll idle cap, the hard stop at 12:45, blocks in priority order). Why: so that windows 3, 4, 4b, 5 and 6 are
  read with one statistic and one claim rule.
- **The tooling lineage.** `tools/window6_run.py` is adapted from window 5's `window5_run.py` (itself window 4b's and
  window 3's) and `tools/wait_idle6.ps1` from `wait_idle5.ps1`; both ancestors are committed under
  `docs/measurements/2026-09-23-combined-tree-l11/tools/`. `tools/lib/driver.py` is identical (LF-normalised) to
  `tools/physics_parity/driver.py` at `4db26681`, `983480a9` and `cbd86a65`; `tools/window_lib/pdhperf.py` is identical
  to window 5's.
- **Jolt v5.6.0's window-3 cells** as the only Jolt reference (owner ruling of 2026-09-21), read in `analysis.md` § 5
  and never re-run.

## Layout

| Path | What it is |
|---|---|
| `analysis.md` | The reduction the queue block and the four ruling lines are built from: selection and input checks, P1 (L10), P2 (L9 and the C4 decision), P3 (C3b's query cost, the block shift at W=1, the action table), P4 (G9's remainder), P5 (extras), the bridges, what cannot be claimed, the follow-up items, open points |
| `plan.md` | The window's plan: launch, protocol, binaries, the pose gate, each item's source, rows and bars, the estimated duration, the control-flow tests |
| `rows6.json` | The run list: the protocol block, the eight binaries (exe, commit, kind), the twelve blocks in priority order with their warm-up cells, the 32 rows (args, W, steps, window, armed, broadphase, pose ref) |
| `run_window.sh` | The launcher (resolves everything relative to its own directory): the driver, then the reduction, then `WINDOW_DONE` |
| `dryrun.txt` | The dry-run schedule (`run_window.sh --dry-run`) |
| `wait_log.txt`, `progress.txt`, `WINDOW_DONE` | The idle-rule poll log, the per-pass completion lines, the completion marker |
| `bin/SHA256SUMS`, `bin/COMMIT.txt` | The eight binaries' sha256; the commit, tree, command, target dir, `Compiling` line and lock of each. The executables are not in the tree |
| `logs/` | The five build logs (`build_4db26681.log`, `build_4db26681_classes.log`, `build_6dd1f916.log`, `build_983480a9.log`, `build_983480a9_broadphase.log`) and `pruned_Cargo.lock.txt` |
| `gate/` | The untimed pose gate: `gate_run.log` (its log), `gate_all.json` + `gate_newcells.json` (one record per gate process), `estimates_runner.json` + `estimates_bench.json` (the per-process time estimates), `fixtures/` (the nine `.pose` files every timed process asserted against, with `fixtures.json`: hash, recording process, binary, commit, args) |
| `raw/runs.jsonl` | One record per process (553): args, exit, receipts before/after, the during-process witness, the window mean, `summary` (pose / `expect_pose`, structural counters, TreeDiag, the frozen step), the binary's hash; the pass markers |
| `raw/<block>-p<n>_090949/<seq>_r<k>_<row>_<bin>_W<w>[_warmup\|_rerun]/` | One directory per process: `stdout.txt`, `stderr.txt`, the per-step `run.csv` (armed rows carry the zone columns); the P5c processes keep criterion's `criterion/<group>/<kernel>/<n>/{base,new}/` JSON instead |
| `raw/manifest_1790230189.json`, `raw/window_state.json`, `raw/window_log.txt`, `raw/driver_stdout.txt`, `raw/shell_log.txt`, `raw/passes_done.txt`, `raw/driver_done.txt` | The window's manifest, its start/end state (power scheme, voids, the binary re-check), its log, the driver's stdout, the launcher's log, the passes done, the driver's exit status |
| `raw/reduction.json`, `raw/tables.md`, `raw/reduce_stdout.txt` | The tester's untimed reduction (`tools/reduce6.py`), run by `run_window.sh` after the last pass |
| `analyst/reduction.json`, `analyst/tables.txt`, `analyst_run.log`, `analyst_argdiff.py` | The analyst's reduction (`tools/analyze6.py`), its tables (`analyst_run.log` is the same text, byte-identical), and the script that compared the `C3b-TA-armed` and `G5-TA-tree-a` command lines (`analysis.md` § 3.2) |
| `tools/` | `window6_run.py` (the driver), `wait_idle6.ps1` (the idle rule), `gate6.py` (the pose gate), `reduce6.py` (the tester's reduction), `analyze6.py` (the analyst's), `build_one.sh` (one runner build from an exported tree), `lib/driver.py`, `window_lib/pdhperf.py` |

**Pose files: one per distinct pose, not one per process.**
- **`raw/` keeps one `pose.bin` for each of the nine poses the window produced**, the first in path order. At record
  time the sha256 of all 513 `pose.bin` files in the scratch `raw/` was computed, and each of the nine hashes maps to
  exactly one `pose_hash` in `runs.jsonl`:

  | `pose_hash` | fixture | sha256 prefix | processes | kept in |
  |---|---|---|---|---|
  | `0xcc2a5400c66eecce` | Son-J1000 | `eb8cc644` | 18 | `raw/P1-L10-p0_090949/000_r-1_L10-Son_b4db_W1_warmup/` |
  | `0x3db47fae414b655c` | Offp-J1000 | `205bc544` | 12 | `raw/P1-L10-p0_090949/001_r0_L10-Offp_b4db_W1/` |
  | `0x32d5e235342b4143` | J500 | `eff361e1` | 303 | `raw/P2b-L9reuse-p0_090949/000_r-1_L9-JA-off_b4db_W8_warmup/` |
  | `0x30c5438bc6ad9ffa` | JAon500 | `e268d7a5` | 49 | `raw/P2b-L9reuse-p0_090949/002_r0_L9-JA-on_b4db_W1/` |
  | `0x87e561d20589d4a5` | R1100 | `c133a50e` | 46 | `raw/P4-G9-p0_090949/003_r0_G9-R_g9p_W1/` |
  | `0x2a2b7926a48aab00` | RS800 | `100975d7` | 45 | `raw/P4-G9-p0_090949/005_r0_G9-RS_g9p_W1/` |
  | `0x8877dbb1192e9b92` | S16-300 | `43d18f22` | 15 | `raw/P4-G9-p0_090949/007_r0_G9-S16_g9p_W1/` |
  | `0xb7f1e9e8f91f75ab` | RSOffp800 | `75cebc27` | 13 | `raw/P5b-L10spans-p0_090949/001_r0_L10-RS-Offp_b4db_W1/` |
  | `0xc8bbe34cf6a8afc6` | Ron1100 | `54b26c34` | 12 | `raw/P5e-L9reuseR-p0_090949/002_r0_L9-R-on_b4db_W1/` |

- The other 504 process directories wrote byte-identical files and those are not committed; each process's
  `pose_hash` and `expect_pose: "match"` remain in `runs.jsonl` and its `stdout.txt`. The nine kept files are also
  byte-identical to the nine `gate/fixtures/*.pose`.

**Not kept in the tree:**
- the executables (their hashes are in `bin/SHA256SUMS`);
- the exported trees (`win6/trees/<sha>/`) and their target dirs;
- the untimed rehearsals (`test/`: the full, cut-and-resume and criterion rehearsals, and the 08:53 idle probe);
- the gate's per-process directories (`gate/base/`, `cells/`, `l9fix/`, `red/`, `probe/`, `bench/`); their results are
  in `gate_run.log` and the two gate JSONs;
- the empty `driver.log`, `tools/__pycache__/`, and the 504 duplicate pose files above.

## How to re-run

1. **Build** into `bin/` and hash. For each of `4db26681`, `6dd1f916`, `983480a9`: `tools/build_one.sh <sha8>` exports
   the commit from `D:/wt/lighttable` into `trees/<sha8>/` (point `W` and the `git -C` path at this directory and at
   any checkout that has the commit), copies a lock in, and builds the parity runner. Use
   `logs/pruned_Cargo.lock.txt` (renamed to `Cargo.lock`) as the lock to reproduce these binaries' dependency set. The
   class bench and the criterion exe are the two further commands in `bin/COMMIT.txt`. Quote each build's
   `Compiling boyko-physics (path)` line. g9p, g9t and w4bt are copies: take them from windows 5 and 4b by hash, never
   rebuild them for a comparison against these rows. Never re-run Jolt.
2. **Gate the poses:** `python -B tools/gate6.py` writes `gate/`, including the fixtures and the red controls. It
   resolves this directory relative to its own location; its L9-fixture path points into `trees/4db26681/`.
3. **Run the window:** `bash run_window.sh` once, with no agent working (`--dry-run [--start HH:MM]` prints the
   schedule; `--test` is an untimed rehearsal under `test/`; `--resume` skips the passes in `raw/passes_done.txt`).
   It waits for the idle rule before each pass, voids and re-runs a pass on any build or lane process, writes `raw/`,
   then runs `tools/reduce6.py` and writes `WINDOW_DONE`.
4. **Reduce:** `python -B tools/reduce6.py` regenerates `raw/reduction.json` and `raw/tables.md`;
   `python -B tools/analyze6.py` writes `analyst/`. Its `WIN6` and `OLD` constants are the absolute scratch paths of
   the machine it ran on: point `WIN6` at this directory and the window-5 / 4b / 4 inputs at their records above.
5. **Quote nothing that is not in `raw/`.** The machine must be quiet: the 5-s receipts gate each process and the
   pass-level void rule gates build and lane processes; a burst inside a process is visible only through the
   during-process witness (`others_busy_pct`), which this window reports but does not gate.
