# Profiling — integration, the 17-rung ladder, and every gate

<!-- CONTRACT
provides: profiling/ladder
exports: profiling/gates
assumes: substrate/loss-fold
assumes: substrate/clock-source
assumes: substrate/section-report
assumes: substrate/lane-write-sites
assumes: substrate/ladder-d0-d1
assumes: substrate/gates-dg
assumes: seam/build-axis
assumes: seam/joint-cost
assumes: seam/free-when-off
assumes: seam/landing-order
assumes: seam/diagnostic-code-space
assumes: profiling/budgets-and-invariants
assumes: profiling/emission-abi
assumes: profiling/store-and-fold
assumes: profiling/gpu-zone-seam
assumes: profiling/statistics-discipline
assumes: profiling/contrast-api
assumes: profiling/game-facing-surface
-->

**Carved from** `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — §Integration, §"Implementation plan —
every rung compiles the workspace alone", and §"Metrics and validation" in full. Gate **GJ1** is
new at the split and comes from S13 (`seam/free-when-off`). One `file:line` anchor is corrected
against the tree; it is marked where it appears. Diff against that document until it is retired.

---

## Integration

| File | Change |
|---|---|
| `crates/boyko_utils/Cargo.toml` | **unchanged, and it must stay unchanged** — still an empty `[dependencies]` (S2). Rev 3's `boyko_utils::profiling_abi` is withdrawn |
| `crates/boyko_diag/Cargo.toml` | **new crate** — `std` only, **zero workspace and zero third-party deps**; a tidy test pins `cargo tree -p boyko_diag` to exactly one node |
| `crates/boyko_diag/build.rs` | **new — the ONE build script that reads `BOYKO_PROFILE`** (S9): emits `GLOBAL_TIER`, `REGION_CAPACITY`, `ENGINE_ZONE_SLOTS`, `MAX_USER_BUDGET`, `DYN_NAME_BYTES`, the logging plan's `GLOBAL_CEILING`, `BOYKO_BUILD_HASH`, and `cargo:rerun-if-env-changed=BOYKO_PROFILE`. **`LANE_COUNT` is NOT emitted** — Q1 deleted its profile axis, so it is a plain `const` in `boyko_diag::lane` and putting it back on this axis re-opens the unsoundness Q1 closed; `compile_error!` on a per-knob override outside `BOYKO_PROFILE=custom` |
| `crates/boyko_diag/src/{clock,lane,loss,storage}.rs` | **new, SHARED with the logging plan** — `ticks/ticks_per_ns/clock_epoch/calibrate/note_forward_jump/invariant_tsc/SessionId` · `lane/set_lane/claim_lane/release_lane` + `LANE_*` · `LossClass/LossCell/LossTotal/LossStatus/fold_into/DiagFlag/raise/take_raised` · `SyncCells` + `assert_bss_eligible` + `section_report` |
| `crates/boyko_diag/src/profiling_abi/` | **new module group** (hosted here, not shared): `channel, scope, zone, dyn_registry, sample, lane_ring, macros, tier`, incl. `profiling_partition!` and `ENGINE_PACKAGES` (B3) |
| `crates/boyko_threadpool/Cargo.toml` | **`+ boyko_diag = { path = "../boyko_diag" }`** — the one new edge for the shared lane TLS (F1/F12/S3). (Rev 3's `+ boyko-utils` edge is withdrawn) |
| `crates/boyko_threadpool/src/tls.rs` | `+ #[cfg(debug_assertions)] OPEN_DEPTH`; re-export `boyko_diag::lane::*`. **No lane TLS of its own** |
| `crates/boyko_threadpool/src/worker.rs` | `boyko_diag::lane::set_lane(worker_id)` on `worker_main` entry, beside `set_current_worker_id` (F12) |
| `crates/boyko_threadpool/src/thread_pool.rs` | set/restore `set_lane(LANE_DISPATCHER)` on `install` entry/exit |
| `crates/boyko_ecs/build.rs` | **NOT created** (S9) — rev 3's row is withdrawn; the consts are re-exported from `boyko_diag` |
| `crates/boyko_ecs/Cargo.toml` | **`+ boyko_diag`, `+ boyko_log`** — the fold reads `take_raised()` and is the only site that *emits* a `W92xx` |
| `crates/boyko_ecs/src/ecs/core/profiling/` | **new module group**: `store, fold, lifetime, hist, contrast, floor, concurrency, telemetry, ecs_control, plugin` |
| `crates/boyko_ecs/src/ecs/core/system/system_meta.rs` | `+ pub(crate) zone: ZoneId` (u16, unconditional in **both** axes, offset 242); `+ const _: () = assert!(size_of::<SystemMeta>() == 256)` beside the test at `:421` |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | `zone!` around `run_unsafe` (`:1267`) and the dispatcher-inline path (`:1108`); `RoundRecord` after the `to_spawn` loop (`:1222`) — all `Deep` tier |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | mint each system's `ZoneId` at `try_build` **under `const GLOBAL_TIER >= Dev`**, refusing (not panicking) past the budget; snapshot the `ConflictGraph` at arm under `profiling-analysis` |
| `crates/boyko_ecs/src/ecs/core/app/app.rs` | fold call at the top of **`update_with_delta` (`:655`)**, before step ①; `__frame` / `__events` / `__fixed_step` / `__main_run` zones (D16/F2). `update` (`:736`) unchanged |
| `crates/boyko_rhi/src/device.rs` | `+` three verbs; mark `read_query_pool_ns`/`_ticks`/`_pairs_ns` **FROZEN — no new callers** |
| `crates/boyko_rhi_vulkan/Cargo.toml` | **`+ boyko_diag = { path = "../boyko_diag" }`** — the second new edge (F1/S2), justified in-file by the `boyko_sdf_math` precedent at `:44-49`, whose rationale block gains a `boyko_diag` row |
| `crates/boyko_rhi_vulkan/src/ffi.rs` | `+ VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x0000_0004`; `+ PfnVkResetQueryPool` |
| `crates/boyko_rhi_vulkan/src/device.rs` | enable `hostQueryReset` when advertised; load `vkResetQueryPool`; expose the capability |
| `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | the three verb bodies, beside `fetch_query_pair_ticks` (`:1249`); `GPU_ZONE_QUERY_FLAGS` + its `const _` WAIT_BIT assert (G2a) |
| `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | **new** — `GpuZoneRecorder`, slot ring, marks+seal, 2×2 label, `submit_epoch` |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | `TsWitness` → `GpuZoneWitness`; `write_zero_pair` + epilogue gap-fill deleted at the retirement rung; `CommandWitness` counters + `zone_open_order` at the `vkCmd*` sites (`:107-156`'s pattern) |
| `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | 4 + 1 brackets ported |
| `crates/boyko_rhi_vulkan/src/present/swapchain.rs` | `PresentModeConfig`; `:199` becomes a probed choice with a loud fallback |
| `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` | **retired at rung 7**, not before |
| `crates/boyko_render/src/profiling_bridge.rs` | **new** — `retire_gpu(world, recorder, render_epoch, frame_now)`, a host-called function (**not** a system — F14); `frame_now` is M13's second horn |
| `crates/boyko_app/src/runner.rs` | call `retire_gpu` at `:1320`, beside the `RenderEpoch` publication and **before** the 0x0 `continue` at `:1328` (M13b); `flush_gpu` on the teardown path (`:261`); `claim_lane()` -> `LANE_HOST` at boot; **the `VB-P1d` / `VB-P4` print sites (`:3224`, `:3231`, `:3256`, `:3272`), the harness bodies and the statistics helpers deleted at rung 7** (S1) |
| `crates/boyko_app/src/gpu_scene/mod.rs` | env arming → `ProfilerConfig`; the `vb_timing_for_frame`-shaped predicate becomes a scope test |
| `crates/boyko_app/src/profiling/{reduce,artifact,stream}.rs` | **new, and this row is a MULTI-RUNG UNION — annotated per third at rung 7's re-measurement**: `reduce.rs` (`WindowReducer`, no console form) and `artifact.rs` (TOML, analysis feature) are **rung 7**, because rung 7 cannot migrate six consumers onto a file nothing writes; `stream.rs` (framed binary + its `.bss` double buffer and file handle, S5/M8) is **rung 13** (`:148` assigns it there). The row carried no rung at all while its neighbours did — `:61` "retired at rung 7", `:66` "new, rung 15" — which is how the writer came to be read off rung 8's row instead |
| `crates/boyko_ui/src/profiling_overlay.rs` | **new, rung 15** — reference overlay over `Res<Profiler>` + `ProfiledZone` |
| `crates/boyko_demo` | `profiling_partition!(User)` at the crate root (it is a GAME, B3); overlay wiring + a console command driving `commands.entity(e).enable::<ProfilingScopeEnabled>()` (the game-facing acceptance path) |
| `tools/prof_decode` | **new** — the telemetry stream decoder; the only reader of the binary format |

**Two new Cargo edges for this plan** (`boyko_threadpool -> boyko_diag`,
`boyko_rhi_vulkan -> boyko_diag`), plus `boyko_ecs -> {boyko_diag, boyko_log}` which the two plans
share — all downward, all in-house, all argued in §Crate graph
(`profiling/budgets-and-invariants`). Rev 2 listed **no** Cargo change at all, which is why three
of its rungs could not compile (F1); rev 3 listed two that pointed at the wrong leaf (S2).

**`Arena` / `ComponentPool` / `UnitId`: untouched, deliberately.** The profiler stores no
per-entity data. **A game's `DynZoneHandle`s, by contrast, DO live in ECS storage** (D27) — the
profiler does not store them; the game does, in a component or a `Resource`-owned column.

**Diagnostic codes (block `92xx`) — and WHO emits them.** `W9201` engine zone registry exhausted
(**warning, not error** — C-III/F5) · `W9202` GPU pair budget exhausted · `W9203` region overflow /
unclaimed drops · `E9204` profiler already bound to another world · `W9205` zones LOST this window
(**once per window, with a count** — F20) · `W9206` contrast NOT RESOLVED · `W9207` invariant TSC
absent (**the single invariant-TSC code for both subsystems**; the logging plan's `W0101` is
deleted — S4) · `W9208` engine registry ≥ 90 % · `W9209` late samples dropped · `W9210` user zone
budget or name arena exhausted · `W9211` fold working set exceeds L1d (`zone_stride` too large) ·
`W9212` `register_zone` refused an engine scope (< 32) · `E9213` re-arm with a different geometry ·
`W9214` telemetry path unwritable at boot · `W9215` telemetry write error, streaming disabled ·
`W9216` clock epoch break, window discarded, clock recalibrated · `W9217` GPU slots abandoned at
teardown · `W9218` telemetry quantile subscription refused past `MAX_TELEMETRY_QUANTILE_ZONES`
(M7).

**`profiling_abi` emits NOTHING (S5/S6).** The leaf is diagnostically mute: every `W92xx`
condition observed below or before the logger is a `boyko_diag::loss::raise(DiagFlag::…)` plus a
counter. **`boyko_ecs::…::profiling::fold` is the only emitter**, reading `take_raised()` at the
first fold after boot and emitting through `boyko_log`. This is what makes a `W9201` refused at
`ScheduleBuilder::try_build` — before `LogPlugin::build` has run — *late* rather than *lost*;
"boot the logger earlier" is unenforceable across every host and is not relied on. Rev 3's
`emit_diag`-as-`eprintln!` seam is deleted; **the profiler never prints, from any path** (S7).

**Per-rung registry obligation (S6).** The logging plan's L2 seeds all 18 rows of block `92xx` as
`Pending(<rung>)` in its code registry, and its check 2 (a doc page must exist) is narrowed to
`Live` rows only — otherwise L2 would owe eighteen pages for codes with no emitters, which is
doc-rot manufactured by a gate. **Every profiling rung that introduces a code carries three
explicit line items: flip its registry row `Pending → Live`, add `docs/diagnostics/<code>.md`, and
land one test that observes the code being emitted.** This is measurable rather than aspirational:
this corpus already contains the literals `boyko-W9207` and `boyko-E9204`, and the logging plan's
check 4 scans `docs/**.md`, so the rows must exist before these files can pass that gate.

---

## Implementation plan — every rung compiles the workspace alone

**Additive first, subtractive once.** Rev 3 amends rev-2 rungs in place rather than adding re-do
rungs, because nothing is built yet and an interim design deferred "for later" is the pattern this
project has been corrected on.

**Two cross-plan prerequisites (S5's landing order).** Rung 1 **requires `boyko_diag` D0**
(clock/lane/loss/storage) and **D1** (`boyko_threadpool → boyko_diag`, `set_lane` at its existing
sites — **three of them, not two**: `worker.rs:24`, `thread_pool.rs:190` and the load-bearing
`thread_pool.rs:279` in `InstallGuard::drop`, which covers the unwinding path; see
`substrate/lane-write-sites`). Rung 2 **requires `boyko_log` L3** (the sink, `flush`/`shutdown`,
`write_oracle_line`, the panic-hook chain and `PRE_FLUSH`), because the fold is what emits every
`W92xx`. Neither is this plan's to build; both are named so a rung cannot be started against a
missing seam.

**Nothing on this ladder runs at process start.** Every one-time cost the profiler has —
the `VmReservation` reserve/commit/publish/`mem::forget`, the clock calibration it consumes, the
telemetry file open, the scope registration — is inside `Profiler::arm`, and **`arm` IS the enable
path** (S13). No rung may move any of it earlier; a rung that did would be caught by G2's
flag-off legs on the logging side and by GJ1's control leg here.

| Rung | Content | Gate(s) landing with it | Compiles alone because |
|---|---|---|---|
| **1** | `boyko_diag::profiling_abi`: `ARM_MASK: AtomicU64`, two-region `ZoneLane`, `REGISTRY`, the 24 B `Sample`, `ZoneTier` + `GLOBAL_TIER` (from `boyko_diag/build.rs`), `profiling_partition!` + `ENGINE_PACKAGES`, macros; `boyko_threadpool → boyko_diag` edge + `set_lane` at `worker_main` / `install` / `InstallGuard::drop`. **Requires `boyko_diag` D0/D1** | **G1, G4a (`overflow > 0`), G7, G22a (`LANES` + `REGISTRY`)**, SPSC unit + property tests, the loom SPSC case | purely additive; `boyko_utils` keeps zero deps (F27: rung 1 no longer commits green with nothing exercising it) |
| **2** | `boyko_ecs::…::profiling`: `VmReservation`-backed store with an arm-time `zone_stride`, `fold.rs` (two regions, monotone-overflow delta, clock-epoch check, bidirectional walk), `arm`/`disarm`, `ProfilerPlugin`, world-bind check. **Requires `boyko_log` L3.** Flips the `W92xx` registry rows it emits from `Pending` to `Live` with their doc pages | **G4b (the `u64` accumulator + the consumer-side delta, = logging's G11), G21, G23a (`section_report{LANES, REGISTRY}` — the two statics that exist here)** | additive |
| **3** *(**COMPLETE** — **3a** the field, the mint, `W9201`/`W9208`; **3b** the `App` zones; **3c** the per-system spans; **3d** the analysis half)* | `SystemMeta.zone` + const-assert; tier-gated minting at `try_build` with **non-terminal** refusal; the four `App` zones (`__frame`/`__events`/`__fixed_step`/`__main_run`) at `update_with_delta`; the dispatch-round pair `__round`/`__round_width` **in place of `RoundRecord`**; `intervals` + `ConcurrencyReport` under `profiling-analysis`, **without `compat` and without `sys_of`** — see "What rung 3d SHIPPED" below for all four departures and their arguments | **G8, G9, G11 (engine half)** | one field in tail padding; four zone sites |
| **4** *(**SHIPPED**)* | RHI seam: three verbs + Vulkan impls + `ffi.rs` constants + `GPU_ZONE_QUERY_FLAGS` const-assert + Mock defaults and their pinning tests. **No consumer.** Plus `VkPhysicalDeviceHostQueryResetFeatures` (granular, for the VUID reason the descriptor-indexing struct already carries) and `DeviceCaps::host_query_reset` — see "What rung 4 SHIPPED" below | **G2a, G2c** | old readers untouched |
| **5** *(**COMPLETE** — **5a** the edge, `gpu_zone.rs`, the 2×2 label, **G2b**; **5b** `CommandWitness` behind `profiling-census` + **G5**; **5c** the VB port, the A/B and **G10**'s witness clause — see "What rung 5c SHIPPED" below for the four departures)* | `boyko_rhi_vulkan → boyko_diag` edge; `gpu_zone.rs` + `CommandWitness` (`zone_open_order` **and** `stamp_positions`, behind `profiling-census`); VB brackets ported. **Serial A/B against the old collector** (never both armed in one frame — F17) | **G2b, G5, G10** | both collectors exist; every existing test still compiles and passes |
| **6** *(**COMPLETE** — see "What rung 6 SHIPPED" below)* | gbuffer + SV0 ported through `GbufWitness`, the sibling `record_gbuffer` never had; `ZONE_BASE_VB`/`_GBUFFER`/`_SV0` const-asserted disjoint; the R0 collector given the host arming path it never had (`BOYKO_GBUF_BENCH`), which is what let G10's witness clause extend to these passes | **the port gate (ids in their own family range, no LOST/TORN) + G10's witness clause for the gbuffer/SV0 families**, both in `gbuffer_zone_port_gate.rs`, each with a run RED | additive |
| **7 (NOT purely subtractive — MEASURED; see "What rung 7 must BUILD before it can subtract")** | **First: `reduce.rs` + `artifact.rs` + a reader** (moved here from `:143`). Then delete `gpu_timing.rs` (713 lines), the runner harness bodies and the statistics helpers (**1381 lines, 31 % of `runner.rs`**) and the `VB-P1d`/`VB-P4` print sites — **ELEVEN `println!`s, not four**: five in `print_vb_bench_summary` (`runner.rs:3224`, `:3231`, `:3236`, `:3256`, `:3272`) and six in `print_sv0_bench_summary` (`:3837`, `:3850`, `:3862`, `:3871`, `:3879`, `:3885`). Four is right as a WORK ITEM only because both functions are deleted whole; it is wrong as a census, and a census is what `G24`'s grep performs. **And migrate the surviving five stdout consumers to the artifact** (S1; list below — `vb_bench_totality_gate.rs` is deleted, not migrated, see the list's note) | the post-rung `rg` gate **plus the S1 stdout gate (G24)** | **NOT "one commit, green before and after" as written** — the build half has no caller until the reader's round-trip test exists, so the rung lands as build-then-subtract |
| **7b (NEW — S1)** | **Floor re-measurement on the artifact channel.** Re-run A6's protocol (7 processes × 3 repetitions) reading the artifact instead of stdout; publish `docs/PROFILING-FLOOR.md` with the new `WorkloadTag`, all three repetition floors, and `FLOOR_REDUCTION = Max` | **G3a's reduction RED** | needs rung 7's channel; blocks nothing but *licenses* rung 8's verdicts |
| **8** | `Floor`/`Twin`/`resolve` + `NotResolvedReason`, present mode (**labelling only if `Immediate` is unsupported — D12**), counters at `vkCmd*` sites, optional `profiling-alloc`. ⚠️ **`WindowReducer` and the TOML artifact MOVED TO RUNG 7** — `03:477-478` says so in the reducer's own words (*"it is what lets rung 7 delete the stdout measurement channel, and it is why `vg_decidability_floor.rs` and its five siblings must be migrated in the same commit"*), `:142` calls the channel "rung 7's", and `G24` is annotated rung 7 while requiring a reader that refuses a stale artifact — a green leg that needs a writer. This row was the ONLY line assigning them to rung 8. `G4c` and the `NotResolvedReason` round-trip therefore become additions to an existing writer rather than its first caller | **G3a, G3b, G6, G13, G4c (the artifact clause), G25** | additive |
| **9 (v1.1)** | `VK_EXT_calibrated_timestamps` + rejection sampler; `cpu_gpu_offset` becomes a number with `max_deviation_ns` | — | additive |
| **10** | `dyn_registry.rs`: `DYN_DESCS`/`DYN_NAMES` static arenas + `SyncCells`, `USER_ID_NEXT`, `register_zone`, `DynZoneHandle`, `zone_dyn!`/`counter_dyn!`/`gauge_dyn!`, `zone_dyn_open`/`close` | **G11 (user half), G17, G20, G22b (`DYN_DESCS`/`DYN_NAMES`), G23b (the same two statics added to the residency sum — and the `MAX_USER_BUDGET` RED, which is not showable before this rung)** | purely additive; fold/store already index by `ZoneId` |
| **11** | `ecs_control.rs`: `ProfilingScopeEnabled` + `ProfilingScope`, `register_scope`, the **fold-step projection** (A8), the `Commands` write path, `ProfiledZone`, the `latency()` table | **G12** | additive; the mask exists from rung 1 |
| **12** | `lifetime.rs` + `hist.rs`: retention-tier-B accumulators (always on when armed) and retention-tier-C histograms (opt-in) | **G16, G18** | additive; both fold at the end of an existing fold pass |
| **13** | `stream.rs` framed binary telemetry + header/block/`ZoneRow`/`WindowRec`, `__telemetry_reduce` with its quantile cap, rotation, failure handling; `tools/prof_decode`; session identity + `fixed_elapsed_ns` | **G15 (incl. the torn-write clause), G9 (telemetry clause), G26** | additive; the decoder is a separate binary |
| **14 (= joint rung J1, merged with logging L17 — S9)** | The **single `BOYKO_PROFILE` axis**: 5 CI legs (`dev`/`editor`/`shipping`/`shipping-min`/`off`), per-profile sizing consts, the `profiling-analysis` `#[cfg]` split, the `compile_error!` on a stray per-knob override | **G14** — whose clause (a) is, like logging's G16, a cross-profile census run as a CI *step* over the `shipping` and `dev` legs' artifacts, not a sixth leg | a build-configuration rung; the workspace must be green in **all five** profiles. One axis cannot be split across two rungs, so this rung is shared |
| **15** | `boyko_ui/profiling_overlay.rs` + `boyko_demo` wiring (`profiling_partition!(User)`) + a console command calling `commands.entity(e).enable::<ProfilingScopeEnabled>()` | **G19** | additive; the acceptance path for the whole game-facing half |
| **16 (= joint rung J2 — S10)** | **The joint baseline sitting.** Re-take `zone_cost`, `fold_cost`, P1 and P2 in the **both-present** configuration, in one sitting; stamp every baseline file with `config_tag = {profiler, logger}`. **`GJ1` — the measured off-cost — runs in this sitting** | the `config_tag` clause on every regression gate; **GJ1** | whichever subsystem landed second must not be measured against a baseline taken without it. **Until this rung, the +25 % gate, the revert clauses and GJ1 record `UNPROVEN` and may not fail a rung** |

**Rung 7's consumer list, MEASURED this revision** (`rg` over
`TimestampCollector|VbTimedPass|Sv0TimedPass|TimedPass|PASS_COUNT`), **13 files** — rev 2 listed 15
names and omitted three production sites (F16):

| File | What names it |
|---|---|
| `crates/boyko_app/src/gpu_scene/mod.rs` | collector construction + arming |
| `crates/boyko_app/src/occlusion_force.rs` | pass enum |
| `crates/boyko_app/src/runner.rs` | the two env-var harnesses + statistics helpers |
| `crates/boyko_app/tests/sv0_deferred_term_bench.rs` | reads stdout |
| `crates/boyko_app/tests/vb_bench_totality_gate.rs` | **retired — DELETED, not migrated.** ⚠️ This contradicted the second list, which said it "reads the artifact". Resolved in favour of deletion, and the reason is that BOTH its gates lose their subject: gate A tests the totality epilogue, which rung 7 deletes with the collector, and gate B tests `disarm_vb_bench_unless_vb`, which rung 7 deletes with the bench. A file whose every gate has lost its subject has nothing to migrate |
| `crates/boyko_app/tests/vg_occ_split_timing.rs` | reads stdout; migrates to the artifact |
| `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` | **deleted** |
| `crates/boyko_rhi_vulkan/src/present/mod.rs` | **re-export at `:52-56`** — unlisted by rev 2 |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | recorder |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | **`use` at `:21`; the three public `Option<&'a …Collector>` fields, now at `:2633`, `:2645`, `:2702`, plus rung 5c's two — `gpu_zone` at `:2671` and `vb_cmd_witness` at `:2690`** — unlisted by rev 2. **(Anchor corrected twice, and the second time by an edit of my own: rev 4 wrote `:2645` for `vb_gpu_timing`, which was then `:2643` with `:2645` a doc-comment line; rung 5c inserted two fields between `vb_gpu_timing` and `sv0_gpu_timing` and moved all three. Re-verified against HEAD. A row that cites five line numbers in one file is a row that goes stale every time that file grows — it is kept because the FIELD NAMES beside the numbers are what a reader searches for when the numbers rot.)** |
| `crates/boyko_rhi_vulkan/src/swapchain.rs` | **re-export at `:14-16`** — unlisted by rev 2 |
| `crates/boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs` | migrates to zones |
| `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | migrates to zones |

**Rung 7's SECOND list — the six stdout consumers (S1), measured this revision.** Rev 3 treated the
collectors' *type* names as the whole surface. The measurement channel is the other half, and it
has six consumers, not one:

| File | What it consumes | Migration |
|---|---|---|
| `crates/boyko_app/tests/vg_occ_split_timing.rs` | `VB-P4 pass=…`, `VB-P4 regime …` | reads the artifact's per-zone rows |
| `crates/boyko_app/tests/vb_bench_totality_gate.rs` | printed totality lines | reads the artifact; its own mechanism is retired (replaced by G2a/G2b) |
| `crates/boyko_app/tests/vb_bench_query_validation.rs` | the printed line as a **liveness witness** that the reset and every timestamp write executed | the witness becomes `CommandWitness` + the artifact's label census — a *stronger* witness than a line's existence |
| `crates/boyko_app/tests/vg_decidability_floor.rs` | **the shipped bench's own stdout** (`:133-160`, `field_after`/`extract` over a `VB-P1d ` line) | reads the artifact. **This is why rung 7b exists**: it is the floor instrument, and its output is the input to D11's band |
| `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs` | the `VB-P1d` protocol it documents and drives | drives the artifact channel |
| `crates/boyko_app/tests/sv0_deferred_term_bench.rs` | printed lines transcribed into test source | reads the artifact |

**Both lists are LOWER BOUNDS, and the rung rests on neither.** Two mechanical gates: after rung 7,
`rg 'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` must return **zero matches**, and
`rg 'VB-P1d |VB-P4 pass=|VB-P4 regime|VB-SV0-S1\.5 ' crates/*/src` must return **zero**; the
workspace must be green with `--workspace --all-targets`. A list one file short fails a gate rather
than shipping. RED for the second: leave one `println!("VB-P4 pass=…")` in `runner.rs` — caught by
that grep and again by the logging plan's `print_census.rs`.

**And the consequence rung 7 has on numbers already published, stated where it happens:** the new
channel carries a new `WorkloadTag`, `Floor::from_session_file` is the only constructor, and
`resolve` already checks `floor.workload == a.workload` — so **every contrast returns
`NotResolved { FloorWorkloadMismatch }` until rung 7b re-measures the floor**. No new mechanism
enforces that; the existing tag check does. Any floor number published before rung 7 is invalidated
by `vg_decidability_floor.rs:27-30`'s own rule (*"a floor established on a different instrument
bounds nothing about this one"* — verified verbatim against HEAD), not by a choice made here.

**Ordering constraints:** 4 before 5 (seam before consumer) · 5 and 6 before 7 (the serial A/B is
what licenses the deletion) · **7b immediately after 7** · 8 after 7b (its verdicts are unlicensed
before it) · 10 after 2 · 11 after 10 · 12 after 2 · 13 after 12 · 14 after 13 · 15 after 11 ·
**16 last, after both subsystems are present**. Cross-plan: **rung 7 must precede the logging
plan's L8b**, whose 20 measurement-migration rows cease to exist because rung 7 already removed
their producers. Rungs 10-15 do not block 3-9 and may land interleaved; each is independently
green.

### Four rung-1 decisions taken at implementation, and what rung 1 still owes

Rung 1 landed in two commits: the ABI half (`profiling_abi` — tiers, `ARM_MASK`, the registry, the
guard) and this one, the **sample transport**.

1. **`push` cannot distinguish its three refusals, and does not try.** No lane, no buffer, no room
   differ in what a *host* should do about them, and that is a question for the report the fold
   writes — which has the lane table and the arm state to answer it with. A producer on the hot path
   has neither, and a discriminating return value would be three branches paid on every sample to
   carry information nobody reads there. The one refusal the transport genuinely cannot count is
   "no lane": there is no row to charge it to, so it lands on `boyko_diag::loss`'s un-laned row.

2. **`publish_region` is a CAS, not a store, and refuses a second buffer.** Publication is once.
   Replacement would strand every producer that had already read the old pointer — the pointer is
   deliberately never nulled, which is what lets a producer hold it without a lifetime, and that
   same property makes replacement unsound rather than merely surprising.

3. **`ENGINE_PACKAGES` is a `const` list with a `const fn` membership test**, so
   `profiling_partition!(Engine)` in a downstream crate is a **compile error** rather than a lint.
   `str` equality is not `const` on stable, hence the byte loop. Residual, named: a workspace member
   that lies is one greppable line; there is no per-site escape at all, which was the actual hole.

4. **`declare_zone!` deliberately writes `crate::__BOYKO_ZONE_PARTITION`, not `$crate::…`**, and
   carries `#[allow(clippy::crate_in_macro_def)]` with the reason. The lint's usual case is a bug;
   here it is the mechanism — `$crate` would resolve to `boyko_diag` and make every zone in the
   workspace an engine zone, silently, in the one field whose whole job is to keep a game's samples
   out of the engine's region. A crate that never wrote the partition line fails with an unresolved
   path, which is the intended outcome and is proved by `boyko_diag` having to write the line for
   its own tests.

5. **The region's `overflow` is MONOTONE, not `fetch_sub`-cleared** — `substrate/loss-fold`'s
   Q2(b) shape, applied rather than re-derived. The first version of the transport used
   `fetch_sub(observed)`. That was not *wrong*: this producer increments with an RMW, so the
   lost-update window Q2 describes — an owner's `load; add; store` overwriting a consumer's
   subtract — cannot open here. It was **a second shape for a question the substrate had already
   answered**, and a reader who had learnt the rule from `boyko_diag::loss` would have had to
   re-derive it. The counter is now `u64` and never cleared; `overflow_since` puts the delta at the
   consumer. Widened from `u32` for the same reason the logging plan widened its loss counters: a
   monotone `u32` has a wrap that is *unlikely*, and "unlikely" is not a statement a loss counter
   may rest on. **The layout pins caught the repair's own defect** — the padding was recomputed by
   hand and was wrong by four bytes, and `size_of::<ZoneLane>() == 256` said so at compile time.

**What rung 1 still owes, stated rather than absorbed: `G20`'s two-crate leg.** The transport-level
isolation is proved here — the `USER` region fills, refuses and counts while `engine_overflow` stays
0, and a closed engine zone reaches the `ENGINE` region and not the other. What is **not** proved is
the *macro*-level claim: that a zone declared by a `profiling_partition!(User)` crate lands in the
`USER` region. That needs a second crate in one process, because one crate can only be one
partition, and it is `G20`'s by construction. Until it exists, the partition constant's route from
a crate root to a sample's region is argued, not measured.


### Six rung-2 decisions taken at implementation, and what rung 2 still owes

1. **The fold DRAINS THE WHOLE REGION instead of stopping at the cut, and nothing is deferred.**
   A2 step 3 stops a region at the first sample with `stamp >= cut`, and states the cost: a long
   outer span written after a short inner one that opened past the cut waits a fold. That stop
   exists because A2 opens the new frame **after** the drain, so a sample past the cut belongs to a
   frame that does not exist yet. The implementation opens the frame **first**, so the live frame is
   open with `cpu_begin == cut` and a sample past the cut is attributed to it by the same walk on
   the same rule. Attribution is identical either way — a sample lands in the frame containing its
   stamp — and a slot is freed one fold earlier, which is strictly less overflow pressure. **The
   deferred-outer-span cost A2 states does not exist in this implementation.**

2. **`Q4`'s flag-to-code table is landed here, because this rung is the first `take_raised` caller**
   (measured: zero callers outside the substrate's own docs). `ClockEpochBreak` -> `W9216`,
   `LaneExhausted` -> `W9203`, and **`ClockUncalibrated` -> no code at all**. The third is a
   decision, not a gap: the `92xx` block is exactly eighteen dense rows and a nineteenth is
   un-addable without moving it, and the condition does not want a code anyway — its consequence is
   that the window's magnitudes are unscaled, which is a **status on the data**, reported as
   `FRAME_FLAG_CLOCK_UNCALIBRATED` on every frame record of the affected window. *Not every raised
   flag deserves a code; a flag whose consequence is a status on the data is reported as that
   status.*

3. **`G23a`'s domain 1 asserts ZERO, not `> 0`.** The gate's "each domain > 0" is the right instinct
   applied to the wrong domain: the std-allocator domain exists to catch the profiler reaching for
   the heap, and the design's whole claim is that it never does, so `> 0` would demand an allocation
   in order to prove there are none. The two-sidedness is kept and moved to domains 2 and 3, where a
   stub that reserves nothing and links no static still fails.

4. **`G23a`'s domain 3 is measured with `size_of`, not with `section_report`.** The tool proves
   **`.bss` residency** — no raw data in the image — while this gate needs the symbol's **bytes**,
   which for a `static` array are a compile-time constant. Re-measured this rung: no `llvm-readobj`,
   `objdump` or `nm` on `PATH` under the active toolchain, so under the literal reading the row
   could not be green on this box at all. Splitting them makes the bound exact and toolchain-free;
   the residency claim is **not made here** and stays `G22a`'s, where it remains RED for want of the
   tool. That RED is pre-existing — `rustup component add llvm-tools` is a D0 line item never taken.

5. **`FrameRecord` is 32 B here, not the corpus's pinned 88.** Every omitted field (`run_gross`,
   `fixed_total`, `main_total`, `instrument_*`, `gpu_total`, `fixed_steps`, `rounds`) is filled by
   the four `App` zones at rung 3 or by the GPU channel at rung 5. Same rule for the absent offsets
   (`lifetime`, `hist_of`, `hists`, `sys_of`, `rounds`, `legs`, `compat`, `intervals`), for
   `FrameState::Partial`, and for three of the five `CellLabel` variants: **a value that is
   structurally always zero is indistinguishable from a measurement of zero**, and a reader cannot
   tell the difference. The pin moves as each lands.

6. **The residency gate's own instrument was defective first, and it is the finding worth keeping.**
   The counting allocator counted into a process-wide `AtomicUsize`. Both tests failed, reporting
   136 B for `Profiler::new()` and 11 753 B for `arm` — figures with nothing to do with the
   profiler: `libtest` runs tests on separate threads, and a global counter read before and after a
   call reports whatever the *whole process* allocated in that interval. A direct probe measured
   `Profiler::new()`, `calibrate()`, a first `warn!`, a second `warn!` and `arm` at **exactly 0**
   each. The red was not the problem. **The same instrument would have gone GREEN BY LUCK had the
   scheduler placed the two tests further apart** — a gate whose verdict depends on thread timing is
   not measuring its subject. The counter is now per-thread.

**What rung 2 still owes.** `G7`'s clause (c), the JOIN — one fixture emitting one `warn!` and
opening one zone on the same worker, asserting the log record's `lane` field and the sample's lane
index are the same integer — is listed as landing at "profiling rung 2 / logging L5". It does not
land here: the log **record** carries no reader-visible `lane` field until the ring is read back at
L16, so the two halves of the equality cannot both be observed yet. Named rather than absorbed.

Also unshipped and named: **`W9207`'s emission has no reachable state on this box.**
`invariant_tsc()` is `true` on every x86-64 machine this project targets, so what is tested is the
*selection* (`diag::clock_code(false) == Some(9207)`, which reds if the mapping is deleted) and not
the emission. The code is `Live` because it has an emitter and a page; its firing is **UNPROVEN
here** and is stated as such at the site, in its doc page and in this row.


### Four rung-3a decisions taken at implementation

1. **A system zone has NO `ZoneDesc`, and that is the design.** A `declare_zone!` site registers a
   `&'static ZoneDesc` because it has one; a system's name lives in `SystemMeta.name`, and the
   window reducer walks the schedule it already holds. The alternative — a
   `SyncCells<ZoneDesc, ENGINE_ZONE_SLOTS>` arena, the twin of rung 10's `DYN_DESCS` — would be
   +96 KiB of `.bss` storing a copy of a string the struct already owns, which is the parallel-store
   shape Principle 0 refuses. Consequence, stated: `zone_desc(id)` is not total over ids. It never
   was — an un-minted id already answers `None`.

2. **The builder writes the id through a no-op-default `System::set_zone`, not through a
   `meta_mut`.** A `fn meta_mut(&mut self) -> &mut SystemMeta` obliges **every** implementor to
   provide one, including the ten test stubs in `boyko_ecs` that carry a `SystemMeta` only because
   the trait's field surface demands it. `set_zone` names exactly what the scheduler needs. The
   residual is named on the method and asserted by a test: a type that does not override it stays
   `ZONE_ID_UNASSIGNED`, which the artifact shows as an **absent** system rather than as a wrong
   number.

3. **Two `DiagFlag` bits were added, and the emitter's non-`_` `match` is what made it safe.**
   `ZoneRegistryExhausted` → `W9201`, `ZoneRegistryNearFull` → `W9208`, extending Q4's table in the
   one function that states it. Measured: the two rows were added **by a compile error**, not by
   remembering — `flag_code`'s exhaustive match refused to build the moment the variants appeared.
   That is what makes "every flag has exactly one paired report" a property of the build.

4. **`W9208`'s threshold is an exact equality, not a range.** `NEXT_SLOT` is monotone, so the mint
   that lands *on* 90 % is the one and only crossing; `slot == NEAR_FULL_SLOT` therefore fires once
   by construction, with no second piece of state remembering whether it already warned and nothing
   to get wrong when two threads mint concurrently.

**And one miss, recorded where it happened.** Rung 2 flipped seven registry rows to `Live` and was
verified with `cargo test -p boyko-log --test code_registry`, which selects ONE integration target
and does not build `codes.rs`'s **lib** tests at all. `every_pending_row_names_its_rung_and_the_live_set_is_pinned`
pins the `Live` set and was therefore **red for a whole rung**, found only when rung 3a ran the
crate's full suite. The lesson is not "run more tests": a target filter is a claim about coverage,
and this project already carries a standing note that `--test <name>` and `--lib` are different
worlds. The pin is now updated with all ten rows and the miss is written into the test itself.



### Rung 3c — the per-system span, and why it is not `zone!`

`zone!` takes a bare identifier and reads a `static ZoneHandle` plus its `mod` companion. A system
has neither: its id is minted at `try_build` into `SystemMeta.zone`, and its name lives in
`SystemMeta` rather than in a `&'static ZoneDesc` — rung 3a's decision. So the bracket is written
out as `SystemSpan`: the tier gate is a `const` read from `SYSTEM_ZONES_COMPILED` instead of from a
companion module, and the id comes from the meta instead of from a handle. **Everything else is
A1 verbatim** — the `&&` chain, the runtime scope test, one `rdtsc` at open and one at close, and
`Drop` as the closing discipline so a panicking system still closes its span.

**The guard opens INSIDE the spawned closure**, on the worker that runs the system, not on the
dispatcher. That is what charges the sample to the worker's own lane — and the lane is one half of
the pair the overlap analysis reads, so a span opened on the dispatcher would name a producer that
never ran the system. It is the same fact `G7(b)` states as a positive control, applied at the one
site that could get it wrong.

**Without this rung `SystemMeta.zone` would have been a minted number nobody reads** — an id
assigned at `try_build` and consumed by nothing, which is the "structurally always zero" shape one
step removed. RED, run at implementation: delete the `SystemSpan::open` at the concurrent dispatch
site ⇒ both systems' cells stay empty at a count the gate expects to be 3.


### What rung 3d inherits from 3a/3c — three of its specified structures are no longer needed

Recorded before 3d is written, because each follows from a decision already shipped and
re-deriving them at implementation time is how a rung ends up building storage nothing reads.

1. **`sys_of` (`[u16; zone_stride]`, 2 KiB, built at arm) is unnecessary.** It exists so the fold
   can turn a sample's `zone` into a `sys` before appending an interval. But rung 3a put the
   mapping in `SystemMeta.zone`, and the **schedule** owns it — so zone → system is resolvable at
   **report** time, by the one party that has the schedule, instead of being denormalised into a
   side table at arm by a party that does not. `F19c` asked for the table because rev 3's `sys`
   "was not derivable"; after 3a it is.

2. **The `compat` snapshot (1024×1024 bits = 128 KiB, taken at arm) is unnecessary in this
   engine.** It protects against the `ConflictGraph` changing under a window. This engine builds
   each schedule once, at `App::finish`, and never rebuilds it — so the report can read the live
   graph. **The residual, named rather than hidden:** if a later rung ever makes a schedule
   rebuildable, the report must either snapshot or refuse, and this paragraph is where that
   obligation is written down.

3. **`Interval.occ` needs no state of its own.** It is the occurrence index of a span within its
   frame, and the fold already has exactly that number in hand: `count[f*Z+z]` **before** the
   increment. Reading it there costs a load the fold performs anyway.

What 3d does still need, whole: the `profiling-analysis` feature, the `intervals` append ring (an
append and not an assignment — a system running N times per frame contributes N intervals, which
was `F19b`'s defect), `RoundRecord` (dispatch shape only: rounds per frame, wave width, round span,
**no membership mask, hence no truncation**), `ConcurrencyReport` and its serialisation index, and
`G8`.

**3d does not split further, and this is why.** The obvious cut is "land `RoundRecord` first" — but
`RoundRecord` is read by the report, so landing it alone puts 90.8 KiB of storage in the
reservation that nothing consumes. That is the shape this campaign refuses everywhere else; it
would be inconsistent to make an exception for it because the alternative is a larger commit.

### What rung 3d SHIPPED, and the one place it departs from the corpus

Shipped: the `profiling-analysis` cargo feature (**default ON**), the `intervals` append ring, the
`ConcurrencyReport` with its serialisation index and per-pair form, and `G8`. The three structures
the section above ruled unnecessary stayed unbuilt, and one more was replaced.

**`RoundRecord` did not ship. Two zone sites did, and this is the argument.** The corpus specifies
`RoundRecord { frame, round, dispatched, begin, end }` at 24 B × 121 × 32 = **90.8 KiB** of the
reservation, to keep *"dispatch shape only: rounds per frame, wave width, round span"*. All three
are per-frame statistics, and all three are cells the store already had:

| Corpus quantity | Where it is read now |
|---|---|
| rounds per frame | `__round`'s `count` |
| round span | `__round`'s `total` / `min` / `max` |
| wave width | `__round_width`'s `total` / `min` / `max` — a `Counter`, so `total` is Σ dispatched |

Four things follow, and only the last is a loss:

1. **90.8 KiB of the reservation is not spent**, and neither is the `rounds` drop class.
2. **`MAX_ROUNDS_PER_FRAME = 32` and its truncation are gone.** A schedule whose dependency chain
   is 33 rounds deep would have had its 33rd round *counted as dropped* rather than measured; two
   zones truncate at nothing.
3. **The write path exists.** This is the decisive half, not the arithmetic. The dispatcher does
   **not** hold `&mut EcsMaster` while a round is in flight — the cell it minted is shared with the
   workers — so a column write needs either a second published pointer into the reservation, from a
   thread the fold's `&mut` does not cover, or a per-schedule scratch buffer flushed later, which is
   profiling state owned by the scheduler. A lane push has neither problem and is the mechanism
   rung 3c already blessed for `SystemSpan`, one level down.
4. **What is lost, named:** the *correlation* between one round's width and that same round's span.
   Per-frame aggregates cannot answer "was the widest round also the longest one?". Nothing in this
   corpus asks that question, and the price of being able to is items 1–3. **Raised in
   `docs/OPEN-QUESTIONS.md` so the owner can reverse it.**

Two smaller departures, both consequences of decisions already shipped:

- **`Interval.sys` is `Interval.zone`.** The field holds a zone id, because `sys_of` is gone and
  zone → system resolves at report time. Naming it `sys` while it holds a zone is the kind of name
  this corpus exists to catch.
- **`G8` has no SKIP clause.** The corpus says the gate skips below two workers and CI fails on any
  non-zero skip count. `ThreadPoolBuilder::num_threads(2)` is clamped only to `[1, MAX_WORKERS]`
  (`crates/boyko_threadpool/src/thread_pool.rs`) and never consults the machine, so two workers are
  spawned on a single-core box as readily as on a sixteen-core one. A worker count below two would
  be a threadpool defect, not an environment to excuse — so the clause is an unconditional
  assertion with the reason printed, which is strictly stronger than a skip that has to be counted.
  The gate additionally **rendezvouses** the pair before the pinned 100 µs spin, so overlap is
  structural whenever the executor really dispatched them concurrently, rather than a coin toss on
  a loaded machine; the wait is bounded, so a serialising executor fails an assertion instead of
  hanging.

**Five REDs, each run rather than asserted.** (a) return immediately from `append_interval` ⇒ `G8`
fails at `frames_analysed >= 1` — worth stating exactly, because it is one step *earlier* than the
obvious guess: the report does not compute a serialisation index of 1.0 for a frame that ran in
parallel, it reports **no frames analysed** and `serialisation_index() == None`, which is the
corpus's "`observed` unavailable" literally. (b) assign instead of append ⇒ the `F19b` test and
`G8` both red. (c) delete `round.close(dispatched)` ⇒ the round pair's cells stay empty. (d) count
`append_interval`'s out-of-horizon return in `intervals_dropped` ⇒ the horizon test reds, which is
the whole point of separating a stated bound from a loss. (e) drop the full-bank
`intervals_dropped += 1` ⇒ the truncation test reds.

**A measured figure the ladder should carry.** `profiling_residency` now prints its configuration,
and on this box it reads **total 14 667 776 B (reservation 14 614 528, statics 53 248)** with
analysis ON, against the 16 MiB dev budget. The corpus's own dev rows are 6.67 MiB (analysis off)
and 7.05 MiB (on) — roughly **half** the measured figure, and the gap is **not** the interval ring
(262 144 B of it). It is `D8`'s `Z = 1024` against the shipped `ENGINE_ZONE_SLOTS = 4096`: the
columns are 21 B × 4096 × 121 = 10 407 936 B where the table budgets 21 B × 1024 × 121. That
contradiction was already recorded at rung 3a as `J1`'s to settle; this is the first time it has a
measured number attached, and the number says the dev budget has 1.3 MiB of headroom rather than
the 9 MiB the table implies.

### What rung 4 SHIPPED — and the three things measuring it corrected

The seam landed as specified: `read_query_pool_pairs_available` / `reset_query_pool_host` /
`host_query_reset_supported` on `boyko_rhi::RhiDevice` with `Unsupported` defaults, the Vulkan
bodies beside `fetch_query_pair_ticks`, `VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x4` and
`PfnVkResetQueryPool` in `ffi.rs`, `GPU_ZONE_QUERY_FLAGS` with its const-assert, and the Mock pins.
The three FROZEN readers carry a "no new callers" line each. **No consumer**, as the rung specifies.

**Both halves of the flag word are const-asserted, not one.** The corpus pins
`GPU_ZONE_QUERY_FLAGS & WAIT_BIT == 0`. A second assert pins
`& WITH_AVAILABILITY_BIT != 0`, because the two halves fail in **opposite directions**: `WAIT_BIT`
present turns an unwritten pair into a hang, and the availability bit *absent* turns it into a
confident wrong answer read out of the caller's own staging buffer. One assert covers one of them.

**Three things the measurement corrected, each found by running the gate rather than by reading:**

1. **`VkResult::is_success()` does NOT accept `VK_NOT_READY`.** It is `self.0 == 0` —
   `VK_SUCCESS` alone. `fetch_query_raw_ticks`'s comment had asserted for four rungs that
   `is_success()` *"would also accept"* `VK_NOT_READY`/`VK_INCOMPLETE`. **The claim was harmless
   where it was written and that is precisely why it survived:** `WAIT_BIT` makes both codes
   unreachable there, so no test could ever contradict it. It was contradicted the moment a
   non-blocking reader inherited it — G2c's first clause failed with
   `Vk("vkGetQueryPoolResults", VK_NOT_READY)` on the one poll the whole seam exists to make legal.
   The new reader tests `SUCCESS || NOT_READY` explicitly; the stale sentence is corrected in place
   with the reason it lasted.
2. **This box's driver DOES advertise `hostQueryReset`.** D18 says *"Nothing establishes that this
   box's driver does"* — something does now: `host_query_reset_supported = true`, MEASURED
   2026-08-09, and the host reset was exercised end-to-end (reset ⇒ every pair reads unavailable
   again). **The consequence is that the UNPROVEN half has swapped.** D18's specified fallback —
   `needs_cmd_reset` plus a recorded `vkCmdResetQueryPool` at the frame top — is the path this
   hardware never takes, so the Vulkan body's refusal branch is unproven here and is named as such
   in the gate. It is pinned where it *is* reachable: `MockDevice` enables no feature, and the
   `boyko_rhi` pin asserts the `Unsupported` error there.
3. **An empty GPU bracket on this box does not measure 0.** Two back-to-back
   `TOP_OF_PIPE`/`BOTTOM_OF_PIPE` stamps with nothing between them read **128 ticks at
   1 ns/tick**, both pairs. A pass that "cost nothing" and a pass that was never bracketed are
   therefore **not** distinguishable by a near-zero duration on this hardware; they are
   distinguishable by the availability byte, which is the seam's point.

   **CORRECTED at rung 5a, and the correction is the lesson.** This row originally called 128 *"the
   hardware lattice step `G`"*. It is not established to be: rung 5a's own empty bracket read
   **96**. Two identical readings do not establish a step — they establish a common multiple of
   one, and `gcd(128, 96) = 32` is as far as two observations take it. `read_query_pool_ticks`'s
   doc says exactly how `G` is obtained (*"observe that every raw delta shares a common factor"*),
   and two samples is not that observation. **The load-bearing half survives untouched: an empty
   bracket does not read zero.** The number does not, and no gate rests on it.

**G2a's census clause caught a file on its first run.** The pinned list was seeded from a grep over
the paths I expected; the census — which walks the whole tree — immediately named
`crates/boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs`, whose module doc describes the
blocking readback its harness performs. It is a correct use of a FROZEN reader (one bracketed pass,
one pair, read after the fence), so it is now a `PINNED` row with that reason written in it. The
episode is the argument for the census's shape: a hand-maintained list of "files I think call this"
is exactly what it replaces.

**G2c has no SKIP for a missing feature, only for a missing GPU.** The host-reset clause branches
instead: supported ⇒ exercise it, unsupported ⇒ assert the verb *refuses*. A capability that varies
by driver cannot be a skip, because a skip on the machine that has the capability and a skip on the
machine that lacks it are the same green.

### What rung 5a SHIPPED — and G2b as the corpus states it could not fail

`crates/boyko_rhi_vulkan/src/present/gpu_zone.rs`: the slot ring, the per-pair marks with their one
`Release` seal, the bump allocator, the two-horned retire with the **guarded** grace decrement, the
host-reset close with its `needs_cmd_reset` fallback, `flush` for teardown, and the 2×2 label. No
`CommandWitness`, no VB port, no ECS write — those are 5b and 5c.

**G2b, as the corpus writes it, is satisfied by a label that never reads the witness. MEASURED.**
The two specified clauses are *"an unbracketed pass yields `NOT_BRACKETED`"* and *"a bracketed pass
yields `MEASURED` with a non-zero duration"*. Replacing `begun` with `available` in the label match
— deleting the witness from the decision entirely — left the two-clause gate **green**, because on
those two inputs the witness and availability agree: a bracketed pair is available and an
unbracketed one is not. A gate whose entire subject is the difference between the two was testing
nothing.

**The third clause, and it is constructible.** On a working driver the witness and availability
disagree in exactly one reachable place: a pair whose BEGIN was recorded and whose END was not. Its
begin query is written, its end query is not, so availability reports `0` — the *same answer it
gives for a pass that never ran* — while the marks say `begun && !ended`, which is `TORN`. The gate
now records such a pair and asserts `TORN` plus `RetiredFrame.torn == 1`, and the availability
injection reds on it. The fourth row, `LOST`, needs a query that never returns and is **not**
constructible on demand against a working driver; it is pinned as a pure table in the module's unit
test and named as not-exercised-on-hardware in the gate, rather than implied by the three that are.

**The two horns and the `LOST` row are gated too, and `G2b` could not reach either.** `G2b`
exercises `Complete` alone — every bracketed pair comes back — which leaves the subtlest code in the
recorder ungated: the two independent deadlines, and the grace decrement between them that an
earlier form of this design executed as `0u8 - 1`.

`crates/boyko_rhi_vulkan/tests/gpu_zone_deadlines.rs` reaches all three. **`LOST` is constructible
after all**, one level up from where G2b looked for it: submit the pool **reset** alone and
fence-wait it, so every query is definitively unavailable, then record the brackets into a second
command buffer that is **never submitted**. The witness marks say begun-and-ended and the queries
were never written — which is precisely the state the blocking design could not express, because it
hung on it. (The reset is submitted rather than merely recorded: Vulkan leaves a never-reset query
undefined, and a gate reading undefined state is asking the driver a question instead of asking the
recorder one.)

Three clauses, each with its RED run: horn 1 spends exactly `RETIRE_GRACE_FRAMES` polls and then
retires `EpochDeadline` with the pair `LOST`; horn 2 fires on a **frozen** epoch, at `>` and not
`>=` — one frame short must not retire; and a poll below both deadlines with the grace already
spent must neither retire nor underflow. That last one is checked in the direction that matters:
after it, ONE epoch-true poll must retire immediately, because a grace wrapped to 255 would need
255 more — which is what distinguishes *"did not underflow"* from *"underflowed and nobody
noticed"*. **RED: move the epoch condition out of the `else if` (`} else if true {`) so the
decrement escapes its arm ⇒ all three fail.** A control was run first — rewriting the guard as an
equivalent `if grace == 0 { retire } else { decrement }` — and stayed green, so the RED is the
escape and not the edit.

**Two smaller decisions worth recording.** The retire buffers are one `RetireScratch` type rather
than five slice parameters — ~9.3 KiB the host holds once beside the recorder, because a retire that
allocated would be a profiler allocating on the frame path, and five separate slices made the
verb's length contract five preconditions instead of one type. And `VulkanCommandEncoder` gained a
`raw_command_buffer()` accessor: a timestamp is a **witnessed** command, so its `vkCmd*` site has to
be the recorder's own line, not the interior of an RHI verb — routing it through the typed surface
would put the command and its witness on opposite sides of a call boundary.

Golden `grand_showcase` byte-identical: the module records nothing yet.

### What rung 5b SHIPPED — and G5's disarmed clause needed a control the corpus does not state

`crates/boyko_rhi_vulkan/src/present/command_witness.rs`, behind `feature = "profiling-census"`
(**default off**): `profiling_cmds` / `query_resets` / `timestamps` / `recorded_pairs`,
`zone_open_order` (the record-order witness) and `stamp_positions` (the vocabulary-free cross-leg
witness rung 5c compares). Every counter is incremented **at the `vkCmd*` call site** and never
derived from the arming predicate — a counter derived from the predicate agrees with the predicate
by construction, which is the tautology `VbRecordProbe`'s own header names.

**The disarmed clause, as written, is satisfied by a dead instrument.** *"`profiling_cmds == 0` and
every sub-counter 0"* is equally true of a witness that was never threaded through anything — the
vacuous-green shape this campaign keeps finding, and the same defect class as G14's missing dev-leg
control. The gate therefore also asserts **`stream_pos > 0`** on the disarmed leg: the witness saw
the frame's ordinary commands and reported no profiling ones, which is a different statement from
"the witness saw nothing". Its own RED — deleting the `witness.command()` call from the scene loop
— fires with exactly that message.

**Three REDs, run.** (a) record one profiling command on the disarmed path ⇒ *"a disarmed frame
recorded a profiling command"*. (b) drop one real bracket ⇒ the `recorded_pairs ==
declared_bracket_count` equality fails. (c) stop threading the witness ⇒ the positive control fires.
Measured figures from the green run: disarmed `stream_pos = 12, profiling_cmds = 0`; armed
`pairs = 3, profiling_cmds = 7` (one reset + six stamps) with `stamp_positions =
[1, 6, 7, 12, 13, 18]` — each bracket spanning exactly the five commands recorded inside it, which
is the property that makes a bracket shifted by ONE command visible as a shifted position.

**One design change fell out of writing the gate.** `record_reset` took `&mut self` (only to clear
`needs_cmd_reset`) while every other recording verb takes `&self` — so a caller holding the recorder
shared for a frame's recording could not call it at the frame top, which is the one place it
belongs. `needs_cmd_reset` is now an `AtomicBool` and the verb is `&self` like its siblings. The
gate found it by being the first caller to record a whole frame through one shared borrow.

### What rung 5c inherits, and the two things measuring it settled before it is written

Recorded before 5c starts, for the reason the 3d handoff was: a rung that re-derives these at
implementation time either guesses at 168 call sites or invents a tolerance.

**1. `G10`'s TIMING clause is not evaluable at rung 5, and must not be approximated.** Its verdict
is `resolve`'s — *"per pass, `|median_old − median_new| <= band`, where
`band = max(floor, twin, se_floor, measured quantum)`"* — and `Floor` / `Twin` / `resolve` are
**rung 8's** content, listed as such in this ladder's own table. There is no band at rung 5.
Inventing one is the F6 mistake with the sign flipped: F6 killed *"one quantum"* because on this box
that is a tolerance of **0** and therefore unsatisfiable, and a band picked here to be satisfiable
would be picked to pass. **G10's own text already resolves this**: *"the witness clause, not the
timing clause, is what licenses the deletion"*. So 5c lands the witness clause and records the
timing clause as **deferred to rung 8**, where its band exists.

**2. The witness clause needs `witness.command()` at every recorded `vkCmd*` in the region, and
`vb.rs` has 168 of them.** MEASURED: 37 `cmd_bind_descriptor_sets`, 26 `cmd_bind_pipeline`, 22
`cmd_push_constants`, 20 `cmd_dispatch`, and 63 more across sixteen other verbs. This is not an
afterthought to the port — it *is* the bulk of the rung, and it is load-bearing: if only the
timestamps increment the counter, `stamp_positions` degenerates to `[0, 1, 2, …]` and carries no
information at all. The property the clause rests on — *"shifting one bracket by a single command
changes one entry"* — is exactly the property that dies when the non-profiling commands stop
counting. **The threading design is a real fork and belongs to 5c**, not to a guess here; the
candidates are a `&mut CommandWitness` threaded through every helper (invasive, but the counter is
where the command is), the witness living in the recorder behind the feature (one field, but
`&self` recording verbs would need interior mutability), or a macro wrapper around each `vkCmd*`
call (smallest diff, largest amount of magic in a hot recorder).

**3. The serial A/B is cheap, because `TsWitness` already has the shape.** It carries
`tc: Option<&VbTimestampCollector>` and is a no-op recording zero commands when `None` — which is
every golden frame. 5c adds a second `Option` for the `GpuZoneRecorder`, and F17's *"never both
armed in one frame"* becomes a structural property of the constructor rather than a discipline.

**5. 5c DOES NOT SPLIT, and this was measured rather than argued.** The obvious cut is "land the
`TsWitness` extension now, instrument the sites later". It was written and then reverted, because
the extension has **no caller**: `with_zone_recorder` arms the new leg, and nothing can call it
until `GBufferScene` carries the recorder and the harness passes one — 5 construction sites and 15
references to thread. Shipped without that, every new field and method is unarmed scaffolding, and
clippy says so as `dead_code`. Silencing it with an `#[allow]` would be exactly the shape this
campaign refuses everywhere else: *a value nothing can make move is indistinguishable from a
measurement of zero, and an offset nothing reads is an extent nothing can prove wrong.*

Two things the attempt DID settle and that 5c should not re-derive:

- **The pair index in `end` must be DERIVED, not allocated.** The bump allocator hands pairs out in
  OPEN order, so the k-th pass to open is pair k; `end` computes
  `(begun & ((1 << slot) - 1)).count_ones()` rather than calling `alloc_pair` a second time, which
  would allocate a fresh pair and leave the opened one unclosed — a `TORN` label produced by the
  port itself.
- **The VB passes have no minted zone ids, and inventing some would be a private vocabulary.** The
  schedule's mint is for systems; these are not systems. At this rung the zone id IS the
  `VbTimedPass` slot, which is honest (it names the pass) and is not a claim to be part of the
  engine-wide zone space.

**4. And the one thing that makes the comparison non-tautological: BOTH legs feed the SAME
`CommandWitness`.** The stamp positions are recorded by `TsWitness`'s own `begin`/`end`, at the same
line, whichever collector is armed underneath. If each leg had its own instrumentation the equality
would be comparing two instruments; with one, it compares two recorders. That is the same
distinction `stamp_positions` exists for one level up — it has no vocabulary, so no mapping table
can be wrong — and it applies to the instrument as well as to the datum.

### What rung 5c SHIPPED, and the five things measuring it refuted

`TsWitness` now carries both collectors and picks exactly one (`vb.rs`, `TsWitness::open`); the VB
brackets record through whichever is armed; `CommandWitness` counts **211 record sites** in `vb.rs`;
`GpuSceneBundles` owns the recorder behind `BOYKO_VB_ZONE`; and
`crates/boyko_app/tests/vb_zone_ab_witness_gate.rs` is G10's witness clause with a run RED.

**GREEN, measured:** 26 frames compared, 520 bracket timestamps, every position identical.
A steady `VisibilityBuffer × Mesh` frame reads
`stream_pos=68 profiling_cmds=21 resets=1 stamps=20 repairs=0 pairs=10`, positions
`[1,2,3,4,16,17,18,19,27,28,41,42,43,44,45,46,47,55,56,57]` — **the same list on both legs** — and the
zone leg retires every frame `cause=Complete lost=0 torn=0`. Four goldens (`vb_mesh`, `vb_mesh_tex`,
`vb_both_sdf`, `grand_showcase`) byte-identical after the port.

**1. The prescribed pair-index DERIVATION is WRONG, and would have swapped every END.** The 5c
handoff recorded as settled that `end` should compute `(begun & ((1 << slot) - 1)).count_ones()`,
because the bump allocator hands pairs out in open order. The premise is true; the formula does not
follow — it counts begun passes with a *lower slot number*, which equals the open index only if the
passes open in increasing slot order. **MEASURED: `record_vb` opens `0, 1, 9, 3, 4, 5, 6, 7, 8, 2`**
(`VbRun` third, `VbShade` last — `VbTimedPass`'s own leg table says so). For `VbRun` the derivation
yields **8** where the pair is **2**. Nothing at rung 5 reads a duration, so every END written into
another pass's query would have been invisible until rung 8 read numbers that were never anyone's.
`TsWitness::pair_of` REMEMBERS the index instead; remembering has no ordering premise.

**2. `G10`'s prescribed RED is not producible, because the port did not duplicate its sites.**
*"Shift a bracket by one command"* injected at a bracket site shifts **both** legs equally and stays
green: `TsWitness::begin`/`end` are one call each and dispatch to whichever collector is armed. That
shared site is the design's strength (the corpus's own point 4 — one instrument, two recorders) and
it makes the RED necessarily **leg-specific**. RUN: one `ts.cmd()` inside `begin`'s `zr` arm only ⇒
leg B reads `[2,3,5,6,19,21,22,24,32,34,47,49,50,52,53,54,56,64,66,67]` against leg A's list ⇒ red at
frame 4 with both streams printed. A control preceded it: the same file with no injection is green.

**3. `168` command sites was wrong in both of its numbers.** MEASURED at HEAD: **167** through
`self.fns.cmd_*` across **19** verbs (37/26/22/20 for the four the corpus names, then 15 more
summing to 62 — the corpus says "63 more across sixteen other verbs"), **plus 2** through
`crate::accel::cmd_*` helpers the `fns.cmd_` grep cannot see. Neither 167 nor 169 is 168. And the
instrumented total is **211**, not 169, because 42 more sites are calls to helpers that record
elsewhere (`record_vb_pass` ×34, the AA/post chain ×5, the 2 accel helpers, ×1 more) — a delegate
counts as ONE whatever it records inside, so `stream_pos` is a position in `vb.rs`'s record stream
and not a count of `vkCmd` calls. Rung 5b's *"every recorded command in the witnessed region"* was
wider than any instrument that does not also thread through the shared post-process recorders; the
doc now states the bound instead of the claim.

**4. The threading "fork" has a measured cheapest branch, and it is one parameter.** The handoff
called the choice between a threaded `&mut CommandWitness`, a witness in the recorder, and a macro
wrapper *"a real fork"*. Attributed to their enclosing functions, **all 167 direct sites live in
THREE functions** — `record_vb` (158), `record_hzb_poison_build` (5), `record_vb_viewt_dispatch` (4)
— and the first two already have the witness in scope. Exactly one function needed a new parameter.

**5. `first_pair_of` never existed.** Rung 5b shipped the member as `zone_open_order`; the corpus
named `first_pair_of` in 11 places across three files (`02-GPU.md`, `03-STATISTICS.md`, this file's
rung-5 row, `G10`'s row, and the touched-files table), and a `rg` for it returns nothing in any
`.rs`. Renamed here, and the sketch's `[ZoneId; …]` corrected to `[u16; …]` — `ZoneId` is not the
type either.

**And one gate that had been RED for five commits without anyone asking it.** `G2a`'s second clause
— the census of files naming `vkGetQueryPoolResults` — went red the moment rung 5a landed
`gpu_zone.rs`, whose module doc explains what the BLOCK cost. It stayed red through `ee9196b6`,
`cb54752d`, `8ca4e05b`, `cf8ffd20` and `7ae9162a`, three of which reported the workspace green. The
row is added with that history written into it: *an enumeration that is not executed is a list, not
a gate.* The mechanism was right; the running of it was the gap, and no gate can close that one.

#### What rung 5c did NOT do, stated so rung 6 does not re-derive it

- **The A/B is TWO PROCESSES, where G10's row says "in one process".** Not a shortcut: leg A's
  readback is `read_vb_bench_ns`, which waits with `VK_QUERY_RESULT_WAIT_BIT`. A single process
  alternating legs would reach it on a frame whose pool leg B recorded instead — **the hang class
  P4-1 removed**, and the one failure `vb_bench_totality_gate.rs` says it cannot convert into a red.
  One boot, one leg, makes it unreachable; `GpuSceneBundles::boot` asserts the two knobs apart.
  The **ABBA** ordering cancels temporal drift between the two legs' *timings*, and the timing clause
  is deferred (below), so there is nothing here for it to cancel.
- **`G10`'s TIMING clause stays deferred to rung 8**, exactly as the pre-5c note argued: its band is
  `resolve`'s and `Floor`/`Twin`/`resolve` are rung 8's content. The witness clause is what G10's own
  text says licenses the deletion.
- **The frames the comparison trusts start at 4.** MEASURED: frames 0–2 read `stream_pos=70` with
  positions starting at 3, frames 3+ read `68` starting at 1 — the first frames have no previous
  depth pyramid and record a different set of passes. Identical on both legs, but comparing them
  would make the gate's subject "does frame 0 look like frame 0".
- **The `profiling-census` feature moved what it gates.** It now covers the ~211 increments, not the
  `CommandWitness` type or `GBufferScene`'s `Option<&CommandWitness>` field. Features unify per
  PACKAGE: a `#[cfg]`'d field appears or vanishes for `boyko_app`'s construction site depending on a
  flag no `boyko_app` source names, and the workspace then stops compiling for a reason no crate
  shows. `boyko-app` and `boyko-render` gained forwarding features so the gate can turn it on.
- **The gate's own first live run was a false red, from its own parser.** `line.find("pairs=")`
  matches inside `repairs=0` and read the repair count as the pair count, reporting "every `pairs=`
  is 0" against a line printing `pairs=10`. `key_u32` is token-anchored now. Recorded because it is
  the census's own defect class one level up: an instrument wrong about which number it was reading.

### What rung 6 SHIPPED, and the one thing it could not do without a scope decision

`record_gbuffer`'s ten bracket sites now record through `GbufWitness` — the carrier that file never
had — so the four software-ray passes and the SV0 marcher reach the `GpuZoneRecorder` on the same
terms `record_vb`'s ten do. `gbuffer.rs` counts **125 record sites** (98 `fns.cmd_*`, 25 delegates,
2 `crate::accel::*`), all in `record_gbuffer`. `crates/boyko_app/tests/gbuffer_zone_port_gate.rs`
gates it with a run RED.

**GREEN, measured:** 30 retired frames on `Deferred × Both`, 30 gbuffer-family and 30 SV0-family
brackets, every id inside its own base range, no `LOST` and no `TORN`. Five goldens across three
render paths (`grand_showcase`, `deferred_mesh_only`, `forward_mesh`, `vb_mesh`, `vb_both_sdf`)
byte-identical, and rung 5c's G10 still green at 26 frames / 520 timestamps.

**1. Zone ids needed FAMILY BASES, and the reason is a collision that already existed.**
`TimedPass::DdgiUpdate`, `Sv0TimedPass::Marcher` and `VbTimedPass::CullReset` are **all slot 0**, and
the first two are recorded into the SAME frame's ring slot by the same `record_gbuffer`. Rung 5c's
*"the zone id IS the `VbTimedPass` slot"* is complete for one family and stops naming a pass at two.
`ZONE_BASE_VB` / `ZONE_BASE_GBUFFER` / `ZONE_BASE_SV0` are const-asserted disjoint, and each family's
width is const-asserted at the seam beside the enum it constrains — two separate assert items, not
one conjunction, because `SV0_PASS_COUNT <= PASS_COUNT` makes a conjunction's second half dead and
clippy says so. **RED, run:** drop `ZONE_BASE_GBUFFER` from the alloc ⇒ ids appear in the VB range on
a Deferred frame ⇒ the gate names the id and the family.

**2. One recorder and one slot per FRAME — so `vb_gpu_zone` lost its prefix.** A frame records either
`record_vb`'s brackets or `record_gbuffer`'s, and inside the latter two families share the slot. The
field is `GBufferScene::gpu_zone`.

**3. The disarm had to be REMOVED, and its removal is the finding.** Rung 5c gave the zone leg
`vb_bench`'s boot-time disarm, on the argument that *"its writers are the same `record_vb` brackets,
so a path that cannot feed the collector cannot feed the recorder either"*. True of a recorder only
`record_vb` wrote; false the moment rung 6 ported `record_gbuffer`. Left in place it would have made
the entire rung-6 port unreachable on every path that can reach it — scaffolding with no caller,
wearing a predicate that looked like a safety property. `vb_zone_disarmed` is deleted rather than
left `false`: *a field nothing can make move is indistinguishable from a measurement of zero.*

**4. `record_gbuffer` had no carrier at all, and `Sv0TimedPass`'s own doc says what that costs.**
Ten bare `if let Some(tc) = scene.gpu_timing` sites, no witness, and — verified — **no
`write_zero_pair` anywhere in the file**, so no totality epilogue. `Sv0TimedPass::Marcher`'s doc:
*"A render path that does not dispatch the marcher therefore leaves this pair UNWRITTEN, which would
hang the `WAIT`-bit readback — the caller must only arm this collector on a marcher-carrying path."*
The same holds for `DdgiUpdate` (bracketed inside `scene.ddgi_update`'s arm) and for
`CsmDepth`/`PunctualDepth`. What stood between the R0 harness and an infinite wait was the harness's
own configuration. `GbufWitness::finish` deliberately does **not** invent a totality fill for the old
legs: repairing that hazard at the rung that replaces it would hide it from the gate meant to show
it. The zone leg has no epilogue because it needs none — an unwritten pair retires `NotBracketed`.

**5. The 5c pair-index lesson transferred, and this file is where it would have bitten again.**
`Marcher` opens FIRST here, at the marcher dispatch, before every `TimedPass` — so `count_ones` of
the bits below a slot is not that slot's open index in `record_gbuffer` either. `pair_of` remembers.

#### The fork G10's extension turned on, and why it went the way it did (owner: *"whichever is more
#### performant and optimal"*)

`TimestampCollector` was constructed in exactly **two** places in the tree —
`boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs:367` and
`.../tests/window_present_gbuffer.rs:9265` — and **never from `boyko_app`**, so the two-process A/B
had no worker to play leg A for the software-ray family. Two branches:

- **(a) give the R0 collector a host arming path** — one boot-time `Option` that is `None` in every
  shipped run, and one predicate in `GpuSceneBundles::scene`, which is exactly what `vb_bench` and
  `sv0_bench` already are. Zero hot-path cost, zero recorded commands, and it **sunsets itself**:
  rung 7 deletes the collector and takes `BOYKO_GBUF_BENCH` with it.
- **(b) move the A/B into the RHI test that owns the collector** — that test builds its scene ONCE
  for 220 frames while the zone ring slot changes per frame, so the scene would hold
  `&GpuZoneRecorder` across the loop and `open_frame` would have to take `&self`. `retire` would
  follow, because `&mut self` is unexpressible while that shared borrow lives. **That deletes clause
  (c) of `FrameSlot`'s `Sync` argument** — *"`retire` takes `&mut self`, so no recording call can be
  in flight against the same slot"* — permanently, in shipped code, and pushes `set_mark` toward a
  locked read-modify-write in a hot recorder (principle 4's ban, in the one place the campaign has
  been most careful about).

**(a).** The decisive asymmetry is *where the cost lands*: (b) charges the shipped recorder for a
test's borrow shape, and charges it forever; (a) charges boot-time state that is `None` in every run
that is not the gate, and expires one rung later.

**And the witness clause needs no readback at all**, which removes (a)'s only real hazard: the R0
collector's `read_query_pool_ns` waits with `VK_QUERY_RESULT_WAIT_BIT`, and three of the four
software-ray passes are bracketed inside their own `if let` arms — a frame without DDGI or without a
spot light leaves those queries unwritten and the read would hang. G10's witness clause compares
stream POSITIONS; the timing clause is rung 8's. So the leg is **armed and never read**. Rung 8's
reader must consult the witness masks before it waits on anything.

**Two things the extension measured that rung 5c's A/B could not have.**

- **The old side records TWO pool resets per frame and the zone side records ONE**, so every bracket
  position on leg A sits exactly one record site higher. Not a defect — it is the difference the
  port *makes*, one recorder with one pool replacing two collectors with two. A plain position
  equality reds on it, and rung 5c never saw it because there one collector faces one recorder. The
  gate's clauses are therefore (i) the per-frame offset is CONSTANT across the frame's brackets — no
  bracket moved relative to its neighbours — and (ii) that constant EQUALS `resets_A − resets_B`,
  read from the census's own counters. Comparing `p[i] − p[0]` would have satisfied (i) alone and
  accepted any prologue difference at all.
- **The OLD knobs are not exclusive with each other, and being stricter than the truth broke the
  gate.** The first form of `boot`'s assert refused `BOYKO_GBUF_BENCH` alongside `BOYKO_SV0_BENCH`.
  But the zone leg brackets *every* family `record_gbuffer` records, so a leg-A worker arming only
  one old collector records fewer brackets and the comparison reds on the ARMING rather than on the
  port. The real exclusivity is zone-versus-old, which is what `GbufWitness::open` already says in
  code (`old_armed = tc.is_some() || sv0.is_some()`, not an exclusive-or).

**RED, run:** one `ts.cmd()` inside `begin`'s zone arm, gbuffer family only ⇒ *"bracket timestamp 2
sits 0 record site(s) apart while the frame's other brackets sit 1 apart"* — the gate names the
timestamp that moved and the family it belongs to, and the SV0 brackets in the same frame stay put.
**GREEN:** 28 frames compared, 112 bracket timestamps.

### What rung 7 must BUILD before it can subtract, and the format nobody wrote

Measured at rung 7's opening, before a line was deleted, because the rung's own row was wrong about
its own shape.

**1. The ordering defect, verified from primary sources.** Rung 7 must *"migrate all six stdout
consumers to the artifact in the same commit"* and be *"green before and after"*. **No artifact
writer exists in this tree** — `crates/boyko_app/src/profiling/` is absent and no TOML writer exists
anywhere. Six tests reading a file nothing writes is not green. Four lines put the writer at rung 7
(`:142` calls the channel *"rung 7's"*; `G24` is annotated rung 7 and its reverse RED needs a reader
that refuses a **stale** artifact, whose green leg needs a fresh one; the six migration cells all say
*"reads the artifact"*; and `03:477-478` says it outright). **One** line put it at rung 8 — that row
— and its own file table did not corroborate it: the `{reduce,artifact,stream}.rs` row carried no
rung while its neighbours did, and it is a multi-rung union whose `stream.rs` third belongs to rung
13. The rows are amended above.

**2. The corpus never writes one line of TOML.** MEASURED: `rg '\[\[[a-z_]+\]\]|^\[[a-z_]+\]'` over
`docs/diagnostics/profiling/` + `SEAM.md` returns **zero**. What is actually specified is:
`schema_version` on a *"flat TOML"* (`03:144-145`), `p95_lo`/`p95_hi` (`03:484-485`), the measured
quantum trio (`03:165`, `03:283-285`), `sum = NOT_VALID (mixed stage)` (`01:496`),
`cpu_gpu_offset = UNCORRELATED` (`02:259-260`), *"per-zone rows"* (`:179`) and *"the artifact's label
census"* (`:181`). Everything else a consumer needs — median/mean/p95 as named fields, `n`,
`begin_off_ns`/`end_off_ns`, the per-zone label, the VB-P1d leg fields, the regime provenance, the
whole SV0 S1.5 block — is **SILENT**. So is the reader: no parser is named, placed, or given a
signature anywhere.

**3. `build_hash` and `SessionId` are a FORMAT gap, not a symbol gap.** Both ship at rung 0
(`boyko_diag/build.rs`'s `BOYKO_BUILD_HASH`; `SessionId` in `clock.rs`). What is missing is a layout
that places them — and `substrate/01-CLOCK.md:33`'s *"joins the two artifact headers"* resolves to
the binary stream's header and the **logger's**, not to this TOML.

**4. And `G24`'s reverse RED has a hole that is not a format question.** `SessionId` is *"minted once
at first touch"* **inside the child process**, so a parent test cannot know it in advance. The only
header field a parent can check is `build_hash` — constant across a whole session — so as written the
reverse RED detects an artifact from a **different build**, never a stale one from the previous child
of the same run. `vg_decidability_floor.rs` spawns 42 sequential children. Whatever the format
decision is, the staleness the gate claims to catch needs a per-run discriminator the corpus has not
specified.

**5. Ranked by blast radius, the judgement calls rung 7 cannot avoid** — recorded so the rung does
not make them silently: (i) **numeric precision**, because `vg_occ_split_timing.rs:916` reconstructs
the GPU tick lattice by GCD over *tenths* and full-precision `f64` collapses that GCD, sub-flooring
every band — its own doc measures the error at **32×** and calls it *"satisfying every assertion here
while under-stating the instrument's resolution by the whole lattice factor"*, which is a silent
false-win rather than a red; (ii) **file path and per-run uniqueness**, with 42 sequential children in
one test and no `BOYKO_*_FILE`-shaped knob anywhere; (iii) **one file = one sitting, one process, or
many appended runs**, which `G24`'s RED is *defined on*; (iv) who aggregates the 21 per-session
artifacts into `docs/PROFILING-FLOOR.md`, which 7b depends on; (v) whether `WorkloadTag` is an
artifact field at all — `resolve` checks it, 7b publishes it into markdown, nothing says the session
file carries it; (vi) what the artifact records when the device declines timestamps, which today is
an `eprintln!` three consumers key their third outcome on.

**These are VALUES calls, not perf forks, and they are the owner's** — see
`docs/OPEN-QUESTIONS.md`. A format guessed here would be a format the six consumers are then
rewritten against, and (i) alone can make every band gate pass while measuring nothing.

### Two rung-3b decisions

1. **Nothing the zones measure is copied into `FrameRecord`.** The corpus's record carries
   `run_gross`, `fixed_total`, `main_total`, `instrument_measured` and `fixed_steps`. Every one is
   **already in a cell**: `run_gross` is `__frame`'s `total` for that frame row, `fixed_total` is
   `__fixed_step`'s, `instrument_measured` is `__fold`'s — and `fixed_steps`, the substep count N,
   is `__fixed_step`'s **`count`**, because a zone that opens N times per frame counts N. Copying
   them would be a second statement of five facts the store already holds, written by a different
   code path, which is how two numbers for one quantity come to disagree. `FrameRecord` therefore
   does not grow at this rung and stays 32 B.

2. **`__fold`'s sample is always one fold further behind than `__frame`'s, structurally.** Its
   guard must close *after* the drain — otherwise the sample it produces is taken by the very drain
   it is measuring and attributed to the frame it was measuring. So it is pushed after that fold's
   drain has finished and waits for the next one. A reader comparing `__frame` and `__fold` in the
   same row would be comparing two different frames; the gate reads `__fold` one row further back
   and says why.

RED for the containment claim, run at implementation: move the `__frame` guard **above** the
`fold_frame` call in `App::update_with_delta` ⇒ the fold's span falls inside the frame's ⇒
`__frame` no longer opens once per frame in the row the gate reads ⇒ red. The instrument being
outside its own primary number is then a property of the call order, not an approximation.

---

## Metrics and validation

### Gates — every one has a showable RED and an explicit "cannot claim"

The second-pass review found **seven gates that could be GREEN while their claim was FALSE**, plus
one target row whose proof was a `debug_assert`. All eight are repaired below; the repairs are
marked **[F-fix]**. *A gate that cannot fail is worse than no gate*, so two rev-2 gates are deleted
outright rather than patched.

The third-pass review found four more defects of the same family, repaired here and marked
**[B*-fix]** / **[M*-fix]**: a gate whose two clauses contradicted each other so no RED existed
(G14/B5), two gates whose REDs were produced by a path the plan does not recommend (G11, G20 / B3),
two gates asserting mechanisms that land three and seven rungs later (G4, G22 / B6), a gate over a
domain that could not see the thing it bounded (G23a/G23b / M10), and a licensing clause that required a
hand-written table to compare its two sides (G10 / M12).

> **Count note, recorded rather than silently corrected.** The checklist in `profiling/06-DISPOSITIONS.md`
> says "28 showable-RED gates". The table below has **33 rows** (34 with `GJ1`) — 32 before the
> `G23a`/`G23b` split below, which adds a row without adding a base number — and neither the
> row count nor the count of distinct base numbers (26) is 28. The discrepancy is carried forward
> as known in-document arithmetic rot, not resolved in one direction: repairing doc-rot is as
> error-prone as writing it, and picking a number here would be a guess about which side rev 4
> intended.

| # | Claim | Showable RED | **CANNOT claim** |
|---|---|---|---|
| **G1** **[F-fix]** | A site above the tier ceiling (or with the feature off) emits **zero instructions** | **Two-sided, token-level:** (a) under feature-off, `zone!(NEVER_DECLARED_IDENT)` must **compile** — proving the expansion never names its argument; a `{ let _ = &$IDENT; }` else-arm makes it a build failure. (b) under feature-on the *same* source must **fail** to compile (`trybuild`). (c) an object-symbol census over the feature-off test binary shows zero references to `profiling_abi::ZoneGuard`/`record` | It cannot claim the *frame* got faster. It proves the instruction is absent from the binary, nothing more. Rev 2's "the macro cannot name the recorder" proved a **different proposition** and would have passed a `{ let _ = &IDENT; }` expansion (F8) |
| **G2a** **[F-fix]** | **No blocking GPU reader can exist** | `const _: () = assert!(GPU_ZONE_QUERY_FLAGS & VK_QUERY_RESULT_WAIT_BIT == 0)` — adding the bit is a **compile error**. Second clause: a source gate asserts the set of files naming `vkGetQueryPoolResults` equals a pinned list, so a new blocking reader in a new file fails by existing | It cannot claim the *driver* never blocks internally, only that this code never asks it to. Rev 2's grep scope (`gpu_zone.rs`, `profiling/**`) structurally excluded `rhi_impl/device.rs`, where the body must live (F3), and its behavioural red would have been a **hang**, which is not a showable red here (`vb_bench_totality_gate.rs:44-53`) |
| **G2b** | Label positive control | An unbracketed pass yields `NOT_BRACKETED` **and** a bracketed pass in the same frame yields `MEASURED` with a non-zero duration. A stub labelling everything `NOT_BRACKETED` fails clause 2 | It cannot claim the duration is *correct*, only that it is non-zero and labelled |
| **G2c** **[F-fix]** | Availability truth control | Poll before the fence ⇒ `available == 0` for every pair; poll after ⇒ `1` for bracketed, `0` for never-written. Flip `WITH_AVAILABILITY_BIT` to a wrong value ⇒ availability words are not written, `scratch` retains stale bytes, gate fails | Rev 2's version was passable by setting `WAIT_BIT` (the poll would block and the leg would **hang**, not fail). That escape is closed by G2a's const-assert, not by this gate — stated so the two are not confused |
| **G3a** **[F-fix]** | A delta below the band cannot return `Resolved` | A/A contrast (same code both legs) ⇒ `NotResolved { BelowBand }`. Shrink the band to a quantum ⇒ `Resolved` appears ⇒ gate fails. **Now unescapable in production too:** `Floor` has exactly one constructor (`from_session_file`), `FLOOR_SIGMA` is a `const`, and `resolve` checks `floor.workload == a.workload`. **Reduction RED (M11, lands at 7b):** a pinned three-floor fixture whose `min` is below and whose `max` is above an injected delta; with `Reduction::Max` the contrast is `NotResolved { BelowBand }`, with `Reduction::Min` it becomes `Resolved`. No other input moves | It cannot claim the *floor file* was measured honestly — only that the API cannot manufacture one. Rev 2 exported `Floor::from_aa_control(control, sigma)`, so production could hand `resolve` a one-sitting, caller-sigma floor and G3a constrained only the floor **the test** built (F4) |
| **G3b** | `Resolved` positive control | Contrast between a calibrated spin of K and 3K ticks ⇒ `Resolved`, `median_delta` within tolerance of 2K. `fn resolve(..) -> NotResolved{..}` fails here | It cannot claim `resolve` is right on *real* workloads; it is a synthetic with a known answer |
| **G4a** (rung 1) **[B6-split]** | A full region refuses and counts | Fill a region past capacity ⇒ `overflow > 0` and no sample is written past the cursor. Remove the capacity test ⇒ the producer overwrites unread slots ⇒ the SPSC property test reds | At rung 1 there is no fold and no artifact, so this clause claims **only** that the refusal is counted in the region — it is *not* the accumulator claim, and rev 3's single G4 silently reduced to exactly this at rung 1 |
| **G4b** (rung 2) **[B6-split]** — **the same gate as the logging plan's G11** (S8) | The fold's accumulation is lossless | Preset a lane's cell, drop N, assert the folded `u64` global advanced by **exactly** N, and that a **second** fold with no new drops advances it by **0**. **RED, rewritten at rung 2 with the mechanism it tests:** replace `overflow_since(lane, region, seen)` with `overflow(lane, region)` in `fold.rs` — fold the monotone total instead of the consumer-side delta ⇒ every fold re-adds every earlier refusal ⇒ the counter runs away. MEASURED: an injected 5 read 10 on the second fold. *(The row said "replace `fetch_sub(observed)` with `store(0)`" until `substrate/loss-fold`'s Q2 resolved to **(b)**, monotone counters with the delta at the consumer. There is no clear to replace: `loss.rs` ships with no `fold_into`, no `store(0)` and no `fetch_sub`, and their absence IS the argument. The claim is untouched; only its mechanism moved.)* | It cannot claim samples were *not* lost — it claims the loss is counted exactly. One gate serves both subsystems because the counter lives in `boyko_diag`. **The producer-side window it used to disclaim no longer exists**: `substrate/loss-fold`'s Q2 resolved to (b), the consumer never writes the cell, and there is therefore no interleaving between a clear and an increment to reason about. What it still cannot claim is that a *reader* acts on the figure, which is `G4c`'s |
| **G4c** (rung 8) **[B6-split]** | The loss reaches the reader | The artifact names every non-zero drop class with its `LossClass` and its count; zero a class in the writer ⇒ the artifact and the `DiagCensus` disagree ⇒ red | It cannot claim a *reader* acts on it |
| **G5** **[F-fix]** | Command census, two-sided | Disarmed ⇒ `profiling_cmds == 0` and every sub-counter 0. Armed ⇒ **`timestamps == 2 × recorded_pairs` and `recorded_pairs == declared_bracket_count`**. Record one profiling command on the disarmed path ⇒ clause 1 fails; drop one real bracket ⇒ clause 2 fails | It cannot claim the *pixels* are unchanged (golden pins do that, secondarily) — and golden pins cannot claim the *commands* are unchanged, because `PINS.toml:3` is a BMP SHA-256 (verified: *"Each pin records the SHA-256 of a dumped BMP plus the exact pipeline it was blessed under"*). Rev 2's armed clause `timestamps >= 2` was satisfied **by the instrument's own `__gpu_null` probe alone**, so a recorder that dropped every real bracket passed |
| **G6** | Partition check | A `PartitionGroup` containing a `TopOfPipe` member refuses to sum: declare a TOP zone into `PartitionGroup::VbRun` ⇒ the window reducer prints `sum = NOT_VALID` and the test asserts it. **Second clause (S5):** the window reducer has no API that adds two reduced values — a test that tries to must fail to compile | It cannot claim a `BottomOfPipe`-only sum is *complete*; a pass nobody bracketed is `NOT_BRACKETED`, not missing |
| **G7** **[F-fix]** | Unclaimed refusal **and correct lane attribution** | (a) Emit from an unclaimed `std::thread` ⇒ `unclaimed_drops > 0` **and** no lane cursor moved; routing unclaimed threads to lane 0 fails clause (a). (b) **Positive control:** a zone emitted on worker `k` lands in `LANES[k]` and nowhere else — deleting the `set_lane` call in `worker_main` makes every worker read `LANE_UNCLAIMED` and fails (b). (c) **JOIN clause, new in rev 4 (S3):** one fixture emits one `warn!` and opens one zone on the same worker; the log record's `lane` field and the sample's lane index must be the **same integer**. Give the logger its own registry back ⇒ they differ ⇒ red. (d) 200 short-lived threads ⇒ per-thread allocations on first emit == **0**; reinstate a `Drop`-guarded TLS ⇒ 1 ⇒ red | It cannot claim the *host* lane is claimed in every host configuration — a host that never calls `claim_lane` drops, which is (a)'s behaviour and is counted. Clause (b) is what would have caught rev 2's contradiction between A1 step 4 and D2 (F12); clause (c) is what would catch two registries diverging. **Clause (c) lands at profiling rung 2 / logging L5, not at rung 1** — a JOIN needs both emitters to exist (`substrate/gates-dg`'s deferred list) |
| **G8** **[F-fix]** | Concurrency computability | A two-system schedule with a known conflict and a known-compatible pair ⇒ `declared` matches the conflict graph and `observed` is non-zero. **Configuration pinned:** the gate builds a pool with ≥ 2 workers and both systems spin a calibrated ≥ 100 µs, so the overlap is not marginal. If the pool has < 2 workers the gate **SKIPS with a printed reason, and CI fails on any nonzero skip count**. Discard intervals at fold ⇒ `observed` unavailable ⇒ red | It cannot claim the *serialisation index* of a real frame is accurate: `intervals` is an 8-frame ring with `INTERVALS_PER_FRAME = 2048` and an `intervals_dropped` counter, and it covers **one** schedule (`analysed_schedule`), with the rest in `systems_unanalysed`. Rev 2's version could be flaky-red on a small pool and self-overwrote a `Fixed` system N−1 times per frame (F19) |
| **G9** **[F-fix]** | Instrument disclosure, with **magnitude** | (a) `instrument_measured > 0` when armed; (b) `run_net < run_gross`; (c) **`instrument_measured >= instrument_zone_count × __cpu_null_median`** — a constant stub of `1` fails this; (d) a frame carrying a telemetry write has strictly larger `instrument_measured` than **both** neighbours by more than the band | It cannot claim the profiler's *total* perturbation is known. `instrument_estimated` (`zone_count × zone_cost`) comes from a different binary and is **printed, never subtracted** (F18). Rev 2 had only (a) and (b), which a constant `1` satisfied |
| **G10** **[F-fix]** | The old and new GPU collectors agree well enough to license the deletion | **Serial, not simultaneous** (F17): K frames with only `VbTimestampCollector` armed, then K frames with only `GpuZoneRecorder` armed, in one process, ABBA-ordered. Verdict is `resolve`'s: per pass, `\|median_old − median_new\| <= band`, where `band = max(floor, twin, se_floor, measured quantum)` — **not "one quantum"**, which on this box is a tolerance of **0** and therefore unsatisfiable (F6). Plus **`CommandWitness::stamp_positions` must be identical between the two legs** — same timestamp count, each at the same position in the recorded command stream (D17/M12). RED: shift a bracket by one command ⇒ one position differs ⇒ red before any timing is consulted. **⚠️ AS WRITTEN THIS RED IS NOT PRODUCIBLE, measured at rung 5c:** the port did not duplicate the bracket sites — `TsWitness::begin`/`end` are one call each, dispatching to whichever collector is armed — so a shift injected at a site moves BOTH legs and stays green. The RED must be LEG-SPECIFIC (one extra witnessed site inside `begin`'s zone arm only), which is what `vb_zone_ab_witness_gate.rs` runs | It cannot claim the two collectors are *bit-equal*: they write different queries in different pools, and the P4-6 lesson is that timestamps cannot license record-order conclusions. **The witness clause, not the timing clause, is what licenses the deletion**; the timing clause is a magnitude sanity check with a band. **Rev 3's witness could not have been compared at all:** `zone_open_order` is `[ZoneId; …]` and the old collector has only `VbTimedPass` slots (`gpu_timing.rs:229`, `VB_PASS_COUNT = 10` at `:391` — both verified), so the equality needed exactly the hand-maintained mapping table D6 rejects — and a table written alongside the port makes the equality a tautology. `stamp_positions` has no vocabulary, so no mapping exists to be wrong |
| **G11** **[B3-fix]** | A game cannot starve the engine — **id space**, with the RED produced by the **recommended** game path | The exhausting leg is a **static `declare_zone!` in a crate whose root says `profiling_partition!(User)`** (rev 3's leg used `register_zone`, which is not the path the plan recommends — so the gate's input class excluded the defect). Exhaust `user_zone_budget` until `W9210`, **then** mint a fresh engine `declare_zone!` from `boyko_ecs` and assert it succeeds with an id `< ENGINE_ZONE_SLOTS` and its samples land in the window. Key the partition on the macro instead of the crate ⇒ the user crate's static zones mint from `ENGINE_ID_NEXT` ⇒ the engine mint is refused ⇒ red. Second RED: delete the `profiling_partition!` line ⇒ the user crate does not compile (no default). **Third clause (C-III):** exhausting the *engine* range must **not panic** — it must return `DISABLED`, bump `zones_refused` and emit `W9201` once | It cannot claim a game gets the zones it asked for — only that its refusal is counted and does not propagate. It cannot bind an out-of-workspace crate that writes `profiling_partition!(Engine)`; that crate fails the `ENGINE_PACKAGES` const-assert instead (D6) |
| **G12** **[B2-fix]** | Scope toggle round-trip, two-sided, **through the path a game actually has** | With scopes A and B armed, an ordinary **parallel** system issues `commands.entity(a).disable::<ProfilingScopeEnabled>()` ⇒ **the next** frame has zero A samples **and** a non-zero count of B samples; re-enable ⇒ A returns. Clause 2: the same assertions through the direct `world.disable::<ProfilingScopeEnabled>(a)` path. Clause 3 **(the B2 red)**: make `ProfilingScope` itself the enable tag ⇒ it does not compile (`boyko_macros` rejects a fielded bitset tag); force the id through anyway ⇒ `is_enabled` returns `false` for every scope ⇒ the projected mask is 0 ⇒ **no samples of any scope** ⇒ red. An implementation that clears the whole mask passes clause 1 and fails clause 2; one that writes the wrong bit fails clause 1 | It cannot claim the toggle is *instantaneous*. The projection runs at the next fold, so the gate asserts the **next** frame, never the same one (D20) — which is true of **both** write paths, because the command applies inside the same schedule run. Stated in the API doc |
| **G13** | `resolve` refuses an incomplete window | Force a region overflow inside leg A ⇒ `NotResolved { WindowIncomplete }` with the delta fields still populated. Remove the refusal ⇒ `Resolved` on a truncated window ⇒ red. Sibling clauses: differing `epoch` ⇒ `EpochBreak`; a `LOST` label ⇒ `LabelNotMeasured`; a foreign `WorkloadTag` ⇒ `FloorWorkloadMismatch` | It cannot claim a *complete* window is trustworthy — completeness is necessary, not sufficient |
| **G14** **[B5-fix, PD3-fix]** | Tier folding is per-SITE, and the shipping build is not vacuous | **Three clauses, none contradicting another.** (a) *Per-site BY CONSTRUCTION, across two profiles:* a workspace-member fixture **bin** whose only dependency is `boyko_diag` — which has out-degree 0, so the fixture links no other emitter — carrying `profiling_partition!(User)`, exactly ONE `declare_zone!` at `tier = ZoneTier::Deep` and exactly ONE `zone!` on it, is built from the same source by the `shipping` and the `dev` CI legs. The `shipping` artifact (`GLOBAL_TIER = Always`) must carry **zero** references to `profiling_abi::ZoneGuard::open`/`record`; the `dev` artifact must carry **at least one**. A whole-binary census answers a per-SITE question here because the binary holds exactly one site: B5's objection — a census cannot attribute a reference to a site — is discharged by the fixture's SHAPE, not by a token test. RED: delete the `const { $h::TIER as u8 <= GLOBAL_TIER as u8 }` gate from the macro (D21) ⇒ the reference appears in the `shipping` leg ⇒ red. **The `dev` leg is the instrument's positive control and is not optional:** zero in *both* legs is `NOT RESOLVED (census inert)`, never green — and if `open`/`record` inline away in the `dev` leg the census has no subject at all, so the emission path must expose an `#[inline(never)]` symbol for the census to name. The control is what forces that choice at implementation time instead of discovering it after the gate has shipped green. Tool prerequisite and its rule are G22a's: absence of the census tool is a **RED, never a SKIP**. (b) *Behavioural liveness:* in the shipping binary, arm the profiler and assert `__frame`'s span count > 0 over 10 frames. RED: a ceiling that folds everything ⇒ zero samples ⇒ red — the clause rev 3 wanted, obtained from behaviour instead of from a symbol. (c) *Symbol census, but only where a census can answer:* the shipping binary must contain **no** reference to `ConcurrencyReport`, `resolve` or the TOML writer (all `#[cfg]`-removed with `profiling-analysis`). RED: leave the feature on in the shipping leg ⇒ the symbols appear ⇒ red | It cannot claim "zero cost in a shipping game": (a) and (c) prove code is absent, (b) proves the surviving path runs — none proves the frame got faster, and this box's own floor (6.3 / 14.3 / 4.7 / 13.5 %) makes a frame-time claim of that size undecidable. **TWO earlier versions of (a) could not run, for OPPOSITE reasons, and both are recorded here because the second was written as the first's repair.** Rev 3 asked ONE census over the application binary to report the recorder symbol absent (clause 1) and present (clause 2) at once; that binary holds many sites, and a census answers "is symbol S referenced in this object" per binary. Rev 4 replaced it with a token-level two-profile expansion test — a `Deep` `zone!(NEVER_DECLARED_IDENT)` compiling under `shipping` and failing under `dev` — which is unsatisfiable in the other direction: **a `macro_rules!` expansion is a function of its invocation token stream alone.** `zone!`'s only input is the identifier; the tier is a per-site property carried on `ZoneDesc.tier`, set by `declare_zone!`, and invisible to `zone!`. "Does the expansion name its argument" is therefore UNIFORM over every site in one build configuration, so that (a) demanded `zone!` drop the token for the `Deep` site AND for `__frame` — whose tier is `Always`, which D21's table ships, and which rung 3 declares as one of four `App` zone sites in the same shipping binary — killing (b). Nor does D21's gate license a token claim: `const { false } && …` deletes CODEGEN, not tokens, and `if false { UNDECLARED; }` is still `E0425`. **G1(a) is untouched and keeps its token form**, because a cargo FEATURE is uniform across the binary and can `#[cfg]` the macro DEFINITION, dropping the token before name resolution ever runs; a per-site tier cannot. **It also cannot claim anything about the RUNTIME flag** — (a) is the compile ceiling only; the runtime axis is GJ1's subject |
| **G15** **[M8-fix]** | The stream survives a kill **and a torn write** | (a) 900 frames with telemetry, `process::abort` mid-window, decode ⇒ N complete blocks, header + `ZoneRow`s parse, N equals the number of window boundaries crossed. Buffer across windows ⇒ the decoded count is short ⇒ red. (b) **New:** inject a writer that returns after `len/2` bytes of the last block (the `ENOSPC` shape) ⇒ the decoder returns N−1 blocks, reports `truncated_tail_bytes > 0`, and returns **no** record from the torn block; a decoder that accepts the torn block, or that fails to parse the whole file, is red. (c) The round-trip property is restated against the framing: re-encoding the decoded blocks equals the input minus `truncated_tail_bytes` | It cannot claim no telemetry is ever lost — only that loss is bounded by **one window**, and that a torn tail is *detected* rather than silently decoded. Power loss and a driver hang remain uncovered and no in-process gate can cover them. Nor can it claim `flush_on_panic`'s registrant bound ("no allocation, no lock, one `write_all`") in general — that is asserted per registrant (S5) |
| **G16** | Histogram fidelity | 10⁵ synthetic durations from a known distribution ⇒ `quantile(0.99)`'s bucket **edges bracket** the sorted-oracle p99, and the reported count equals the fed count. An off-by-one in the bucket index ⇒ the oracle falls outside ⇒ red | It cannot claim histogram quantiles can resolve a contrast. 6.25 % bucket width is the same order as the measured floor, which is why `resolve` does not consume histograms |
| **G17** | Dynamic path cost, one sitting | `zone_cost` reports static-armed ≤ 12 ns, dyn-armed ≤ 14 ns, static-disarmed ≤ 2 ns, dyn-disarmed ≤ 3 ns, script ≤ 18 ns — **all legs interleaved in one process**. Implement `zone_dyn!` with a `REGISTRY[id]` dereference to recover the scope bit ⇒ the dyn-armed leg exceeds 14 ns ⇒ red. Control: if the static leg regresses, the sitting is invalid and **no** dyn claim is made | It cannot claim the dyn path is fast *in a game*. It measures an isolated loop with the handle in a register; a real `DynZoneHandle` may be a cold load from a component. The bench measures the path's **floor** |
| **G18** | Lifetime accumulator agrees with the ring | Over 10 000 frames, `lifetime[z].count` equals Σ per-frame `count[z]` and `lifetime[z].max` equals max per-frame `max[z]`. Fold the lifetime row from the *previous* frame's row (after the ring overwrote it) ⇒ counts diverge ⇒ red | It cannot claim the accumulator is correct across an **epoch break** — a break discards the in-flight window by design, and the gate runs without one. A separate clause of G21 covers that |
| **G19** | The overlay read path is allocation-free | The reference overlay runs 600 frames under the counting-allocator gate ⇒ 0 allocations, **and** a control system that formats a `String` in the same test ⇒ > 0. Remove the control ⇒ the gate cannot distinguish "no allocations" from "the hook is not installed" | It cannot claim a *game's* overlay is allocation-free — only the reference one |
| **G20** **[B3-fix]** | **A runaway game scope drops ZERO engine samples** | The runaway loop emits from a **static `declare_zone!` site in a `profiling_partition!(User)` crate** — the recommended game path — until `user_overflow > 0`, while `boyko_ecs` emits a known number of `ENGINE` zones in the same frames ⇒ **every** engine sample is accounted for and `engine_overflow == 0`. Collapse the two regions into one ring ⇒ engine samples are lost ⇒ red. Second RED: key the region on the macro rather than the crate ⇒ the user crate's static sites write the `ENGINE` region ⇒ engine samples are lost ⇒ red | It cannot claim isolation under an **unclaimed** thread: a thread with no lane is refused entirely (G7) so it cannot overflow anything, and a mod spawning 100 threads exhausts the spares and has its zones refused and counted — different behaviour, separately gated |
| **G21** | Clock epoch break, **asserted on BOTH artifacts** (S4) | Inject a synthetic forward jump of 10 s into `boyko_diag::clock` ⇒ `clock_epoch_breaks == 1`, the in-flight window is discarded, `W9216` is emitted, the *next* window is complete, and no `FrameRecord` carries a duration above `MAX_PLAUSIBLE_FRAME_TICKS` — **and** every log record emitted after the jump carries the incremented `clock_epoch`. Remove the detector ⇒ a 10 s interval appears in `max` and in p95 ⇒ red. Give the logger its own `ticks_per_ns` back ⇒ its rendered wall times drift by the injected amount while the profiler's window is quarantined ⇒ the cross-check reds | It cannot claim a **backward** TSC jump is handled the same way; a backward jump produces a `value` computed from a larger `stamp`, which the `MAX_PLAUSIBLE_FRAME_TICKS` test catches at the frame level and which is counted separately |
| **G24** (rung 7) **[S1]** | **The stdout measurement channel is gone** | `rg 'VB-P1d \|VB-P4 pass=\|VB-P4 regime\|VB-SV0-S1\.5 ' crates/*/src` returns **zero**. Leave one `println!("VB-P4 pass=…")` in `runner.rs` ⇒ red, and red again in the logging plan's `print_census.rs`. Reverse RED: point a migrated consumer at a **stale** artifact ⇒ the header's `build_hash`/`SessionId` mismatch makes the reader refuse rather than parse | It cannot claim the artifact carries the same *numbers* the printed lines did — it is a different instrument, which is precisely why rung 7b re-measures the floor rather than reusing it |
| **G25** (rung 8) **[M13-fix]** | **A slot retires while submits are frozen** | Drive `retire_gpu` for N > `GPU_FRAME_DEADLINE` iterations with `render_epoch` **held constant** and `frame_now` advancing (the minimised-window shape) ⇒ every in-flight slot retires `Partial`, `gpu_frame_deadline > 0`, and the ring never exhausts. Remove the frame horn ⇒ slots stay in flight forever ⇒ `gpu_budget` climbs ⇒ red. Second RED: put the `grace` decrement back in the `else` arm and enter it with `grace == 0` ⇒ debug panic / release wrap to 255 ⇒ red | It cannot claim a *real* minimised window behaves identically — the gate drives the same function with a frozen epoch rather than driving the OS. What makes the two the same code path is the call site at `runner.rs:1320`, which is before the 0×0 `continue` at `:1328-1332` (both verified against HEAD); that placement is a plan decision, not something this gate proves |
| **G26** (rung 13) **[M7]** | **The telemetry window's total cost is measured, not assumed** | `telemetry_window` reports `__telemetry_reduce`, `__telemetry_write` and their sum separately at 64 quantile zones; the sum's p95 ≤ 350 µs. Subscribe 65 quantile zones ⇒ refused, `telemetry_zones_refused == 1`, `W9218` once. Remove the cap ⇒ subscribe 400 ⇒ the sum exceeds the budget ⇒ red | It cannot claim the spike is invisible to a *player*; it claims it is ~2.1 % of one frame in 121, which is below this box's own decidability floor and therefore below what the project can measure. If a title measures otherwise, D23's named escalation applies |
| **G22a** (rung 1) **[B6-split]** | `LANES` and `REGISTRY` are zero-initialised `.bss` | `boyko_diag::storage::section_report` (the ONE `llvm-readobj --sections` wrapper, shared with the logging plan — S12) shows the sections owning `LANES` and `REGISTRY` carry a size with **no raw data**. Initialise one element non-zero ⇒ raw data appears ⇒ red | It cannot claim `.bss` residency is *guaranteed*: PE/COFF placement is a toolchain behaviour, not a language guarantee; the gate pins today's toolchain. **Rev 3 listed `DYN_DESCS`/`DYN_NAMES` here too, but those symbols do not exist until rung 10** — the gate would have run against one symbol while its title claimed three. **And the tool is not installed on this box** — `substrate/section-report` MEASURED that no `llvm-readobj`/`objdump`/`nm` is on PATH under the active `stable-x86_64-pc-windows-gnu` toolchain, so tool absence is a **RED, never a SKIP**, and `rustup component add llvm-tools` is a D0 line item |
| **G22b** (rung 10) **[B6-split]** | The dynamic arenas are zero-initialised `.bss` | The same `section_report`, now over `DYN_DESCS`/`DYN_NAMES`. Second clause — the policy's own red (S12): a `#[test]` declaring a `.bss` array sized from a `ProfilerConfig` value must fail `assert_bss_eligible` **at compile time**; remove the const-assert ⇒ it compiles ⇒ red | same |
| **G23a** (rung 2) **[F-fix, M10-fix, B6-split]** | Resident memory is bounded **and allocated once** — over **three** measurement domains, the third narrowed to the statics that exist at this rung | In a test binary: `arm()` under the counting allocator, then assert `std_bytes + Profiler::reserved_bytes() + section_report{LANES, REGISTRY}.total` ≤ the profile's budget **and** each domain > 0 (two-sided — a stub that allocates nothing fails); then a **second** `arm()` after `disarm()` allocates **0** additional bytes (D15). RED for the bound, produced by a const this rung can resolve: raise `LANE_COUNT` from 80 to 256 ⇒ `LANES` becomes 256 × 256 B = 64 KiB in domain 3 and the sample slab becomes 256 lanes × 2 regions × 128 × 24 B = 1536 KiB in domain 2, so 64 KiB + 1536 KiB = 1600 KiB crosses the **1 280 KiB** shipping budget before the columns are added at all ⇒ red. **(The RED said "raise *shipping* `LANE_COUNT` from 32" until Q1 deleted the profile axis; the constant is now global and 80, and the RED is unchanged in everything but its starting point.)** ✅ **UNBLOCKED 2026-08-08 — the owner raised the shipping budget from 1 024 to 1 280 KiB.** The row sits at 1 208.2 KiB, so the bound assertion has a reachable green state again with 71.8 KiB of headroom. It was BLOCKED for one revision: Q1 put the row above the old budget, so the assertion failed **at the baseline**, before any RED could be applied — and a gate whose green state does not exist is as useless as one whose red state does not | It cannot claim the *steady-state* footprint of a shipped title, only the boot total for a given `ProfilerConfig`; nor driver-side query-pool memory, reported separately from `DeviceCaps`; nor the **joint** figure with `boyko_log`, which is `seam/joint-cost`'s row and not this gate's — and which this file therefore does not state (rev 4 stated it here as ≈ 1.99 MiB shipping; that figure was built on a logger half the logger's own files contradict). **Rev 3's two domains — the std allocator and the reservation — could not observe a static array at all**, so the ≤ 1 MiB row was green regardless of 234 KiB of `.bss`; and it named `VmReservation::reserved_bytes()`, which does not exist in the tree (`vm.rs:190` has `os_len()`, and the type is `pub(crate)` — verified). **It also cannot claim a flag-off residency**: what it measures is the ARMED total, and `arm` is the enable path. **And at rung 2 the third domain is not the four-symbol domain the claim names** — `DYN_DESCS`/`DYN_NAMES` arrive with `dyn_registry.rs` at rung 10, so what is bounded here is `LANES` **20 KiB** + `REGISTRY` 6 KiB = **26 KiB** against the shipping sizing row's four statics, 20 + 6 + 24 + 16 = **66 KiB** (both read 8 / 14 / 54 at 32 lanes, before Q1). **No `.bss`-only RED is showable at this rung**, which is why the `MAX_USER_BUDGET` RED is G23b's: raising shipping `MAX_USER_BUDGET` from 512 to 3072 grows `REGISTRY` by 2560 × 8 B = 20 KiB and moves nothing else here — `DYN_NAMES` is sized by the separate `DYN_NAME_BYTES`, and the reservation does not move because shipping's `user_zone_budget` defaults to 0. **Q1 changes the reason, not the conclusion:** taking the whole shipping row as the ceiling reaches 1 208.2 + 20 = **1 228.2 KiB, still under the 1 280 KiB budget** — so the 20 KiB does **not** cross on its own, and the split's original justification holds verbatim: no `.bss`-only RED is showable at rung 2. *(For one revision, between Q1 and the budget decision, the baseline itself was over and this clause read the other way. Both the number and the reason are restored.)* Rev 4 left this clause naming all four symbols at rung 2. That is not a link error — `section_report` takes a `&str` (`substrate/04-STORAGE.md`), so the two absent names resolve at run time, not at compile time — which means the gate would have **RUN** against two symbols while its text claimed four: B6's failure mode surviving one gate to the right of the gate B6 split |
| **G23b** (rung 10) **[B6-split]** | The same three-domain bound, now over the **complete** `.bss` set — and the budget RED that only the dynamic arenas can produce | The G23a assertion with `section_report{LANES, REGISTRY, DYN_DESCS, DYN_NAMES}.total` as domain 3; the each-domain > 0 clause and the second-`arm`-allocates-0 clause are unchanged and are re-run over the wider domain. RED: raise `MAX_USER_BUDGET` in the shipping profile from 512 to 3072 ⇒ `REGISTRY` grows by 2560 × 8 B = 20 KiB **and `DYN_DESCS` by 2560 × 48 B = 120 KiB**, so 1 208.2 + 20 + 120 = **1 348.2 KiB** crosses the **1 280 KiB** budget ⇒ red (the sum read 908.2 + 20 + 120 = 1048.2 against a 1 024 KiB budget before Q1 moved both). ✅ **Unblocked with G23a**: the 68.2 KiB by which the RED crosses is smaller than before, so the gate is *tighter* than it was — which is the honest consequence of a budget with less slack, not a weakening. The 120 KiB is the whole reason this RED waits for rung 10: it is `DYN_DESCS`, and `DYN_DESCS` is not linkable before `dyn_registry.rs` exists | Same as G23a, and additionally: it cannot claim that the rung-2 green covered this — the two greens bound different symbol sets, which is precisely why they are two rows. G22b proves those two arenas are `.bss`; this row is what puts their bytes inside the budget sum |
| **GJ1** (rung 16 = J2) **[S13, NEW at the split]** | **Turning the runtime flag off removes the subsystems' cost down to the per-site floor — and the per-site floor is what only the compile ceiling can remove** | **Three legs, ONE sitting, ABBA-counterbalanced, with an interleaved zero control, on the headless schedule bench (`crates/bench_bevy_vs_boyko`) — never on a windowed frame** (FIFO is unconditional at `present/swapchain.rs:199`, verified, so a wall-clock channel is structurally incapable of responding and its verdict would be pre-determined — the F3 lesson from logging's P1). **(A) FLAG ON** — profiler armed, logger enabled, at the shipping ceiling. **(B) FLAG OFF** — the *same binary*, same scene, flags absent; `ARM_MASK == 0`, every `CONTROL` byte `Off`. **(C) CONTROL LEG** — built with the const ceiling forced permissive (`BOYKO_PROFILE=dev` ⇒ `GLOBAL_TIER = Deep`, `GLOBAL_CEILING = Trace`) and the runtime flags **OFF**, so every site the shipping ceiling deleted survives and pays exactly one `.bss` load and one predicted branch. Verdict is `resolve`'s, same band, same `NotResolved{reason}` discipline, reported as three pairwise verdicts (A vs B), (B vs C), (A vs C). **THE RED, and the entire reason leg C exists: if C does not resolve apart from B, the instrument measured nothing** ⇒ report `NOT RESOLVED (control inert)` and record the free-when-off claim as **UNPROVEN on this box** — it is not restated. **Second RED:** delete the runtime gate from the emission macros so B becomes the same code as A ⇒ B collapses onto A and (A vs B) stops resolving. **Third:** move the sink-thread spawn back into `boyko_log::boot()` ⇒ logging's G2 leg (b) thread-count probe reds on the flag-off run while GJ1 itself may not move at all — which is why the memory and boot claims are gated by G2 and G3, not by GJ1 | It cannot claim a *frame* got faster: this box's decidability floor is 6.3 / 14.3 / 4.7 / 13.5 % (`docs/VG-DECIDABILITY-FLOOR.md`), so a frame-time claim below roughly 15 % is undecidable here. GJ1 bounds **CPU schedule work at a stated profiler/logger state** and nothing else. It cannot claim the MEMORY row — `.bss` residency is proved by `substrate/section-report` (absence of raw data in the **image**), and whether the OS commits an untouched page is **UNPROVEN and not asserted by anything in this corpus**. And it may not **fail** a rung before **J2**: a flag-off number taken without the other subsystem present is not a number about the both-present configuration, so before J2 it records `UNPROVEN` like every other regression gate |

**Deleted rather than patched:**

- **rev 2's `__gpu_null` quantum probe and every gate clause that used it** — measured-inert on this
  box (F6/D5).
- **rev 2's `vb_bench_totality_gate.rs`** — its mechanism (the totality epilogue) is deleted at rung
  7, so the gate would pass vacuously. Replaced by G2a + G2b.
- **rev 3's G14 recorder-symbol census OVER THE APPLICATION BINARY** — self-contradictory, no RED
  constructible: one binary holding many sites was asked to report the recorder symbol absent and
  present at once (B5). What is deleted is running a census over a binary with more than one site,
  **not the census as an instrument** — G14(a) is one, and the single-site fixture is what makes it
  answer per-site.
- **rev 4's replacement for that clause, the token-level two-profile expansion test** — deleted for
  the opposite reason and recorded because it was written as the above's repair: `zone!`'s expansion
  is a function of its token input alone, so no expansion can drop the identifier for a `Deep` site
  and keep it for an `Always` site in the *same* build, which is what G14(a) and G14(b) jointly
  demanded. Replaced above. The `{ let _ = &$IDENT; }` RED it carried is not lost — it belongs to
  the FEATURE axis and is already stated at G1(a), where `#[cfg]` on the macro definition makes it
  sound.

**Every regression gate carries a `config_tag` clause (S10).** A baseline file is stamped
`config_tag = {profiler: bool, logger: bool}`; a sitting whose tag differs from its baseline's
returns `NotResolved { ConfigMismatch }` through the existing `FloorWorkloadMismatch` path and
**records `UNPROVEN` rather than failing the rung**, until the joint baseline rung (16) re-takes
every baseline in the both-present configuration. RED: hand a gate a baseline with a foreign tag ⇒
`NotResolved`, rung not failed; remove the tag check ⇒ an armed-with-logger sitting is compared
against a logger-absent baseline and a false regression appears ⇒ red. This is why the +25 %
`zone_cost` gate — **and GJ1** — cannot fail a rung before rung 16.

**No gate proves the profiler is honest about its own perturbation.** `instrument_estimated` is an
estimate; only `__fold`, `__reduce`, `__hist_fold`, `__telemetry_reduce`, `__telemetry_write` and
`__cpu_null` are measured directly. That sentence is written in the artifact, next to the number.

### Unit tests (assigned to rungs — F27)

**Rung 1:** SPSC ring empty/full/wrap **per region** · `u32` cursor driven across `u32::MAX` ·
**`Sample` is 24 B / align 8 and every kind round-trips `stamp`+`value` unchanged (B1)** · **a
`Span` of `u64::MAX/2` ticks survives with no saturation and no second record** · `ZoneLane` = 256 B
with four distinct lines (`offset_of!` const-asserts) · concurrent first-execution mints **one
dense id** across 16 threads with **no leaked counter value** · registry exhaustion is
**non-terminal** and `zones_refused` increments · the 90 % warning fires exactly once · `ZoneGuard`
is `!Send` (compile-fail) · `zone!` feature-off accepts an undeclared identifier (G1a) and
feature-on rejects it (G1b) · **an `Engine` mint from a `profiling_partition!(User)` crate is
impossible by construction and a crate with no partition line does not compile (B3, both
compile-fail)**.

**Rung 2:** `SystemMeta` = 256 B in **both** tiers (const-assert + the test at
`system_meta.rs:421`, verified to be `fn system_meta_size_is_256_bytes()`) · frame attribution — a
sample straddling a boundary lands in the frame containing its `stamp` · **a `Counter` whose value
is 10³ and one whose value is 10¹⁸ both land in the CURRENT frame, not in `late` (B1's direct
red)** · **a nested span pair written out of stamp order is attributed to the right frames by the
bidirectional walk** · **one zone receiving 100 000 samples in one frame keeps `count` exact and
`total` consistent (M9's boundary)** · a sample older than the window increments `late` · sealing
with `GpuPass` disarmed · `WINDOW % 2 == 1` · `zone_stride` arithmetic at
`user_zone_budget ∈ {0, 1, 256, MAX}` and the `W9211` threshold · `arm` twice with a different
geometry ⇒ `E9213` · a region's refusals are folded **exactly once** and a second fold with no new
refusals adds nothing.

**Rung 3:** tier folding — a `Deep` zone's `ZoneId` is never minted at `GLOBAL_TIER = Always` ·
`FrameRecord.fixed_steps` equals the substep count for a 0-, 1- and 3-substep frame · `__frame`
excludes `__fold`.

**Rung 4-6:** the 2×2 label truth table, all four rows · `VK_NOT_READY` maps to `Ok` with clear
availability (Mock) · a slot retires on the epoch deadline with an unwritten pair · **a slot
retires on the FRAME deadline with `render_epoch` frozen, and `grace` never underflows from 0
(M13)** · `flush_gpu` at teardown labels every in-flight pair and bumps `gpu_slots_abandoned` ·
`CommandWitness::zone_open_order` records the recording order, not the timestamp order ·
**`stamp_positions` is identical across two recordings of the same pass list and differs when one
bracket moves by one command (M12)**.

**Rung 8:** `resolve` is `NotResolved` at exact equality with the band · every `NotResolvedReason`
round-trips into the artifact · ABBA leg order is `A B B A` · leg summaries survive a window wrap ·
`Floor` cannot be constructed from anything but a session file (compile-fail) · a `Floor` with a
foreign `WorkloadTag` ⇒ `FloorWorkloadMismatch` · **`Floor::from_session_file` reduces three
repetition floors by `max`, publishes all three in `rel_all`, and never averages (M11)** · **a
baseline with a foreign `config_tag` ⇒ `NotResolved`, not a failed rung (S10)** · `counter(id)`
returns `None` for a `Span` zone (no panic) · `measured_quantum_ns` excludes means and returns
`Unknown` on an all-zero sitting.

**Rungs 10-13:** `register_zone` refuses `scope < 32` with `W9212` · refuses past budget **without
leaking an id** · a truncated name sets the flag · 16 threads registering concurrently produce 16
distinct ids, name ranges and `REGISTRY` entries · `DynZoneHandle` is `Send + Sync + Copy`,
`size_of == 16` · the scope projection sets exactly one bit and clears exactly one ·
`scope_by_name` returns `None` for an unregistered name and never allocates · `hist_fold` bucket
index at the clamp boundaries · `HistSlot` saturation increments `hist_saturations` exactly once
per saturating add · lifetime `min` on an empty zone stays `u32::MAX` and reports "no samples",
never a value · the stream header round-trips · **a block's `crc32`/`len` reject a one-byte
corruption and a truncated tail, and the decoder returns no record from a torn block (M8)** · a
`ZoneRow` is emitted exactly once per zone per file, including after rotation · **the 65th quantile
subscription is refused with `W9218` (M7)** · a clock epoch break discards the window and bumps
`clock_epoch`.

**Rung 16 (J2):** GJ1's three legs run and produce three pairwise verdicts; a run whose control leg
(C) does not resolve apart from (B) records `NOT RESOLVED (control inert)` and does **not** fail
the rung.

### Property tests

For any interleaving of `n` pushes and `m` folds: `pushed == folded + in_ring + overflowed`, **per
region independently** · median/p95 match a sorted oracle over random windows · the overlap matrix
is symmetric and reflexive · a `BottomOfPipe`-only partition group's per-frame member sum equals
its run bracket exactly (the sum is formed per frame, S5) · **frame attribution is a total function
OVER ALL THREE KINDS — for any interleaving of spans, counters and gauges with arbitrary payload
magnitudes, every folded sample lands in exactly one frame or one drop counter, and no counter
value influences which (B1's formal statement)** · **`engine_overflow > 0` implies the engine
region alone exceeded its capacity** — user traffic never contributes (the formal statement of
G20) · for any multiset of durations the histogram's `count` equals the number folded and every
reported quantile's edges bracket the oracle · for any sequence of scope toggles, `ARM_MASK` equals
the bitwise OR of the enabled scopes' bits and nothing else · for any `user_zone_budget` and any
mix of static-user and dynamic registrations, every returned `ZoneId` is in
`[ENGINE_ZONE_SLOTS, ENGINE_ZONE_SLOTS + budget)` and all are distinct · **decoding then
re-encoding a stream is byte-identical up to `truncated_tail_bytes`** (M8 — the unqualified form
fails on any real disk-full file).

### Loom / Miri

**Loom** (debug only — release loom binaries crash at startup on this box): one lane, **both
regions**, 1P/1C each, capacity 2, 4 ops — no lost sample, no double-fold, no read of an
unpublished slot · **the `arm` publication order** (`buf` `Release` before `ARM_MASK` `Release`,
emitter `Acquire`-loads the mask): the emitter must never observe a set mask with a null `buf`
(F11) · `register_zone` racing an `Acquire` read of `REGISTRY[id]` · the `seal`/`marks` publish ·
the scope projection's store racing an emitter's load (asserting the emitter sees one of the two
values, never a torn one).

**Miri under Tree Borrows:** `unsafe impl Sync for ZoneLane` · **`unsafe impl Send + Sync for
Profiler`** and every column accessor that reconstitutes a slice from `base` — the `&'static mut`
shape rev 3 used is exactly what Tree Borrows flags, and neither the impl nor the aliasing was on
rev 3's Miri list (B4) · **the `mem::forget` of the reservation at first arm** (no leak-check
failure is expected; the leak is deliberate and the test asserts the base stays readable
afterwards) · the raw sample write through the published pointer · `FrameSlot.marks` `UnsafeCell`
access · `SyncCells` writes into `DYN_DESCS`/`DYN_NAMES` plus the `&'static str` construction from a
reserved byte range.

### Benchmarks (criterion, `harness = false`)

`zone_cost` — **eight legs, one sitting**: static-on / static-off-mask / static-off-tier / dyn-on /
dyn-off-mask / script-FFI, **× {logger absent, logger booted}** for the static-on and
static-off-mask pair (S10). The static-on leg gates at +25 % over the committed baseline **whose
`config_tag` matches**; the dyn legs gate against their own baselines **and** against the static leg
measured in the same sitting (a machine-wide regression must not be attributed to the dyn path).
The logger legs answer "what does a zone cost in the configuration a title actually ships", which
no isolated sitting can.

`fold_cost` — four legs: 400 samples at `zone_stride` 1024 / 1280 / 4096, and 400 samples with 64
histogram slots active. At the 24 B record the 1024 leg is the one that must stay under the L1d
cliff (30.6 KiB of 32 KiB — D8), so this bench measures the cliff rather than assuming it.

`scope_scan` — the fold's step-0 projection at 1 / 16 / 64 registered scopes (D20's ≤ 5 ns ×
`scope_count` claim).

`window_reduce` (1024 zones × 121) · `overlap_pairs` · `gpu_zone_retire` · `stream_encode` (400
`WindowRec`s + the `write_all`, p95 reported) · **`telemetry_window` — `__telemetry_reduce` at 8 /
64 quantile zones, `__telemetry_write`, and their SUM, all three p95-reported (M7: rev 3 benched
only the encode, which is the smaller term)**.

**GJ1 is not a criterion bench and must not become one.** It is a three-leg contrast resolved by
`resolve`, run once, in the J2 sitting, on `bench_bevy_vs_boyko`'s headless schedule — a criterion
harness would report a delta with no band and no control leg, which is the shape S13 exists to
refuse.

**Every baseline file carries `config_tag = {profiler, logger}`** and a sitting whose tag differs
records `UNPROVEN` instead of failing (S10). The committed baselines are re-taken exactly once, in
one sitting, at rung 16.

Protocol per `docs/BENCHMARKING.md`: median-of-N with an **odd** N (S4), High priority, all-core
affinity, **never two bench jobs concurrently** (hard project rule; `target/` once reached 74 GB and
took the disk to zero, masquerading as mingw errors).

**Naming:** no binary may contain `time` / `update` / `setup` / `install` / `patch` (Windows
os-error-740). Hence `zone_cost`, `fold_cost`, `scope_scan`, `gpu_zone_retire`, `contrast_floor`,
`stream_encode`, `telemetry_window`.

### `debug_assert!` invariants

`lane < LANE_COUNT` · `zone < zone_stride` · `!buf.is_null()` at A1 step 9 · `OPEN_DEPTH == 0` at
fold · `write - read <= REGION_CAPACITY` per region · `observed <= REGION_CAPACITY` at the overflow
clear · `used_pairs <= MAX_GPU_PAIRS` · `pool.count == 2 * MAX_GPU_PAIRS` at reset (the width guard
`gpu_timing.rs:492` already carries — verified: a `debug_assert_eq!` on `pool.count`) ·
**`slot.grace > 0` before every decrement (M13a)** · `!is_in_system_run()` at **arm** (same-thread
assertion only; **not** at a scope toggle, which is system-callable by design) · kind matches the
accessor · `frames[i].state != Pending` before the window reducer reads · `WINDOW % 2 == 1` ·
`spec.scope >= 32` in `register_zone` · `region == User` for every dynamic emission ·
**`storage_kind(ProfilingScopeEnabled) == Bitset` at scope registration (B2 — the read path has no
assert of its own, `enable_tag_api.rs:201-215`)**.

**Release-live** (the GPU path inherits the driver's release profile,
`crates/boyko_app/src/gpu_scene/mod.rs:7498` — verified: *"A release bench run (the timing worker
inherits the driver's profile) does not execute it"*): the label computation, **both** retire horns,
**every one of the 18 drop counters**, the witness clauses, the `NOT RESOLVED` verdict, the
user-budget refusal, the clock-epoch-break detector, the telemetry error path, the block checksum,
and the histogram saturation counter. **A reporting obligation that vanishes in release is the
vacuous-gate pattern by another route.**
