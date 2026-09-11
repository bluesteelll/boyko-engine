# Frame-allocation census gate - the two gate defects fixed

- **Source:** workflow `census-gate-and-commands-id-leak` (run `wf_396905fa-b76`), gate-fix agent, tree `D:/wt/joltab`
- **Status:** DONE by the gate-fix agent (both profiles green, four mutations red as required), NOT COMMITTED: the census file also carries the S2 realloc pin re-derived for the id-recycling change, so the two land together.
- **Copied verbatim** on 2026-09-11 at the checkpoint the owner asked for; agent outputs are reproduced without edits.

---

## Gate-fix report

The gate now goes red on the adjudicator's mutation, and the three mutations that were already red still are. Both binaries pass in release and dev. Clippy (`-p boyko-physics --tests -D warnings`) and `cargo check --workspace --all-targets` are clean. All work was in `D:/wt/joltab`. The only other file touched is `crates/boyko_physics/Cargo.toml`; nothing is staged and nothing is committed. Every `src` mutation was restored with cp + touch, the sha matches the original, and `git status` shows `src` clean.

## (1) S1c pins

I could not pin S1c exactly against the dispatched-colour count. Whether a colour dispatches is decided by private constants in `solver/colored.rs` (`MIN_PARALLEL_SLOTS_PER_COLOR`, `MIN_SLOTS_PER_CHUNK`, `CHUNKS_PER_WORKER`) and a private data structure. Recomputing that in the test would copy the code under test, not observe it. So I dropped the upward headroom instead.

| | scope | chunk | dispatch MAX | total MAX |
|---|---|---|---|---|
| release | 109..=121 (was 109..=133) | 193..=217 (was 181..=241) | 339 (was 375) | 347 |
| debug | 73..=97 (was 73..=109) | 73..=97 (was 73..=121) | 195 (was 231) | 204 |

- **Total MAX is 339 plus the unchanged 8-acquisition first-touch `OTHER` allowance.** That allowance is budgeted separately over the whole window, and S1c's steady window used 0 of it in every run.
- **The existing per-frame check could not catch this mutation.** It requires `scope - 1` to be a multiple of 12, and 132 still is. The header now says so.
- **The long-run numbers are my own measurement, not just the adjudicator's.** I extended S1c to 17 × 256 steady steps in a scratch copy of the file. Release read 302..339 per step, block means 331.8 (block 0 is the census window itself) down to 307.5, 316.1 overall, scope 109..121, chunks 193..217, `OTHER` 0, realloc 0. Nothing exceeded 121 / 217 / 339.
- **Debug had no long-run data, so I ran one.** It read 147..196 per step, scope 73..97, one chunk per scope, dispatch MAX 195. The debug lower bound of 73 is therefore measured, not assumed.

## Mutations (logs in `D:/tmp/censusfix/`)

| mutation | result | log |
|---|---|---|
| M1: one extra `pool.scope` + spawn per colour pass (`solve_all_colors`), release | **RED**: reproduces 355.797 / MAX 363. 4 violations: scope 133 > 121, chunk 229 > 217, dispatch 363 > 339, MAX 363 > 347 | `mut_M1_extra_scope_per_pass_release.log` |
| M1, debug | **RED**, 4 violations: scope 97..109, chunk 97..109, dispatch 219 > 195, MAX 220 > 204 | `mut_M1_extra_scope_per_pass_debug.log` |
| M2: fresh `Vec` + push per step in `physics_apply` | **RED**: S1a, S1b and S1c each show 256 `OTHER` | `mut_M2_vec_per_step_physics_apply_release.log` |
| M2 again, without `--nocapture` | **RED**, exit 101; the held report is printed before the panic | `mut_M2_captured_mode_release.log` |
| M3: fresh `Vec` + push per `Schedule::run` | **RED**: 12 of 12 scenes | `mut_M3_vec_per_schedule_run_release.log` |
| M4: `Local<Vec<u32>>` pushed and never cleared, in `physics_apply` | **RED**: 2 reallocs against a pin of 0 in S1a, S1b and S1c | `mut_M4_local_vec_never_cleared_release.log` |

After the mutation runs I changed only comments and doc text, never code or pins.

## (2) 60-second notice

Both targets are now declared `[[test]] harness = false` in `Cargo.toml`, and each file has its own `main()`. The census runs on the main thread with no libtest thread beside it, so nothing from a harness can reach the counter at any run length.

- **Same command line as before.** `main` still prints `running 1 test` and honours name filters, `--exact`, `--skip`, `--list`, `--ignored` (runs nothing) and `--nocapture` / `RUST_TEST_NOCAPTURE`, and exits with 101 on failure (`cli_contract.log`).
- **Quiet on a pass.** The report is held and printed only on failure or with `--show-output`, so a workspace `cargo test` is not flooded (`census_release_captured_pass.log`).
- **Miri.** I removed `#![cfg(not(miri))]`, because it would also strip `main` and break the Miri build. `main` now returns early under Miri; `cargo miri test` on both targets prints `running 0 tests`, exit 0 (`miri_census_attribution.log`).

## (3) Header corrections, in both files

- **S1c figure.** It is now quoted as 302..339 per step, with 256-step block means of about 307..332. 331.797 is labelled as the census's own fixed window, which is deterministic and so repeats exactly; it is not the steady state.
- **The 99.5 % trace.** It is now paired with its own step, which I re-ran with `BOYKO_ALLOC_TRACE=1`: the 50th step of a fresh pile, still in warm-up. That step had 145 scope frames (12 colours) and 386 acquisitions, 386 of 386 captured. `physics_solve_colored` owned 384 (144 scope boxes + 240 chunk grows); the install frame owned 2. Log: `attribution_release_trace.log`.
- **Coverage boundary.** Both headers now state that the counter sees the Rust heap only. The ECS columns grow through `VirtualAlloc(MEM_COMMIT)` (`VmReservation::commit` in `boyko_ecs/src/ecs/memory/vm.rs`), which is not counted. So "0 from every physics buffer" means zero heap allocations, not zero memory growth. The FREE line in the attribution file's classification table says the same.

## Final runs, on the final file contents

| run | result | log |
|---|---|---|
| census release | GATE GREEN; S1c `MAX 339<=347 dispatch 339<=339 scope 121..=121 in 109..=121 chunk 205..=217 in 193..=217` | `final_alloc_frame_census_release.log` |
| census dev | GATE GREEN; S1c `MAX 196<=204 dispatch 195<=195 scope 85..=97 in 73..=97 chunk 85..=97 in 73..=97` | `final_alloc_frame_census_dev.log` |
| attribution release / dev | pass | `final_alloc_frame_attribution_{release,dev}.log` |
| clippy | clean | `clippy_physics_tests.log` |
| workspace check | clean | `check_workspace_all_targets.log` |

## sha256

- `ae74b27d559834a27ceb22a83dd8fb929a63c356a2801cf6a89ddbea9b27645f`  `D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_census.rs`
- `4cfff8d661ec0e862adc7f8d3eec2d5a4c12ed9dbd9be1f451c676d078d568e1`  `D:/wt/joltab/crates/boyko_physics/tests/alloc_frame_attribution.rs`
- `cc90f98cd361a52c314f92360a0e5d546c7b7f4981789e56370c02763cf489be`  `D:/wt/joltab/crates/boyko_physics/Cargo.toml` (modified, tracked: +18 lines, the two `[[test]]` entries)

The same hashes are in `D:/tmp/censusfix/final_sha256.txt`. Backups of the original files are in `D:/tmp/censusfix/*.orig`.
