# Device-leg verdict — first run on `chore/device-leg-sweep` @ `0e0e3b0e`

**Provenance of this verdict.** The harness delivered me **13 of 75 run reports** (run-list positions 1–13, verbatim; positions 14–75 were cut — the repository's own catalogued `.slice(0,N)` silent-truncation mode) and **7 of 8 triage reports**. Everything below marked *verified* I re-derived myself in `D:/wt/gpuleg` this session; everything marked *recovered* comes from the runner agents' own log artifacts still on disk; everything marked *relayed* is the runner's claim that I could not independently re-measure. Tree clean, `git status --porcelain` empty, HEAD unchanged, nothing edited.

---

## 1. What the leg covers, in numbers

**Denominator — verified.** I re-ran the arithmetic over `D:/tmp/ignore_sites.tsv` (the repository's own scanner output, 146 rows) against `D:/tmp/runlist.txt`:

| quantity | CLAUDE.md | true at `0e0e3b0e` | source |
|---|---|---|---|
| device-leg tests | **135** | **141** | 141 = 146 sites − 1 lib `cfg_attr` − 3 generators − 1 `app12_timer_resolution` |
| device-leg binaries | (unstated) | **75** | |
| tree-wide `#[ignore]` sites | 164 | **311** | census self-report |
| behind `#![cfg(windows)]` | ~131 | **107** | relayed, consistent with my file scan |
| needing a cargo feature | 4 | **5** | relayed; 6th needs it *by protocol* only |

The +6 is attributable to six boyko_app sites added after 2026-08-21 (asset/VB teardown and slot-leak gates). **141 is the figure to put in CLAUDE.md.**

**Attempted / ran / not run:**

| class | tests | note |
|---|---|---|
| **NOT RUN — cannot be built** | **4** | `hwrt_blas_smoke` ×3, `asset_streaming_f7_rt_cap_headless` ×1 |
| **Ran the wrong pipeline leg** | 1 | `grand_showcase_mvpm` — needs `hwrt` by protocol, not by `cfg`, so it compiles and silently renders the non-RT path |
| **Escalated to triage** | 8 | 7 libtest reds + **1 target that reported `3 passed`, exit 0** |
| Remainder | 128 | split between real passes and early returns — see §2 |

**The 4 not-run are a build break I verified end-to-end, and it is worse than reported.**

```
cargo check -p boyko_rhi_vulkan --features hwrt --lib   →  10 × E0308, exit 101
```

Two source sites, not one:

- `D:/wt/gpuleg/crates/boyko_rhi_vulkan/src/device.rs:3254` — `W2102.number()`
- `D:/wt/gpuleg/crates/boyko_rhi_vulkan/src/present/targets.rs:75` — `W2106.number()`

Both are `#[cfg(feature = "hwrt")] #[cold] #[inline(never)]` diagnostic reporters. `git show 718a5129` (2026-08-17, *"a code's class and its number are paired by the compiler"*) converted **two** `W2102.number()` sites in that same file to `W2102` and left the third — the feature-gated one — untouched. **`--features hwrt` has not compiled for 248 commits / 32 days**, and `cargo clippy --workspace --all-targets -D warnings` is structurally incapable of seeing it because it never passes `--features`. This is the same failure genus as the `--workspace` and `--no-fail-fast` entries already in CLAUDE.md: a gate whose *selector* excludes the broken code.

---

## 2. Real work vs. early return — the number that matters

Call it the **silent-skip numerator**. Verified by scanning all 75 target files plus their `tests/common/mod.rs`:

| | targets | tests | share |
|---|---|---|---|
| Carry a silent early-return that still reports `ok` | **46** | **108** | **77 %** |
| No skip route at all — red is honest | 29 | 33 | 23 % |

All 29 no-skip-route targets are `boyko_app` windowed scene/screenshot drivers. **Every test in `boyko_render` and `boyko_rhi_vulkan` is silently skippable.** On a GPU-less box, 108 of 141 report `test result: ok`. Three runners reproduced that live with a bogus `VK_ICD_FILENAMES` (0.02–0.04 s, `1 passed`, exit 0).

Three distinct early-return classes, which the leg currently cannot tell apart from a pass:

**(a) Device/cap skip — 108 tests exposed.** `boot_*_or_skip`, `ddgi_storage_ok`, `timestamps_usable`, WSI/format guards. On this box these did *not* fire for the 13 targets I have full reports for (17 tests, each with a printed device name and a SKIP-line-absence proof backed by a forced-skip control).

**(b) Worker-shaped tests — 16 of 141 (11 %), verified list.** These are child processes a sibling driver spawns; invoked directly by the leg's per-binary `-- --ignored` they measure nothing:

```
hzb_engine_pyramid_gate    hzb_engine_pyramid_dump, _dump_occ, _dump_occ_late
vb_occ_split_gate          vb_occ_probe_dump_{marked,marked_no_hzb,multi,unmarked}
vb_bench_query_validation  vb_bench_query_validation_{bench,control}_worker
vb_occ_mixed               vb_occ_mixed_capture_worker
vb_cull_hzb_pairing        vb_cull_hzb_pairing_worker
vb_sv0_produce_run_timing  vb_sv0_produce_run_worker
vg_occ_split_timing        vg_occ_split_timing_worker
vb_inst_cull_corpus        vb_inst_cull_corpus_worker
vg_density_census          vg_census_rung_dump        ← NO GUARD
vg_r0d_census              vg_r0d_rung_dump           ← NO GUARD
```

**14 carry a skip-by-name guard and pass vacuously; exactly 2 use `.expect("the worker is told its rung")` and panic (exit 101).** Recovered from `<scratchpad>/gpuleg_vbocc_G2/run2.log`: `5 passed` with four `… is unset -- SKIPPED` lines — i.e. **4 of those 5 "passes" measured nothing**. Same shape recovered in the `hzb_engine_pyramid_gate` (3 SKIP lines / 6 passed) and `vb_bench_query_validation` (2 SKIP lines / 3 passed) logs.

**(c) Missing gitignored payload — verified.** `D:/wt/gpuleg/assets/vg_corpus/` holds only `CORPUS.toml` and `README.md`. `vb_inst_cull_corpus` (2 tests) and `vg_r0d_census_gate` skip by name for want of the payload. Instance culling against the fetched corpus was **not** exercised.

**(d) Command-protocol skip — one confirmed casualty.** The campaign-wide `BOYKO_DISABLE_VALIDATION=1` makes `m2_brick_atlas_smoke` return at `m2_brick_atlas_smoke.rs:68` *before* `BrickAtlas::create`, while still printing `1 passed`. It only did real work when run with its own module-header command. The same variable silently disarms the messenger oracle in every `boyko_render` target (`NOTE: validation disabled … messenger oracle skipped`).

**Bottom line for §2.** Of the tests I can attest individually: **≥ 46 did real, asserting work on the RTX 3060** (17 fully reported + 27 `window_present_gbuffer` recovered with a per-test device line and zero SKIP lines in both runs + `vg_decidability_floor` + `vb_occ_split_gate`'s one real gate); **≥ 19 passed by returning early** (15 workers observed skipping + 2 corpus + m2's skipped arm + at least one validation-degraded arm). The residual ~64 sit in the truncated portion of my input and I will not put a number on them.

---

## 3. The eight triaged items, ranked by cost of being wrong

| # | target | verdict | cost if we get it wrong |
|---|---|---|---|
| 1 | `vg_occ_split_timing` | **REAL_DEFECT** | contaminates *published numbers* |
| 2 | `vb_bench_query_validation` | TEST_DEFECT (+ hidden real defect) | a gate that has never once been armed |
| 3 | `vg_r0d_census` / `vg_density_census` | TEST_DEFECT ×2 | two binaries permanently exit 101 |
| 4 | `sdf_gbuffer_hybrid` | TEST_DEFECT | masks 7 sibling assertions |
| 5 | `vb_sv0_produce_run_timing` | STALE_EXPECTATION | design doc predicted the red |
| 6 | `room_smoke` | STALE_EXPECTATION | 611 commits red, zero risk |
| 7 | *(8th — truncated from my input)* | unknown | must be re-requested |

**R-1 — `vg_occ_split_timing` — REAL_DEFECT. Highest cost, and the only one in shipped code.**
`D:/wt/gpuleg/crates/boyko_app/src/runner.rs:117-120` declares `VB_BENCH_WARMUP = 20`; `:2900-2901` feeds **every** retired frame to `vb_zone_reducer.observe_frame(pairs)` with no guard, while `:2928` budgets for a discard that never happens. The 20 shader-compile / clock-ramp frames are inside the published window. `vg_occ_split_timing` is the only harness in the tree that checks the window *size*, which is why this surfaced here and nowhere else. Every VB-P1d/VG zone number taken through this instrument is suspect, and this repository decides by numbers.
→ **Next action:** developer adds the discard guard at `runner.rs:2900`; then the orchestrator must re-measure — not re-bless — every zone baseline taken through the zone leg since `VB_BENCH_WARMUP` landed.

**R-2 — `vb_bench_query_validation` — TEST_DEFECT over a real defect. This one was GREEN.**
It reported `3 passed`, exit 0, and was escalated anyway. `vb_bench_query_validation.rs:269` strips `BOYKO_DISABLE_VALIDATION` and calls that "THE POINT OF THIS FILE", but the backend needs two conjuncts: `runner.rs:221` (`BOYKO_ENABLE_VALIDATION` present) **and** `device.rs:2543` (`BOYKO_DISABLE_VALIDATION` absent). Stripping the second while the first is false enables nothing; the `ValidationUnavailable` escape hatch sits *inside* `if config.enable_validation`, so the dead oracle never lands in INSTRUMENT-DEAD. Measured: 0 validation lines armed, 19 with `BOYKO_ENABLE_VALIDATION=1`. `scripts/golden.ps1:170` was repaired for exactly this three days *before* this file was written; this is the last unrepaired site.
→ **Next action:** two separate fixes — set `BOYKO_ENABLE_VALIDATION=1` in `spawn_worker`, **and** make the gate assert `messages_total > 0` on the control arm so it cannot pass with a dead oracle. Fixing only the first leaves a second false green.

**R-3 — `vg_r0d_rung_dump` and `vg_census_rung_dump` — TEST_DEFECT, structural.**
`vg_r0d_census.rs:152` and `vg_density_census.rs:189` panic on `VarError::NotPresent` in 0.00 s, before any device boot. The binaries exit 101 on every machine, GPU or not. The repository already wrote the fix and the rationale at `D:/wt/gpuleg/crates/boyko_app/tests/vb_inst_cull_corpus.rs:249-258` — *"a panic here reads as a real failure of the corpus gate"*. Neither red is attributable to any of the five merged lanes (the `expect`s predate them by ~7 weeks); they surfaced only because nobody had ever run this leg. Compounding it: `vg_r0d_census_gate`'s `ok` is itself a corpus-absent skip, so **neither** test in that binary adjudicated anything.
→ **Next action:** copy the `vb_inst_cull_corpus.rs:253` skip-by-name guard to both sites. Zero risk, converts two permanent reds into honest skips.

**R-4 — `sdf_gbuffer_hybrid` (a2) at `:6967` — TEST_DEFECT.**
The ±3/255 pin asserting brick-cubic ≡ analytic is refuted by the test's *own* CPU oracle on the same scene by 33/255, and that oracle is byte-unchanged since `6903a487`, the commit that wrote the pin. The GPU's 12/255 is *smaller* than the pinned model predicts; all 32 divergent pixels are on the true surface (`hit/background flips = 0`), so the assert's stated diagnosis — B1 over-relaxation overshoot — does not fit. One red assertion holds seven passing siblings hostage.
→ **Next action:** architect decides what property (a2) is actually meant to bound; do **not** widen the tolerance to fit the observation.

**R-5 — `dp6_0b_fused_leg_matches_its_expectation_table` — STALE_EXPECTATION.**
`c1caa422` (DP6a) made `vb_sv0_split ⇒ mesh_geo_shade_split`; `table_fused` at `:105-125` still declares `ZONE_VB_GEO`/`ZONE_VB_PRESHADE` `Forbidden`. The same commit touched this file — four lines of comment — and left the table. `docs/VB-SV0-DP6-DESIGN.md:421-426` predicts the red in words, and the text is copied verbatim into the test's own doc at `:205-212`. The falsification run (`BOYKO_SDF_MESH` unset) passes cell-for-cell.
→ **Next action:** re-point `table_fused` to the DP6a values; no engine change.

**R-6 — `room_smoke_ten_frames_then_clean_teardown` at `:206` — STALE_EXPECTATION.**
`7ebe9b9f` flipped `DEFAULT_FIT_MODE` to `CatchAll` **43 minutes** after the assertion was written and updated three files, none of them this one. Red for 611 commits. The reducer's `raw_far` recomputes to `9.122565` from scene constants alone — seven significant digits, on a value the test does not hard-code. The engine is right.
→ **Next action:** re-point the expectation to `CatchAll`; no engine change.

**R-7 — the eighth item.** My input carried `TRIAGE (8)` and seven bodies. Re-request it before this verdict is acted on; do not assume it is benign.

---

## 4. What a fully green run of this leg would still not prove

- **Hardware ray tracing: zero coverage, and it cannot be obtained today.** Four tests unbuildable, plus `grand_showcase_mvpm` silently rendering the wrong leg. The hwrt shadow chain's own `#[cold]` diagnostic reporters do not compile — nothing behind that feature has been type-checked in 32 days.
- **Image correctness.** The 27 `window_present_gbuffer` tests and ~20 `*_screenshot_dump` tests *dump* a frame. What the leg establishes is "a frame was produced, no validation message, no hang" — not "the image is right". Renders that are shipped-to-the-GPU-but-wrong-on-screen (this repo's catalogued GI failure) pass this leg.
- **Corpus-backed culling.** `assets/vg_corpus/` payload absent → `vb_inst_cull_corpus` and `vg_r0d_census_gate` skip by name.
- **Teardown-time validation.** `m3_dirty_atlas` asserts `messenger.total() == 0` at `:546-563`, *before* `destroy`. A teardown double-free is visible to the layer and invisible to the test. The mutation run showed the layer does report teardown faults (10 leaked objects when a panic skipped destroy).
- **Two headline parity claims are not checked by the tests that claim them.** `m5_scroll_atlas`'s `scroll_update ≡ rebake_all` asserts only that a pure CPU baker is deterministic (`full_bake_at(x) == full_bake_at(x)`); `m3_dirty_atlas` passed with `atlas_incr` built from an *empty* field. Both print a success line naming the property.
- **Validation as an oracle is conditional on the command.** Any target run with the campaign-wide `BOYKO_DISABLE_VALIDATION=1` has no messenger. One target (`m2_brick_atlas_smoke`) abandons itself entirely under it.
- **Non-Windows.** 107 of 146 sites are `#![cfg(windows)]`; the leg says nothing about Linux.
- **This one box.** Device-conditional skips mean a green here is a statement about an RTX 3060 Laptop / driver 610.47. On any other machine 108 of 141 would report `ok` having measured nothing, and the leg would not notice.

---

## 5. The one change: make skipping mechanically visible, and classify workers out of the leg

Today the leg's only evidence that a test did work is a human reading `--nocapture` and observing the **absence** of a line. That is why every competent runner report in this campaign spent most of its length proving a negative, and it is why 19+ vacuous passes still slipped through as "passed". Concretely, implement:

**(a) One skip marker, machine-readable.** Every early return prints a line beginning `BOYKO-SKIP <test>: <class>: <detail>`, with `class ∈ {device, cap, validation-off, worker-unspawned, payload-absent}`. Today the same event is spelled four incompatible ways — `SKIP <name>: …` (`crates/boyko_render/tests/common/mod.rs:37`), `<label>: <ENV> is unset -- SKIPPED` (`crates/boyko_app/tests/vb_occ_split_gate.rs:351`), `SKIP: validation disabled` (`crates/boyko_rhi_vulkan/tests/m2_brick_atlas_smoke.rs:68`), `NOTE: validation disabled` (`crates/boyko_render/tests/common/mod.rs:55`). Unifying is a mechanical rename across the ~49 skippable targets.

**(b) The leg runner fails on a marker.** A per-binary wrapper that greps the captured stream and returns non-zero if any `BOYKO-SKIP` line appears with a class not on that binary's allow-list. On the owner's box, `device`, `cap` and `payload-absent` are all *failures of the run*, not of the code — which is the correct report and is exactly what the current `exit 0` hides.

**(c) A `worker:` prefix in the reason vocabulary, enforced by the existing census.** CLAUDE.md already proposes `#[ignore = "<class>: <prose>"]`. Add `worker`. Then `tests/ignore_reasons_census.rs` — which already lexes reason strings, joins multi-line attributes and resolves `fn` names — additionally asserts that any `worker:`-classed test's body contains a skip-by-name guard on its driver env var. That single assertion would have caught `vg_r0d_census.rs:152` and `vg_density_census.rs:189` at commit time, seven weeks before this sweep, and it turns the leg's selector into a `grep` that excludes all 16 workers.

**What it buys, in numbers:** the leg's gate count drops from a misleading 141 to **125 real gates**; 14 device-booting processes that measure nothing stop being spawned (cheaper); 2 permanent `exit 101` reds become honest skips (more honest); and the 108 silently-skippable tests stop depending on a human noticing an absent line.

**Files a fix would touch:** `D:/wt/gpuleg/tests/ignore_reasons_census.rs`, `D:/wt/gpuleg/crates/boyko_app/tests/vg_r0d_census.rs:151-155`, `D:/wt/gpuleg/crates/boyko_app/tests/vg_density_census.rs:188-191`, `D:/wt/gpuleg/crates/boyko_render/tests/common/mod.rs:37,55`, and the 16 worker sites listed in §2(b). None of it is engine code.

---

## What this leg did establish

Not a formality — on 46+ tests with individually-verified anti-vacuity evidence, on a merged line carrying a physics rewrite, a render/app merge, a threadpool panic change and a host-toolchain move, the RTX 3060 produced: bit-exact HZB build and cull verdicts against the host oracle across 131 072 corpus pairs and 72 boundary probes; bit-exact DDGI probe sampling across 6×3 lanes; validation-clean BrickAtlas create/upload/rebake lifecycles; validation-clean 27-scene windowed present across every lighting and shadow leg; and a reproducible particle-sim register footprint. **Zero validation messages anywhere the oracle was actually armed.** The reds are, with one exception, tests that had drifted away from an engine that moved correctly underneath them — and that exception, `vg_occ_split_timing`, was found precisely because someone finally ran the leg.