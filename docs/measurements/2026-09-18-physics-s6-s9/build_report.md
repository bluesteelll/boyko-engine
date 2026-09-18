# MEASUREMENT-QUEUE §6–§9: build report (preparation only, nothing timed)

Status: **PREPARED.** All 6 arms are built. The rebuild proof shows nothing compiles, all 6 arms have the same ISA flags, and the structural receipts are recorded.
**Nothing was timed.** There are no criterion measurements. The only runs so far are criterion TEST-mode smoke runs (one iteration each) and the deterministic count probes. The §0 idle-machine receipt belongs to the timing stage.

Scratch root: `docs/measurements/2026-09-18-physics-s6-s9/` (below: `MQ/`)

## 1. Worktrees (all new: no path existed before)

| entry/arm | commit | worktree | target dir | state |
|---|---|---|---|---|
| s6 A | d5782d43 | D:/wt/mq-d5782d43 | D:/wt/_targets/mq-d5782d43 | clean at commit |
| s6 B | b74f7ee8 | D:/wt/mq-b74f7ee8 | D:/wt/_targets/mq-b74f7ee8 | clean at commit |
| s7 A + s8 A | d552be05 | D:/wt/mq-d552be05 | D:/wt/_targets/mq-d552be05 | commit + s8 port (` M crates/boyko_physics/Cargo.toml`, `?? crates/boyko_physics/benches/sleeping_pipeline.rs`) |
| s7 B + s8 B | a56007ab | D:/wt/mq-a56007ab | D:/wt/_targets/mq-a56007ab | clean at commit |
| s9 A | 08fe7b9f | D:/wt/mq-08fe7b9f | D:/wt/_targets/mq-08fe7b9f | clean at commit |
| s9 B | 8d656ad8 | D:/wt/mq-8d656ad8 | D:/wt/_targets/mq-8d656ad8 | clean at commit |
| s9 probe (never time) | 08fe7b9f / 8d656ad8 | D:/wt/mq-probe-08fe7b9f, D:/wt/mq-probe-8d656ad8 | D:/wt/_targets/mq-probe-<sha8> | commit + probe (2 bench files modified) |

Parentage checked with `git log`: b74f7ee8's parent is d5782d43, a56007ab's parent is d552be05 (d552be05 itself is a merge: b74f7ee8 + fc7eb127), and 8d656ad8's parent is 08fe7b9f.
Bench-source deltas between arms: `sleeping_pipeline.rs` differs between 08fe7b9f and 8d656ad8 in doc comments only. `soft_step_sp2.rs` differs between d552be05 and a56007ab (the §7 R3 fixture migration). `jolt_parity_pyramid.rs` is the same in both arms of every entry.

### s8 arm-A port: `MQ/s8_armA_port.diff`
- The `Cargo.toml` hunk is a56007ab's own `[[bench]] sleeping_pipeline` hunk, applied with `git apply` (it applied cleanly).
- `benches/sleeping_pipeline.rs` is `git show a56007ab:…`. Diffed against a56007ab's file, exactly one line differs: line 249, where `pile.world.resource::<IslandSleep>().contact_wakes()` becomes `0`. It compiles with one expected warning, `unused variable: pile`.
- s7's three bench exes on d552be05 (jolt_parity_pyramid, row_identity_churn, soft_step_sp2) were built BEFORE the port. After the port they rebuilt as `fresh`, with an unchanged sha256. So s7 arm A is byte-identical to clean d552be05.
- Procedural slip, now undone: I ran `git add -N` on the ported file by accident, which created an index entry. I used it to write the diff, then removed it with `git rm --cached` (the file was kept). The index now matches HEAD and the file is untracked again.

## 2. Executables: `MQ/exes.json` (full paths + full sha256)

All are `D:/wt/_targets/mq-<sha8>/release/deps/<bench>-<hash>.exe`, bench profile:

| arm | jolt_parity_pyramid-43b10ef74d0e3f27 | row_identity_churn-a5d5ed5921338d69 | soft_step_sp2-5eb4eb9741068bd6 | sleeping_pipeline-b79088505f0ef46b |
|---|---|---|---|---|
| d5782d43 | 9dff38a51a5e5dc5 | (does not exist at A; §6 "B only") | — | — |
| b74f7ee8 | 3bbbd97325e1aa21 | bf089a1f9cc50392 | — | — |
| d552be05 | aa70f3e266f0055e | 6e37d7db2cb3110b | 086758fe0bef689f | e9ec0a2eb67ae10b (ported) |
| a56007ab | 662a96852d1d44a2 | 48e71cca5e94674b | cc01bda69891e974 | ec3ff6ee8ee16db1 |
| 08fe7b9f | c78a9ffe77a9bf0c | — | — | fe421ad35d6c893b |
| 8d656ad8 | 80142a730beaa1c3 | — | — | 225b355d7bb1ba37 |

(These are sha256 prefixes. Filenames are identical across arms, so the directory is what identifies the arm.)

Traps for the timing stage:
- Invoke as `cd <worktree>; CARGO_TARGET_DIR=<target dir> <exe> --bench [filter]`. **Without `--bench` criterion runs TEST mode**: one iteration, and nothing is timed.
- Do not pass `--test-threads` to these criterion binaries.
- d552be05 and a56007ab also contain `sleeping_pipeline-892ed66c006bb282.exe`. That is the release-profile (fat LTO) TEST build left behind by the §8 receipt. **Do not time it.** `D:/wt/_targets/mq-probe-*` holds probe binaries: **never time these either.**
- The verbose all-bench build (§4 below) also built the other bench targets into each dir. They are unused.
- Recording W: set `CRITERION_DEBUG=1`, and criterion 0.5.1 prints `Completed <W> iterations in …` after warm-up. Per-sample iteration counts are in `<target>/criterion/<group>/<bench>/new/sample.json`. Both are needed for §9 R2 (see 5b). As far as I can tell from criterion 0.5.1's source, `CRITERION_DEBUG` only adds print statements.

## 3. No-compile proof (re-run after every build, receipt run and smoke run)

I re-ran both build commands per arm: the JSON `--bench` list and the verbose all-bench `cargo bench -p boyko-physics --no-run -v`.

| arm | JSON re-run | verbose re-run |
|---|---|---|
| all 6 | `Compiling`=0, `Finished … in 0.70–0.77s`, rc=0, every needed exe `fresh:true`, sha256 = first build | `Compiling`=0, rustc `Running`=0, `Fresh`=84, `Finished … in 0.19–0.22s`, rc=0 |

Can this check go red? Yes: the d552be05 post-port run printed 1 `Compiling` line (the new sleeping_pipeline bench), and every first build printed 84 of them. Logs: `MQ/logs/<sha8>.proof.{json,stderr}` and `MQ/logs/<sha8>.verbose-proof.stderr`.

## 4. ISA parity: all arms identical, no RUSTFLAGS

- Environment on every arm (first line of `MQ/logs/<sha8>.verbose.stderr`): `RUSTFLAGS=[unset] CARGO_ENCODED_RUSTFLAGS=[unset] toolchain=stable-x86_64-pc-windows-msvc`.
- Toolchain: rustc 1.98.1 (48a229cea 2026-09-01), host x86_64-pc-windows-msvc.
- `.cargo/config.toml` is byte-identical (md5 22321cda…) on all 6 arms and sets `[target.x86_64-pc-windows-msvc] rustflags = ["-C","target-cpu=x86-64-v3"]`.
- The verbose rustc line for `--crate-name boyko_physics` is the same on each arm: `-C opt-level=3 -C embed-bitcode=no -C codegen-units=1 … --test … -C target-cpu=x86-64-v3`. target-cpu appears once and target-feature zero times. This holds on every arm.
- All rustc lines in each verbose log carry `-C target-cpu=x86-64-v3` (d5782d43 13/13, b74f7ee8 13/13, d552be05 12/12, a56007ab 12/12, 08fe7b9f 14/14, 8d656ad8 14/14), and none carries another cpu/feature flag.
- Direct evidence for the timed artifacts themselves: every cargo fingerprint in each target dir records `rustflags = ["-C","target-cpu=x86-64-v3"]`. That is 128 / 129 / 130 / 130 / 130 / 130 units, with no other value. This covers the bench-profile lib `boyko-physics-a83d8b8d1e51707e` and each timed bench unit. Had RUSTFLAGS been set, its value would replace this field.

## 5. Structural receipts (counts, not load-sensitive)

### 5a. §8 R3/R4: frozen-arm receipts

Source: `cargo test --release -p boyko-physics --bench sleeping_pipeline`, same target dirs, rc=0 on both, 3/3 `Success`. Logs: `MQ/logs/<sha8>.s8receipt.out`.

| tree | froze at step | manifolds | islands | contact_wakes |
|---|---|---|---|---|
| A: d552be05 + port | 61 | 8555 | 1 | 0 (**stubbed**: the helper body is `0`, so this reading cannot be non-zero on A) |
| B: a56007ab | 61 | 8555 | 1 | 0 (real counter) |

- **R3 holds:** freeze step, manifold count and island count are the same on both trees, so the row is comparable.
- **R4 holds:** the manifold count is 8555, not 0, so the row is not void.
- The same receipts come from the bench-profile timed exes themselves: I ran each timed exe once in test mode (`MQ/logs/<sha8>.smoke.out`). All exes on all arms exited with rc=0.
- The s9 trees print the same frozen receipt (61 / 8555 / 1 / 0). The probe shows 34220 points on that frozen pile.

### 5b. §9 R2: live contact points over the timed steps

Probe diff: `MQ/s9_probe.diff`. The 8d656ad8 copy (`MQ/s9_probe_8d656ad8.diff`) has identical +/- content.
- The probe replaces `criterion_main!` in `sleeping_pipeline.rs` and `jolt_parity_pyramid.rs` with a `main`. That `main` builds the bench's own scenes in the bench's order, steps them, and prints per step `Σ manifold.count` over `Manifolds::manifolds()` plus the count of manifolds with `count > 0`.
- It was built with the bench profile into `D:/wt/_targets/mq-probe-<sha8>`.
- Raw series: `MQ/probe/sp_<sha8>.csv` (steps 1..5000) and `MQ/probe/jolt_<sha8>.csv` (steps 1..3000, full_step/1 and /4). Window table: `MQ/probe/r2_table.txt`. Calculator: `MQ/probe/r2_window.py`.

Evidence that the probe counts the right thing:
- At step 600, `pyramid_sleeping_off` reads A = 5223 manifolds / 14605 points and B = 6671 / 22975. These equal §9's own A7-R1 census figures exactly, so this pile is A7-R1's trajectory.
- A fresh second pile replayed the first 300 steps identically on both benches and both arms.
- `full_step/4` produces the same series as `full_step/1` for 3000 steps on both arms.
- `pyramid_awake_sleeping_on` produces the same series as `full_step/1` for 3000 steps on both arms. That is expected: with a zero threshold it never latches, so it runs Jolt's scene.

**Timed-step model** (criterion 0.5.1 source, `routine.rs`: the closure is called once per sample and once per warm-up batch):
- `sleeping_pipeline/*`: ONE persistent pile. The timed window is steps `[31+W, 30+W+N]`, where W = the warm-up count (2^k−1) and N = the measured iterations.
- `jolt_parity_pyramid/full_step/w`: **each sample rebuilds the world**, steps 20 untimed, then times steps 21..20+i_j. Linear sampling: i_j = j·d. Flat sampling: i_j = m.

| row | window | A pts/step | B pts/step | B/A |
|---|---|---|---|---|
| pyramid_sleeping_off | W=127, N=210 (158–367) | 14981 | 22958 | 1.532 |
| pyramid_sleeping_off | W=255, N=420 (286–705) | 14734 | 22972 | 1.559 |
| pyramid_sleeping_off | W=511, N=420 (542–961) | 14554 | 22968 | 1.578 |
| pyramid_sleeping_off | W=1023, N=840 (1054–1893) | 14423 | 22915 | 1.589 |
| pyramid_awake_sleeping_on | W=127…1023, N=100…840 | 13322–13640 | 17113–17271 | 1.261–1.285 |
| full_step/1 = /4 | linear d=1 (steps 21–40) | 16338 | 17359 | 1.063 |
| full_step/1 = /4 | linear d=2 (21–60) | 15533 | 17499 | 1.127 |
| full_step/1 = /4 | linear d=4 (21–100) | 14372 | 17483 | 1.216 |
| full_step/1 = /4 | linear d=8 (21–180) | 13575 | 17463 | 1.286 |
| full_step/1 = /4 | linear d=16 (21–340) | 13362 | 17344 | 1.298 |
| full_step/1 = /4 | flat m=5 (21–25) | 17109 | 17296 | 1.011 |

**FLAG R2-1 (full_step):** the full_step row ratio depends on criterion's own iteration count. Because every sample restarts at step 21, where the two arms barely differ, the ratio runs from 1.01 to 1.30 depending on d or m. A and B will probably pick different d, since B's steps carry more rows, and then the two arms time different step windows. The ratio that applies is per arm: take each arm's own N and its `sample.json` iters, and compute with `python MQ/probe/r2_window.py MQ jolt <sha8> full_step/1 linear <d>`.

**FLAG R2-2 (sleeping_off):** the ratio drifts from 1.53 to 1.59 with the window. A's count keeps falling as its pile creeps, while B's stays near 22.9k. Compute it from each arm's own W and N: `python MQ/probe/r2_window.py MQ sp <sha8> pyramid_sleeping_off <W> <N>`.

## 6. Deviations and what was not done

- **Separate probe worktrees** (`D:/wt/mq-probe-<sha8>`) instead of editing the timed worktrees. Editing a timed tree and then reverting the edit would leave a newer source mtime behind, and the next cargo command on the timed target dir would recompile. That is the §1 "a timed run must never compile" trap. The probe target dirs are separate, as specified.
- The verbose ISA build was the literal `cargo bench -p boyko-physics --no-run -v`. By the time it ran, the timed units had already been built, so they showed as `Fresh`. The rustc lines therefore come from the units that command compiled for the first time: the `boyko_physics --test` unit and the unused benches. The timed units themselves are covered by the fingerprint census in §4.
- Not run, because it was not assigned: §7 R4 (`alloc_frame_census` / `alloc_frame_attribution` on d552be05 vs a56007ab). It is structural and still outstanding before §7's rules can be applied.
- Disk: the 8 new target dirs total about 3.1 GB, all on D:, which has 44 GB free. Nothing was written to C: except scratch.
- Nothing was committed, stashed, reset or deleted.
