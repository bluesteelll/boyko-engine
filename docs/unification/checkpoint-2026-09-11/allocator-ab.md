# System heap vs mimalloc on the Jolt pyramid - arm, measurement, adjudication

- **Source:** workflow `alloc-cost-ab-mimalloc` (run `wf_30ba60cd-586`), tree D:/wt/joltab
- **Status:** COMPLETE and COMMITTED as d11962a9 (the register entry in docs/OPEN-QUESTIONS.md on merge/ke16-into-ecsnative). This file keeps the full agent reports.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Arm (developer)

PHASE 1 (ARM) — DONE. Tree: D:/wt/joltab @ ca582e72, branch merge/ke16-into-ecsnative. Not committed. No timing run.

## Edits (2 files, 36 insertions, nothing else touched; Cargo.lock is gitignored via `.gitignore:4 *.lock`, so it is not a tracked change)

1. `D:\wt\joltab\crates\boyko_physics\Cargo.toml`
   - `[dependencies]`: `mimalloc = { version = "0.1", optional = true }` after `boyko-scene`, with the boyko_ecs Phase X.E comment block (Cargo forbids optional dev-deps; nothing enables it by default; never in the default graph) plus two sentences naming the consumer and the diagnostic's purpose.
   - New `[features]` section: `default = []` and `bench-alloc = ["dep:mimalloc"]`, comment copied in boyko_ecs's form (Cargo.toml:83-87 there).
2. `D:\wt\joltab\crates\boyko_physics\benches\jolt_parity_pyramid.rs` (after the crate-level `#![allow]`, before the first `use`, as comparison_v2.rs does):
   ```rust
   #[cfg(feature = "bench-alloc")]
   #[global_allocator]
   static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;
   ```
   with the comparison_v2.rs:15-20 comment, extended by the specific signal sought (spawner allocates / stealer frees; gap growing with W => (b), flat => (a)).

## Both arms built (`cargo bench -p boyko-physics --bench jolt_parity_pyramid --no-run`)

| arm | feature | executable | size |
|---|---|---|---|
| sys | (none) | `D:/wt/_targets/joltab/release/deps/jolt_parity_pyramid-b5fb77d273df5024.exe` | 6,490,624 B |
| mi | `--features bench-alloc` | `D:/wt/_targets/joltab/release/deps/jolt_parity_pyramid-a48a84e07f2ed6b3.exe` | 6,800,384 B (+309,760) |

## Proof the feature applied (three independent checks)

- **Disassembly of the allocator shim (decisive):** `objdump -d --disassemble=<sym>` on each exe:
  - sys: `__rust_alloc` -> `jmp __rdl_alloc`; `__rust_dealloc` -> `jmp __rdl_dealloc` (Rust default = Windows heap).
  - mi: `__rust_alloc` -> `jmp mi_malloc_aligned`; `__rust_dealloc` -> `jmp mi_free`.
- **Symbol table:** `objdump -t` — sys arm 0 `mi_*` symbols; mi arm has `mi_malloc`, `mi_malloc_aligned`, `mi_free` (26 `mi_(malloc|free|...)` names via `strings`; sys arm 0). Symbol-table line counts 12,415 vs 13,513.
- **Size delta:** +309,760 B in the mi arm (libmimalloc-sys compiled: `cc 1.4.5`, `libmimalloc-sys 0.1.49`, `mimalloc 0.1.52`; both crates were already in Cargo.lock via boyko_ecs, so no new lock resolution).

## Gates (feature off unless stated)

- `cargo check --workspace --all-targets` — exit 0 (19.3 s).
- `cargo clippy -p boyko-physics --benches -- -D warnings` — exit 0; same with `--features bench-alloc` — exit 0.

## Machine identity for the receipt

- `rustc 1.98.1 (48a229cea 2026-09-01)`, commit-hash `48a229ceaefd4985c50990b14116b6d856af0985`, host `x86_64-pc-windows-gnu`, LLVM 22.1.8.
- `nproc` = 16 (available_parallelism; the bench itself should print its own value in the receipt).

## Notes for Phase 2

- The two exes are prebuilt; the timing driver must invoke them directly by path (not via `cargo bench`, which would re-check and put a build process inside the window — the "driver was its own noise" failure).
- Both exes were last written at 20:03:38 / 20:04:07; the later `cargo check`/`clippy` runs wrote only to `debug/` and did not touch them (verified by mtime).
- Neither exe has been executed.

---

## Measurement (tester)

{
 "arms": "Tree D:/wt/joltab @ ca582e72 (merge/ke16-into-ecsnative), not committed; the working-tree diff is still the 2 files / 36 insertions (git diff --stat checked this session). Both exes are prebuilt and ran directly by path, so no cargo ran inside the window. Their md5 hashes match the 20:08 identity record, so they are the same binaries the Phase-1 disassembly proof covered:\n- sys = D:/wt/_targets/joltab/release/deps/jolt_parity_pyramid-b5fb77d273df5024.exe (md5 8ad1ad8d589269b37c6f45ad0707bc16; __rust_alloc -> __rdl_alloc, the Windows process heap)\n- mi  = D:/wt/_targets/joltab/release/deps/jolt_parity_pyramid-a48a84e07f2ed6b3.exe (md5 8851043088851e6bf6a4c1ca95ced3f5; __rust_alloc -> mi_malloc_aligned, __rust_dealloc -> mi_free)\nBench: jolt_parity_pyramid/full_step/{1,8,16}. 1240 bodies, colored solve, parallel_solve and parallel_broadphase on for W>1. Criterion: 20 linear samples, warm-up 3 s, measurement 5 s, 420 or 630 iterations. The reported number is criterion's slope estimate. I also computed the median of the 20 per-sample per-iteration times from each run's sample.json.\nMachine: rustc 1.98.1, commit-hash 48a229ceaefd4985c50990b14116b6d856af0985, host x86_64-pc-windows-gnu, LLVM 22.1.8. std::thread::available_parallelism() = 16 (measured with a one-line rustc probe before the window; the bench does not print it). CPU: AMD Ryzen 9 5900HS, 8 cores / 16 logical.",
 "linked_proof": "VERDICT: mechanism (b) is not supported. mimalloc is not faster at any W, and the mi/sys gap does not grow with W. On this box the 2671 task-cell acquisitions per step cost nothing measurable when the global allocator is swapped. Cross-thread frees on the Windows system heap (process heap with LFH) are not the scaling limiter, so this A/B gives no performance reason to make wiring block.rs the priority fix. The Jolt residual lies elsewhere, most likely in the serial-work entry in docs/OPEN-QUESTIONS.md.\nPrediction, point by point:\n(1) W1 within the band: HELD. The ratio is 0.994 to 0.999 across all 4 fresh passes.\n(2) mimalloc faster at W8, and more so at W16: FAILED. The fresh median mi/sys is 0.998 at W1, 1.023 at W8 (mimalloc about 2% slower, inside the band) and 0.992 at W16. The killed 22:44 session's medians say the same: 0.975 at W1, 1.064 at W8, 0.993 at W16.\n(3) A flat, small gain if only (a) holds: not visible either. Mechanism (a) predicted about 3% at W8, but the W8 band is about ±5%, so (a) is neither confirmed nor excluded at this resolution. Only (b) is ruled out.\nScaling T(1)/T(W) is the same for both arms. Fresh medians: sys 1.79 at W8 and 1.62 at W16; mi 1.75 at W8 and 1.62 at W16. W16 is slower than W8 in both arms, so running 16 workers on 8 physical cores costs the same whatever the allocator.",
 "table": "FRESH SESSION (this tester, driver scratchpad/ab/drive.ps1 unchanged, invoked with -Passes 5,6,7,8; prewindow R1-R3 at 22:58:37 / 22:58:58 / 22:59:19 all build-procs=0; window 22:59:20-23:08:16). Each cell: slope ms [95% CI] / median-of-samples ms / iterations. Every row CLEAN, exit 0.\np5 W1  sys 19.970 [19.661,20.306] / 20.190 / 420 | mi 19.943 [19.715,20.210] / 20.219 / 420\np5 W8  sys 11.133 [10.999,11.259] / 11.131 / 630 | mi 11.429 [11.255,11.660] / 11.276 / 630\np5 W16 sys 12.283 [12.195,12.395] / 12.257 / 420 | mi 12.204 [12.146,12.280] / 12.185 / 630\np6 W1  sys 20.282 [20.057,20.541] / 20.465 / 420 | mi 20.235 [19.942,20.614] / 20.330 / 420\np6 W8  sys 11.171 [10.928,11.507] / 11.193 / 630 | mi 11.385 [11.225,11.602] / 11.439 / 630\np6 W16 sys 12.410 [12.236,12.640] / 12.689 / 420 | mi 12.248 [12.115,12.378] / 12.259 / 420\np7 W1  sys 19.942 [19.643,20.241] / 20.000 / 420 | mi 19.923 [19.608,20.275] / 20.180 / 420\np7 W8  sys 11.319 [11.074,11.621] / 11.238 / 630 | mi 10.755 [10.669,10.836] / 10.786 / 630\np7 W16 sys 11.968 [11.775,12.219] / 11.991 / 630 | mi 12.422 [12.213,12.607] / 12.476 / 420\np8 W1  sys 19.946 [19.619,20.319] / 20.198 / 420 | mi 19.817 [19.636,20.019] / 19.967 / 420\np8 W8  sys 11.015 [10.818,11.272] / 11.050 / 630 | mi 11.640 [11.350,11.865] / 11.340 / 630\np8 W16 sys 13.036 [12.832,13.209] / 12.975 / 420 | mi 12.924 [12.755,13.121] / 12.911 / 420\nFresh medians over 4 passes (slope): sys 19.958 / 11.152 / 12.347 ms; mi 19.933 / 11.407 / 12.335 ms (W1 / W8 / W16).\n\nKILLED 22:44 SESSION (a separate measurement, NOT stitched to the fresh one). Slope ms from results.csv; all receipt-CLEAN (build-procs=0 at R4-R27 of each invocation).\np1 W1 sys 20.018 / mi 22.021 | W8 11.717 / 11.898 | W16 12.821 / 12.739\np2 W1 sys 21.077 / mi 20.628 | W8 12.210 / 11.718 | W16 10.082 / 9.7511\np3 W1 sys 25.194 [23.151,28.047] / mi 22.250 | W8 14.517 / 20.045 [18.301,21.976] | W16 13.032 / 14.777\np4 W1 sys 19.845 / mi 19.276 | W8 10.392 / 11.564 | W16 11.224 / 11.129\np3 passes the build-process receipt but its data are visibly disturbed: W1 sys CI ±12%, W8 mi at 20 ms against about 11.5 ms everywhere else, and receipt load% 50/54 at 22:52:03. The receipt only counts build processes, so it cannot see this kind of disturbance. p1 and p2 come from an EARLIER version of drive.ps1: the file's mtime is 22:49, after p2 ended at 22:49:17. Their criterion output shows 'change:' lines, which means the baseline directory was shared rather than per-tag, and no crit/sample data survive for them. That is a provenance gap for p1 and p2.\n\nEARLIEST 20:09 SESSION (drive.sh, out_p*_*.txt, run.log, and the first 12 rows of results.csv under the old header): every row is DIRTY by its own receipts (build_procs=2-3 at several boundaries). Excluded, and not stitched.",
 "ratios": "mi/sys, taken back to back within each pass (slope | median-of-samples):\nFRESH p5: W1 0.999|1.001  W8 1.027|1.013  W16 0.994|0.994\nFRESH p6: W1 0.998|0.993  W8 1.019|1.022  W16 0.987|0.966\nFRESH p7: W1 0.999|1.009  W8 0.950|0.960  W16 1.038|1.040\nFRESH p8: W1 0.994|0.989  W8 1.057|1.026  W16 0.991|0.995\nFRESH median: W1 0.998|0.997  W8 1.023|1.018  W16 0.992|0.995\nBand, from the fresh pass-to-pass spread of the ratio: W1 ±0.3%, W8 0.950-1.057 (±5%), W16 0.987-1.038 (±2.6%). The first two passes disagreed only within the band, but 4 were taken anyway.\nKILLED SESSION p1..p4 (slope): W1 [1.100, 0.979, 0.883, 0.971] median 0.975; W8 [1.015, 0.960, 1.381, 1.113] median 1.064; W16 [0.994, 0.967, 1.134, 0.992] median 0.993.\nScaling T(1)/T(W), per pass (slope):\nFRESH sys: p5 1.79/1.63, p6 1.82/1.63, p7 1.76/1.67, p8 1.81/1.53 (W8/W16)\nFRESH mi : p5 1.74/1.63, p6 1.78/1.65, p7 1.85/1.60, p8 1.70/1.53\nFRESH median-of-medians: sys 1.790 at W8, 1.616 at W16; mi 1.747 at W8, 1.616 at W16.\nKILLED sys: p1 1.71/1.56, p2 1.73/2.09, p3 1.74/1.93, p4 1.91/1.77; mi: p1 1.85/1.73, p2 1.76/2.12, p3 1.11/1.51, p4 1.67/1.73.\nConfounder: the world is never reset, and warm-up is time-based, so the simulated-time window depends on the iteration count. In p5 and p7 the two arms at W16 ran different iteration counts (420 vs 630). In the passes where the counts match (p6 and p8) the W16 ratios are 0.987 and 0.991, which gives the same answer.",
 "receipts": "All in C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/ab/.\nFresh session: receipts.log lines 55-105, R1..R51 from 22:58:37.081 to 23:08:15.722. Every receipt shows build-procs=0 (the set checked covers cargo, rustc, rust-analyzer, rustup, ld/lld/link, cl, gcc/clang, cmake, ninja, msbuild, dxc, as), with load% between 0 and 40.\nPer-row receipt pairs: p5 R4/R5 .. R14/R15, p6 R16/R17 .. R26/R27, p7 R28/R29 .. R38/R39, p8 R40/R41 .. R50/R51.\nReceipt IDs restart at R1 on every driver invocation, so 'R4' appears three times in receipts.log. Tell them apart by timestamp: 22:44 is p1-2, 22:50 is p3-4, 22:58 is p5-8.\nRaw data: results.csv (last 24 rows), crit/p5..p8_*/.../new/sample.json, log/p5..p8_*.out/.err, console_p5-8.txt ('PREWINDOW OK at 22:59:20', 'DONE at 23:08:16', exit 0). analyze.py recomputes every number above.\nBefore the window: no jolt_parity_pyramid process was running (tasklist). A 5 s CPU census showed only claude.exe (about 3.3 CPU-s in total), Taskmgr and steam idle.\nDuring the window, my only activity was a Monitor loop (grep every 15 s) and one wait loop (grep every 20 s). No builds ran.\nThe rustc probe (ap.exe) was compiled at about 22:58:10, before the prewindow, and the three prewindow receipts are clean.",
 "open": [
  "The allocation COUNT under mimalloc was NOT re-taken. The census harness (alloc_frame_census.rs) exists only on D:/wt/ecsnative, and the standing rule forbids touching that tree. This tree's boyko_physics/tests holds only colored_parallel_alloc_o6.rs, colored_solve_zero_alloc_o5.rs and soft_colored_sp4_alloc.rs. The count is allocator-independent by construction; the census's 2671 mean / 2724 max per step stands as measured there.",
  "What this null covers: THIS machine's Windows process heap (the Rust default path, __rdl_alloc -> HeapAlloc with LFH) at this resolution (W8 band ±5%, median of 4). It shows the system heap handles the spawner-allocates / stealer-frees pattern about as well as mimalloc. It does not show that cross-thread frees are free in principle, and it says nothing about a Linux glibc arm, which was not measured.",
  "Mechanism (a), the roughly 3% predicted cost of the malloc/free instructions, is below the W8 resolution and neither confirmed nor excluded. Wiring block.rs may still be justified on principle-5 (minimum allocations) grounds, but not as a scaling fix on this evidence.",
  "The pyramid world is never reset between criterion iterations and warm-up is time-based. So arms that ran different iteration counts (p5 and p7 at W16: 420 vs 630) measured different simulated-time windows. A per-iteration world reset, or a fixed step-count warm-up, would remove this confound for future A/Bs.",
  "Both arms get slower from W8 to W16 (T1/T16 1.62 vs T1/T8 about 1.77) on 8 physical / 16 logical cores. The allocator does not change this, so the W16 regression (SMT or oversubscription) is a separate open question.",
  "In the killed session, p1 and p2 were produced by an earlier, unrecorded version of drive.ps1: it had a shared criterion baseline directory, as the 'change:' lines prove, and their crit sample data are missing. p3 is receipt-CLEAN but data-disturbed, a disturbance the build-process-only receipt cannot see; a CPU-load threshold in the receipt would catch it. Neither is used for the verdict beyond agreeing with it.",
  "The diagnostic edit (bench-alloc feature + cfg'd global_allocator) remains uncommitted in D:/wt/joltab; keep it or revert it at the orchestrator's call."
 ]
}

---

## Adjudication

**Verdict: VACUOUS as a test of the 2671. Correct as a bound on the binary it measured. The A/B could not have seen mechanism (b), because neither arm contains the 2671 stealer-freed cells.**

**1. Receipts and interleaving**
- **Receipts: PASS.** I checked `receipts.log` myself. The three prewindow receipts are clean and about 21 s apart (22:58:37, 22:58:58, 22:59:19). R4 to R51 bracket every row and all show `build-procs=0`, with load between 0 and 40 %. All 24 rows are CLEAN and exited 0.
- **Toolchain: PASS.** rustc 1.98.1, commit 48a229ceaefd4985c50990b14116b6d856af0985, x86_64-pc-windows-gnu. `available_parallelism` = 16, re-read from `ap.exe`.
- **Interleaving flaw.** Arms alternate by W, but the order is fixed: `sys` always runs before `mi` (drive.ps1's `@('sys','mi')`), so an arm-order bias would count against mimalloc. My one re-run tested this: W8, order reversed as mi, sys, sys, mi, in one pass (p9).
  - Prewindow R1 to R3 clean at 23:16:37, 23:16:59 and 23:17:20.
  - Every row CLEAN with build-procs=0, receipts R4 to R11, window 23:17:21 to 23:18:38.
  - Slope in ms: mi 11.731, sys 12.226, sys 11.728, mi 12.348. That gives mi/sys = **1.005** on slope and **0.974** on median-of-samples.
  - The tester's W8 figure of +2.3 % for mimalloc does not reproduce. Its sign flips when the order flips, so it is noise inside the ±5 % band.
  - My cell has the same iteration-count confound: mi ran 420 iterations, sys 630.

**2. Did the mimalloc arm link mimalloc? CONFIRMED from disassembly.**
- The md5 hashes match the identity record.
- In the mi exe: `__rust_alloc` jumps to `mi_malloc_aligned`, `__rust_dealloc` to `mi_free`, `__rust_realloc` to `mi_realloc_aligned`, and `__rust_alloc_zeroed` to `mi_zalloc_aligned`. It has 246 `mi_` symbols.
- In the sys exe: all four jump to `__rdl_*`, and it has 0 `mi_` symbols.

**3. The census and the A/B ran on different threadpools.**
- The census tree is D:/wt/ecsnative @ ad0ebea4. The A/B tree is joltab @ ca582e72. `d51b4ced` ("stage 3b lands the per-scope block") is an ancestor of ca582e72 and **not** of ad0ebea4.
- So `block.rs` is already wired in both A/B binaries. They link `libboyko_threadpool-cdde635a2642dbec.rlib`, whose `.d` lists `task/{mod,detached,scoped}.rs` (the Stage 3b layout). `ScopeBlock` is present in the exe.
- On ad0ebea4, `task.rs` still says "One alloc per spawn and one dealloc per task" and `block.rs` still says "wired to NOTHING".
- In this tree, every physics fan-out is scoped: `pool.scope` + `scope.spawn`/`spawn_batch` in `solver/colored.rs:2867`, `resources.rs:1957/2049`, `soft/colored.rs:1122` and ECS `par_chunk`/`par_iter`. Scoped spawns go through `ScopeBlock::emplace`. Only `Task::new_detached` still allocates a cell per task.

**4. Allocation census on the A/B's own rlibs**
I built a probe with rustc straight against the prebuilt joltab rlibs, so no tree was touched. It runs the bench's scene verbatim, with a System wrapper that tags each block with the thread that allocated it. 400 warm steps, then 200 counted:

| W | allocs/step (max) | bytes/step | cross-thread frees/step | reallocs |
|---|---|---|---|---|
| 1 | 2.1 (3) | 4.5 KB | 0.1 | 0 |
| 8 | 331.6 (339) | 1.27 MB | **0.1** | 0 |
| 16 | 331.6 (339) | 1.27 MB | **0.1** | 0 |

The ecsnative census measured 2671 per step. Here it is 332, and cross-thread frees are about zero, so mechanism (b) had nothing to act on. The tester's open item "the census's 2671 stands" is true of ad0ebea4 and false of the binaries that were timed. The count does not depend on the allocator, but it does depend on the tree. Bytes went up (856 KB to 1.27 MB): each scope gets at least one 4 KiB chunk, which is allocated and freed on the same thread, not reused across scopes.

**5. Does the gap grow with W?**
- Fresh medians of mi/sys: W1 0.998, W8 1.023, W16 0.992. My reversed W8 cell: 1.005 on slope, 0.974 on median.
- The band is ±0.3 % at W1, ±5 % at W8 and ±2.6 % at W16.
- The gap is **absent**. It does not grow with W, and it is not a flat mimalloc gain either.
- Prediction point (1), W1 within band, held. It was trivially expected: 2 allocations per step at W1.

**6. What the system allocator costs at W=8**
- Measured bound: at most the W8 band, ±5 % or about 0.56 ms of 11.15 ms, taken relative to mimalloc.
- Estimate: 332 alloc/free pairs at about 50 to 100 ns per operation (the tester's figure, not measured here) is 33 to 66 µs, or **0.3 to 0.6 % of the step**. That is below resolution.
- The ecsnative configuration, with its 2671 cells per step, was **never timed** under either allocator.

**7. Would wiring block.rs buy more or less than mimalloc?**
- It is already wired in both arms, so this A/B measured what is left *after* that fix.
- In principle, on a pre-3b tree the block buys **at least as much as** mimalloc. It removes about 88 % of the operations and all of the cross-thread frees, where mimalloc can only make them cheaper.
- On the current tree, the next step of the same kind would be retaining chunks across scopes. It is predicted to buy **less** than mimalloc did here, which was nothing measurable: it could remove at most about 0.3 to 0.6 %.
- If the historical question matters, the right test is pre-3b vs post-3b on this harness. That is the "KE16 contribution A/B owed" item, ⚠(2) in the ca582e72 entry. A mimalloc swap on a post-3b tree is not that test.

**8. Jolt next-fix ordering in docs/OPEN-QUESTIONS.md: UNCHANGED.**
- The allocator is removed as a candidate on the shipped tree.
- block.rs is not a next fix: it landed as `d51b4ced` and is already inside the 5.2× to 2.5× re-take.
- The owed measurement is still the per-stage wall time at W1 vs W8: narrowphase, broadphase, solve, and the inline-colour share. It is set against the Amdahl serial fraction of 0.43 to 0.53.
- This A/B's scaling agrees with that entry: sys T1/T8 = 1.79.

**Files**
- My re-run cell: `C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/adj/` holds `adj_drive.ps1`, `adj_receipts.log`, `adj_results.csv`, `crit/` and `log/`.
- The census probe: `probe.rs`, `scene.rs` and `probe.exe` in the same directory.
- The joltab tree is unchanged by me. The two-file diagnostic is still there, uncommitted.

**Quotable:** "The mimalloc A/B found nothing because there was nothing to find: the tree it timed already ships `block.rs`, so its W=8 step makes 332 same-thread allocations and 0.1 cross-thread frees, not 2671. The allocator costs at most ±5 %, estimated at ~0.5 %, and the Jolt residual is still owed to the serial-stage timing."
