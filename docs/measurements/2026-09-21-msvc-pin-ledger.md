# The msvc pin ledger — every number blessed under windows-gnu, re-read on the msvc gate host (2026-09-21)

Rung AH (4) of the unification plan (`docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`,
"AH (4): the re-bless procedure", on the plan's own tree). The procedure: run each pinned test on
msvc; for every pin that moves, run the same test under `stable-x86_64-pc-windows-gnu`; make ONE
commit that lists every moved pin as (test, gnu value, msvc value, reason); a host difference nobody
can explain stays RED and is escalated. This file is that list.

**Why this file and not `docs/MEASUREMENT-QUEUE.md` §0.** The queue's own §0 rules that structural
verdicts — "counts, a census result" — are not load-sensitive and never belong in the queue; every
row below is a census pin or a receipt predicate, none is a wall clock. And AH (2) classified
`docs/measurements/` as HISTORY: a dated record there keeps its toolchain spelling forever, which is
exactly the property a gnu-then / msvc-now table needs — nobody may later "re-spell" its gnu column.
It is one file rather than a `<date>-<topic>/` directory because it ships no raw data: the receipts
it reads are already in the tree (the census headers, `docs/threadpool/receipts/`), and the tester's
run logs were session scratch, not a gate's input.

## Host, tree, and how the two columns were read

- Tree `D:/wt/uploadleak`, branch `chore/ah-msvc-miri-receipts`, cut from `54e186d9`; the runs
  below are on `85b2aa49` (AH (2)+(3) applied; this file's commit adds no code).
- msvc: `stable-x86_64-pc-windows-msvc` rustc 1.98.1 (48a229cea 2026-09-01); Miri
  `nightly-x86_64-pc-windows-msvc`, miri 0.1.0 (a36d05efab 2026-09-09). Build prefix per RK-11 /
  RK-18: `CARGO_TARGET_DIR=D:/wt/_targets/kernel-msvc`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`,
  `TMP`/`TEMP` = `D:/wt/_targets/tmp`, never `RUSTFLAGS`.
- **"gnu value"** = the value at the last commit whose measurement the file itself dates under
  `stable-x86_64-pc-windows-gnu` rustc 1.98.1: `d5782d43` (2026-09-13) for the physics censuses
  (measured 2026-09-10/11 at `ca582e72` / `d11962a9`); `d51b4ced` (2026-09-08, miri 8925ea358a
  2026-08-20) for the Miri receipts; `7a315058` for the profile-axis census (measured over the gnu
  image). **"msvc value"** = HEAD.
- **Counting rule.** One row = one pin, or one group of pins asserted as a unit. "Moved" = the value
  asserted at HEAD differs from the gnu-blessed value. "Attributed" = a commit in `git log` whose
  message names the cause, or the pin's own header does. A move that is neither is a host
  difference nobody can explain: it stays RED here and is escalated.
- **The msvc switch:** recipes 2026-09-10 (`97bcf826`); rustup default host 2026-09-17; Miri
  checker 2026-09-21 (`85b2aa49`).

## Result

**28 pins, 6 moved, 6 attributed, 0 unattributed.** No pin is RED for a host reason, so no gnu
comparison run was owed under step 2 and none was made (`D:/wt/_targets/joltab-gnu` was never
created). The only measured envelopes that moved are the S1c release and debug pins (rows 12–13),
and every one of their moves is a physics code commit whose message names the cause; the host
control for that scene is zero in both profiles (row 12). Everything else reads today, on msvc,
what it read under gnu.

Pins this commit does NOT touch (msvc-now equals gnu-then): rows 1–11, 14–17, 20–22, 24, 26–27.
Pins are replaced, never widened (P9) — and none is replaced here, because every replacement had
already been made by the commit that caused it.

## A. `crates/boyko_physics/tests/alloc_frame_census.rs` (UG-03, `harness = false`, 1 test)

Pin shape: `scope` range, `chunk` range, `dispatch_max`, `other_per_frame`, `realloc_sum`;
`total_max = dispatch_max + other_per_frame + 2 × workers`. msvc today: release
`GATE: GREEN — 12 scenes`, debug `GATE: GREEN — 12 scenes`, `running 1 test` / `1 passed` each.

| # | pin | gnu value (`d5782d43`) | msvc value (HEAD) | moved | reason | msvc-now measured, mean / MAX (gnu-then in brackets) |
|---|---|---|---|---|---|---|
| 1 | S0, 0 systems | 0..=0 / 0..=0 / 0 | same | no | unchanged | rel 0.000/0, dbg 0.000/0 [0.000/0] |
| 2 | S0, 1 system — `app(1, 2)` | 1 / 1 / 3 (total 7) | same | no | unchanged | rel 2.016/3, dbg 2.016/3 [2.016/3] |
| 3 | S0, 2 systems | 1 / 1 / 3 | same | no | unchanged | rel 2.035/3, dbg 2.031/3 [2.031..2.043 / 3..6] |
| 4 | S0, 4 systems | 1 / 1 / 3 | same | no | unchanged | rel 2.062/3, dbg 2.062/3 [2.066 / 3..4] |
| 5 | S0, 8 systems | 1 / 1 / 3 | same | no | unchanged | rel 2.125/3, dbg 2.125/3 [2.125/3] |
| 6 | S0, 16 systems | 1 / 1 / 3 | same | no | unchanged | rel 2.254/3, dbg 2.254/3 [2.254/3] |
| 7 | S0b — `app(2, 2)` | 2 / 2 / 5 (total 9) | same | no | unchanged | rel 4.125/5, dbg 4.125/5 [4.125..4.133 / 5..6] |
| 8 | S2 — `app(2, 4)`, `realloc_sum 0` | 2 / 2 / 5, realloc 0 (total 13) | same | no | unchanged (pinned post-EM2′ `0afcbd7d`, already so at `d5782d43`) | rel 4.039/6 (OTHER 2 first-touch, ≤ 8), dbg 4.031/5, realloc 0 both [4.031/5, realloc 0] |
| 9 | S3 — `app(1, 2)` | 1 / 1 / 3 | same | no | unchanged | rel 2.062/3, dbg 2.062/3 [2.062 / 3..4] |
| 10 | S1a — `app(1, 1)` | 1 / 1 / 3 (total 5) | same | no | unchanged | rel 2.109/3, dbg 2.109/3 [2.109/3] |
| 11 | S1b — `app(1, 4)`, `other_per_frame` 0 rel / 1 dbg | 1 / 1 / 3 (total 11 rel, 12 dbg) | same | no | unchanged | rel 2.125/3 OTHER 0, dbg 3.125/4 OTHER 256 ≤ 264 [2.125/3] |
| 12 | **S1c RELEASE** (1240 bodies, W = 4) | scope 109..=121, chunk 193..=217, dispatch 339 (total 347) | scope **134..=134**, chunk **230..=230**, dispatch **365** (total 373) | **YES, four times** | `08fe7b9f` 2026-09-18 "fix(physics): a clipped face-contact point is named by the two features that created it … (A7a)" → 109..=133 / 205..=217 / 350; `8d656ad8` 2026-09-18 "fix(physics): a box pair takes the edge path only if the edge is over 5 mm shallower than the realized face patch (A7b)" → 133 / 229..=241 / 375; `56c1e9e7` 2026-09-19 "perf(physics): the default solver is the colored solve with the AVX2 cohort kernel on … simd_solve = true" → 133 / 229 / 363; `de06b6c9` 2026-09-21 "perf(physics): parallel_narrowphase is on by default, and the census pins the one scope and one block its dispatch costs on every frame (L5 C4)" → 134 / 230 / 365. **Host control = zero:** the file's header ("S1c after A7a") records base `9f712204` built on msvc reproducing the 2026-09-11 gnu adjudication to every printed digit (window mean 331.797, range 326..339, scope 121, chunk 205..217, long run 302..339 mean 316.126, K 63) — "the instrument did not move between the two trees, the two dates or the two toolchains (windows-gnu then, msvc now)". | rel 364.125 / 364..365; scope 134 and chunk 230 on every frame; dispatch MAX 365; K 57 — the header's L5 C4 column to every digit |
| 13 | **S1c DEBUG** (385 bodies, height 10) | scope 73..=97, chunk 73..=97, dispatch 195, other 1 (total 204) | scope **98..=110**, chunk **98..=110**, dispatch **221**, other 1 (total 230) | **YES, twice** | `8d656ad8` (A7b) → 97..=109 / 97..=109 / 219; `de06b6c9` (L5 C4) → 98..=110 / 98..=110 / 221. A7a (`08fe7b9f`) and the SIMD flip (`56c1e9e7`) left it unchanged; the header records the debug base reproduced on msvc (147..196, 73..=97), so the host control is zero for this scene too. | dbg 198.250 / 197..221; scope/chunk mean 98.562 within 98..=110; dispatch MAX 220 ≤ 221; K 47 — the header's L5 C4 debug column |
| 14 | Layout-class predicates: `CHUNK0` 4096, `CHUNK_ALIGN` 64, `INJECTOR_BLOCK_BYTES` 1520, `SCOPE_SHARED` 256 B / align 128 | same | same | no | unchanged; cross-checked at run time against `boyko_threadpool::__layout_receipt` | `[class control] 64 empty installs: scope=1 chunk=0 … + 1 spawn: scope=1 chunk=1, OTHER 0`, both profiles |
| 15 | First-touch allowance `2 × workers` (OTHER budget per window) | same | same | no | unchanged | worst today: OTHER 2 on S2 release (≤ 8) |

## B. `crates/boyko_physics/tests/alloc_frame_attribution.rs` (`harness = false`, 1 test)

msvc today: release `1 passed`, debug `1 passed`.

| # | pin | gnu value (`d5782d43`) | msvc value (HEAD) | moved | reason | msvc-now measured |
|---|---|---|---|---|---|---|
| 16 | §A primitive pricing (install = 1 scope + 0 chunk; +n spawns = 1 chunk to 256, 2 at 257, 3 at 1024; nested empty scope = 2 + 0; `spawn_batch(16)` = 1 + 1; W ∈ {1,2,4,8} identical) | structural, tolerance 0.01 | same | no | unchanged | table reproduced: 1.000, 2.016 … 6.062 @256, 7.086 @257, 20.258 @1024; batch16 2.250; nested empty 2.000; nested one 3.016 |
| 17 | §B/§C zero-body rows (8 bodies 0.000/frame; event lane, change detection, Commands +0.000 over baseline; par_iter +1 scope +1 chunk) | structural | same | no | unchanged | `DISPATCH ACCOUNTING: 2.125 = 1.000 + 1.000 + 0.125 + residual 0.000`; C rows +0.000 / +2.000 |
| 18 | §D lane-scaling comparison | "four workers ≥ **one**" | "four workers ≥ **two**" | **YES** | `caac7d06` 2026-09-19 "perf(physics): parallel_solve is on by default, and a one-worker pool solves inline instead of opening a scope for every wide color (L4)" — since L4 the one-worker row dispatches nothing and reads the install frame only (2.125) by code, so it can no longer anchor the comparison (`alloc_frame_attribution.rs:2292`, `:2340`) | 1 W 2.125, 2 W 272.125, 4 W 368.625 |
| 19 | `NP_BLOCKS = 1` (D2′: ON frame = 2 scope / 1 + `NP_BLOCKS` chunk, OFF = 1 / 1; counter 28 ON / 0 OFF) | *absent* | **1** | **YES (new pin)** | `de06b6c9` (L5 C4): "MEASURED 2026-09-21 on the L5 C4 tree (release and debug) … by the D2′ arm itself with this constant unset" (`alloc_frame_attribution.rs:776`) | rel and dbg: "+1.000 scope, +1.000 chunk, +0.000 injector, +0.000 OTHER; its counter moved 28 over the ON window's 28 frames and 0 over the OFF window's 28" |
| 20 | §F/§G bounds: OTHER over the last 2048 frames = 0; `max_locals_per_app ≤ 2`; `late ≤ 2` | same | same | no | unchanged | F row 0.000 / 0 |

## C. `crates/profile_fixture/tests/profile_axis_census.rs` (UG-18 pinned census, 5 tests)

msvc today: `running 5 tests` … `5 passed` (92.4 s); the six fat-LTO fixture trees and the refusal
probe were rooted at `D:/wt/_targets/tmp/boyko-axis-*` (RK-18 honoured; C: untouched).

| # | pin | gnu value | msvc value (HEAD) | moved | reason | msvc-now measured (`llvm-nm` on the post-LTO objects this run built: total / `mint_cold` / `emit_impl`) |
|---|---|---|---|---|---|---|
| 21 | g14a presence cells: `deep_zone` dev > 0, shipping = 0; `always_zone` dev > 0, shipping > 0 (+ g14b `calls=10`, `tier=0`) | 1 / 0 / 1 / 1 over the **image** (gnu, `7a315058`) | 1 / 0 / 1 / 1 over the **post-LTO object** | **subject moved, cells did not** | `27ac8904` 2026-09-10 "fix(diagnostics): the profile-axis census read a file that has no symbol table under MSVC - it now reads the linker's INPUT, and an empty census is a RED". A host difference that IS explained, in the file's header (`:46-85`): `link.exe` writes no COFF symbol table into the image, `mint_cold` is a `Static` local after LTO, the PDB carries publics only — so the subject is the linker's input object on both hosts, and the header pins both hosts' full readings (`:62-69`). | dev 975/1/0, shipping 967/0/0, `always_zone` 975/1/0 and 975/1/0 — the header's msvc column exactly (gnu column 590/1, 575/0, 590/1, 590/1; totals are not pinned) |
| 22 | g16ab: `profile_fixture_log` dev `emit_impl` > 0, shipping = 0 (+ `ceiling=5` / `ceiling=3`) | 1 / 0 | 1 / 0 | no | same instrument move as row 21 | dev 1009/0/1, shipping 965/0/0 (header: gnu 605/1, 575/0) |

## D. UG-18 path-keyed censuses (source scans — host-independent by construction)

| # | census | pin | msvc now | moved | note |
|---|---|---|---|---|---|
| 23 | `scripts/check_hotpath_exceptions.py` | registry rows = allow sites per file; no blanket `#![allow]` outside the registry | **RED**: `crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs:544` — module-level `#![allow(clippy::disallowed_types)]` on the `#[cfg(all(test, not(loom)))] mod tests` is not in `docs/HOT-PATH-EXCEPTIONS.md` "Blanket exemptions" (34 exceptions / 12 files otherwise) | no | **Pre-existing, inherited**, since `0afcbd7d` 2026-09-13 (EM2′). Not a host effect and not AH's: the plan books it as rung A4a's, citing `entity_reservoir.rs:405`; the line is 544 today (the plan's tree owns that citation). |
| 24 | `production_reachability_census` 13/13 · `gpu_blocking_reader_census` 2/2 (`PINNED` file list) · `boyko_log` `code_registry` 16/16 · `ignore_reasons_census` 7/7 · `goldens_pins_wellformed` 7/7 · `isa_baseline_census` 2/2 · `engine_packages_census` 3/3 · `fill_reject_routing_census` 3/3 · `vg_symbol_reachability` 16/16 · `gaia_g0_citation_census` 4/4 | floors / lists | all green, 73 tests, `running N` ≥ 2 on every binary | no | `[ignore census] 333 sites (183 plain, 150 cfg_attr) across 10 crates, 1642 .rs files walked, 0 waivers` |
| 25 | `goldens/PINS.toml` sha256 pins | NOT re-blessed at AH (rule) | msvc byte-identity: owner check 2026-09-10 (32/32, recorded in the plan's AH (4)); in-tree receipt `8668b9c7` 2026-09-18 (software 32/32 PASS at `e3cebe2d`'s gnu-blessed pins, msvc, RTX 3060); `54e186d9`'s receipt 61/61 check-only (34 software + 27 hwrt) on msvc | no | **Text defect, fixed by this commit:** `goldens/PINS.toml:35-42` still said "THE sha256_* VALUES BELOW WERE ALL BLESSED ON THE windows-gnu HOST … UNMEASURED" — false since 2026-09-10, and contradicted by the file's own later sections (re-blessed on the msvc host for stated render causes: 2026-09-18 `0973eec2` (SG4 defined frame; CSM reaching VB), 2026-09-19 `429b6698` (`grand_showcase_2mat`, light-table R2), 2026-09-21 `0759d193` (14 TAA history legs, R4 frame order + R4b open edges)). `scripts/golden.ps1:32-36` repeated the same sentence and is corrected in the same commit. Both edits are line-count neutral (8 in / 8 out; 5 in / 5 out), so no citation into either file moves. |

## E. Miri receipts — `docs/threadpool/receipts/tb-neg-m2w-{0,1,7,15}.stderr` (UG-08)

Pinned content = the 5 receipt predicates of `scripts/tb_neg_gate.{ps1,sh}` and
`tb_neg_m2w_arm_present::receipts_for_every_seed_exist_and_name_the_declared_diagnostic`: non-zero
exit + `error: Undefined Behavior`; `deallocation through … is forbidden`; protected tag born at
`scoped.rs:<line of _cell: &ScopedCell<>` = 303:17; accessed tag born at `block.rs:<line of
unsafe { alloc(layout) }>` = 482:28; deallocating frame `ScopeBlock::free_all`.

| # | pin | gnu value (`d51b4ced`, miri 8925ea358a 2026-08-20) | msvc value (HEAD, miri a36d05efab 2026-09-09) | moved | reason |
|---|---|---|---|---|---|
| 26 | 5 predicates × 4 seeds | all 5 hold, 4/4 seeds | all 5 hold, 4/4 seeds, on BOTH nightlies (`.gnu.stderr` beside each, same tree, same day); `tb_neg_m2w_arm_present` 6/6 today | **backtrace moved, predicates did not** | The frame list gains `Scope::<'_>::join_after_body` (`scope.rs:1230:9`) and shifts `thread_pool.rs` 341→298 / 534→492, `scope.rs` 1479→1470 — `1693234d` 2026-09-17 "fix(threadpool,ecs,app): a panic in a system or pool task resumes on the caller instead of blocking it (A6)". The chunk offset at `free_all` (`alloc…[0x0]` on all four `d51b4ced` receipts) is seed-dependent on both nightlies from this tree (gnu 0x38/0x0/0x38/0x38, msvc 0x38/0x38/0x0/0x0) — the bump slot the protected cell occupies, not a predicate. msvc vs gnu body diff after masking ids and paths: only that offset on seeds 1, 7, 15, and the runner path (`docs/threadpool/receipts/README.md`). |

Independent spot re-run (seed 0, msvc nightly, `D:/wt/_targets/ah-miri-msvc` with
`build/boyko-threadpool/` deleted first → one `Compiling boyko-threadpool` line; the gate's
MIRIFLAGS in full, `-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance
-Zmiri-ignore-leaks -Zmiri-preemption-rate=0 -Zmiri-seed=0`; `RUSTUP_TOOLCHAIN` unset): `running 1
test`, exit 1 (the expected red), receipt body byte-identical to the committed `tb-neg-m2w-0.stderr`
after id/path masking, `alloc60834[0x38]` unmasked. Not committed: the committed receipt is the same
run.

## F. `crates/boyko_threadpool/tests/block_allocation_receipts.rs` (KE16 R0/R0b/R1/R2/R4′)

| # | pin | value | moved | msvc now |
|---|---|---|---|---|
| 27 | R0 `CELL_ALLOCS` 64/64 · R0b `CHUNK_ALLOCS` 1/1 · R1a/R1b 0 · R2 n ∈ {1,16,64,1024} → {1,1,2,5} predicted · R4′ `CHUNK_LIVE` 0 | structural, unchanged since `67563d3b` 2026-09-09 | no | `1 passed` (the `panicked at :592` line in the log is the deliberate `PANIC_MARKER` body of the panicking-scope arm) |

## G. `CLAUDE.md` ignored-suite count (prose, not a gate)

| # | figure | value in this tree | live figure today | note |
|---|---|---|---|---|
| 28 | `CLAUDE.md` "333 sites (183 unconditional + 150 cfg_attr), measured 2026-09-21" | 333 / 183 / 150 | 333 / 183 / 150 | matches. (Main's `CLAUDE.md` still says 164 = 143 + 21 — stale prose on main; the A8 merge carries this tree's block.) |

## Unattributed moves — RED, escalated

None. Every one of the six moves (rows 12, 13, 18, 19, 21, 26) has a commit whose message names
its cause, and the two measured envelopes (rows 12–13) carry a zero host control in their own
header. Had any row lacked that, the rule is: the pin stays at its gnu value, the row is marked
RED here, the same test is run under `stable-x86_64-pc-windows-gnu` in `D:/wt/_targets/joltab-gnu`
on the same tree, and the pair of readings goes to the orchestrator — never a `-Bless`, never a
widened range (P9).

## Gates run for this ledger (msvc, `D:/wt/uploadleak` @ `85b2aa49`)

- **UG-01 on the touched crates:** every pinned census above — `boyko-physics` ×2 binaries ×2
  profiles, `boyko-threadpool` ×2, `boyko-log` ×1, `profile-fixture` ×1, root ×9 — 0 failures, no
  `running 0 tests` anywhere. The full `--workspace --all-targets --no-fail-fast` suite was not
  re-run for this rung: its parent `54e186d9` records 5,874 passed / 0 failed on msvc, and AH (2),
  (3) and this commit change comments, docs, scripts and config prose only.
- **UG-08:** the four committed msvc receipts and the four gnu-beside receipts read and diffed;
  seed 0 re-derived on the msvc nightly (§E).
- **UG-09:** `cargo --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' test -p
  boyko-threadpool --test loom_pool -- --list` → `6 tests, 0 benchmarks` after a `Compiling
  boyko-threadpool` line (the header requires 6). Models not executed: AH touches no loom code.
- **UG-18:** rows 23–25. One RED (row 23), inherited and already scheduled (A4a).

## Cleanup owed by procedure step 4

No gnu comparison target dir exists (never created). `D:/wt/_targets/ah-miri-{msvc,gnu}` (177 MB /
196 MB) are AH (2)'s Miri trees and remain for deletion after this commit.
