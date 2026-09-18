# Skip-route inventory for the device leg: joltab @ a1849541, plus the uncommitted vkval lane

This is read-only; no test was run. Path legend (all absolute):
- **A/** = `D:/wt/joltab/crates/boyko_app/tests/`
- **R/** = `D:/wt/joltab/crates/boyko_render/tests/`
- **V/** = `D:/wt/joltab/crates/boyko_rhi_vulkan/tests/`
- **K/** = `D:/wt/vkval/crates/boyko_app/tests/`

Line numbers are for these snapshots. joltab moved from 9de111ef to a1849541 during this session; none of the files read here changed. **Someone was editing the vkval files during this session** (mtimes 16:01 and 16:03), so the K/ line numbers may drift.

## 0. Where the source contradicts the report

| Report claim | What the source says |
|---|---|
| 2 census workers panic with `.expect` (`vg_r0d_census.rs:152`, `vg_density_census.rs:189`) | **Already fixed on joltab** by e3cebe2d. Both now have skip-by-name guards: `A/vg_density_census.rs:202-208` and `A/vg_r0d_census.rs:165-178`. The `.expect`s (:212, :182) can no longer be reached when a variable is absent. vkval's base is e3cebe2d. |
| "29 boyko_app windowed targets have no skip route — red is honest" | **False for 24 tests.** On boot failure `run_windowed` returns `AppExit(true)` (`D:/wt/joltab/crates/boyko_app/src/runner.rs:233-296`; the non-Windows arm is `:1012-1014`). These 24 tests end in `app.run();` and assert nothing afterwards, so they report ok. The only line printed is the runner's diagnostic: `boyko-E3002: host boot failed at the {stage} stage ({e}) - exiting` (`src/diag.rs:142`, and only when the log has no consumer). See table 1-C. |
| The same event is spelled four ways | **At least nine spellings**, and **several print nothing at all**. See table 1-F. |
| Proposed classes {device, cap, validation-off, worker-unspawned, payload-absent} | Routes that exist in the tree fall outside this set: validation layer not installed (it reaches the test as the `device` message, carrying `ValidationUnavailable`), runtime surface loss, tool absent, sibling binary absent, platform (E3004), silent runner fall-through, and targets that compile to zero tests. See table 1-F. |
| The census "has since added miri-unsupported" | **The census enforces no vocabulary at all.** `miri-unsupported` exists only in CLAUDE.md prose (`D:/wt/joltab/CLAUDE.md:287`) and at 6 sites. |

## 1. Skip routes in the device leg

The device leg is 141 tests: boyko_app 87, boyko_render 6, boyko_rhi_vulkan 48. That is the 145 plain sites in these three crates, minus `app12` (`slow:`) and the 3 generators. vkval adds 10 more.

### 1-A. Shared boot helpers: each is one route used by many tests

| id | Helper file:line | Condition | Exact string printed today | Class | Result after |
|---|---|---|---|---|---|
| H1 | R/common/mod.rs:30-41 `boot_or_skip` | `VulkanContext::boot{validation:true}` returns Err. This covers loader, GPU **and** `ValidationUnavailable` (layer not installed). | `SKIP {test}: validation layer / GPU unavailable ({e:?})` (:37) | device (or validation-unavailable) | return → ok |
| H2 | R/common/mod.rs:49-57 `assert_validation_clean` | `!ctx.validation_enabled()` | `NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) - messenger oracle skipped` (:55) | validation-off (the oracle is dropped; the test keeps running) | ok |
| H3 | V/window_present_gbuffer.rs:1618-1707 `with_windowed_present` | 7 routes: window open :1619-1625, windowed boot :1626-1632, surface :1648-1654, swapchain :1655-1661, extent < composite :1670-1679, no decodable UNORM :1681-1684, no Format variant :1685-1692 | `SKIP {label}: cannot open a window ({e:?})` / `…: windowed Vulkan unavailable ({e:?})` / `…: surface creation failed ({e:?})` / `…: swapchain creation failed ({e:?})` / `…: swapchain extent {}x{} is smaller than the {COMPOSITE_W}x{COMPOSITE_H} composite` / `…: swapchain format has no host-decodable UNORM byte order` / `…: swapchain format has no basic-slice Format variant` | device ×2, cap (WSI/format) ×5 | return → ok |
| H3n | same, :1637-1639 | validation off | `NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel gate still runs` | validation-off (degrade) | ok |
| H4 | V/sdf_gbuffer_hybrid.rs:454-466 `boot_or_skip` | boot Err (validation requested) | `SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})` (:461) | device | ok |
| H5 | V/sdf_gbuffer_hybrid.rs:473-492 `assert_validation_clean` | validation off | `NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — skipping the clean-oracle assert` (:475). **The second branch :478-485 is unreachable**, so its "escape hatch must be set" assert never runs. | validation-off (degrade) | ok |
| H6 | V/sdf_gbuffer_hybrid.rs:3065-3079 `boot_render_or_skip` | H4, plus a NOTE | H4 string; `[{test}] NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — pixel goldens still run` (:3073) | device / validation-off | ok |
| H7 | A/teardown_probe/mod.rs:234-253 `assert_preconditions_and_controls` | `budget_left == BUDGET` (:241); VB boot built no MeshGeometryTable (:248) | `SKIP [teardown-probe {label}]: windowed boot unavailable — the frame loop never ran, nothing was measured (a skip, not a pass)\n{s}` / `SKIP [teardown-probe {label}]: the VisibilityBuffer boot built no MeshGeometryTable on this device (resolve degraded) — nothing was measured (a skip, not a pass)\n{s}` | device / cap | returns false → the caller returns → ok |
| H8 | A/hzb_build_oracle_gate.rs:418-428 | boot Err (validation **off**) | `SKIP hzb_build_oracle_gate: GPU / loader unavailable ({e:?})` (the label is the file, not the test) | device | ok |
| H9 | A/hzb_verdict_oracle_gate.rs:570-580 | same | `SKIP hzb_verdict_oracle_gate::{what}: GPU / loader unavailable ({e:?})`, where `what` ∈ pinned_extents / random_corpus / boundary / sentinel, which are not the fn names | device | ok |
| H10 | FrameBudget idiom, inline in each test (rows in 1-B) | `remaining == BUDGET` after `app.run()` | `SKIP <test>: windowed boot unavailable` | device. It cannot tell loader, GPU, window-host, bindless-table and ValidationUnavailable apart. | ok |
| RUN | D:/wt/joltab/crates/boyko_app/src/runner.rs:233-296, :1012-1014 | any boot stage fails, or the platform is not Windows | `boyko-E3002: host boot failed at the {stage} stage ({e}) - exiting` / `boyko-E3004: windowing is not implemented for this platform - exiting` (src/diag.rs:142, :188; eprintln only when there is no log consumer) | device / platform | `AppExit(true)` → the test decides |

### 1-B. Every device-leg test: its routes and class

In the table: "ok" = libtest reports ok after the route. "red" = no silent route: a missing device panics or asserts.

**boyko_app (87 tests)**

| Test (fn line) | Skip routes | Class | After |
|---|---|---|---|
| A/asset_streaming_f6_churn_headless.rs `mesh_churn_…_against_a_live_device` (300) | :318-325 H10 (string split across 2 lines). The reason says "validation layers ON", but no check in the body. Validation arms only with `BOYKO_ENABLE_VALIDATION` (module doc :78-89). | device; validation-off **silent** | ok |
| A/asset_streaming_f7_grow_headless.rs `f7_grow_and_defer_old_phased_headless` (431) | :461-463 H10. Same silent validation. | device; validation-off silent | ok |
| A/asset_streaming_f7_rt_cap_headless.rs `rt_leg_grows_past_instance_capacity_instead_of_panicking` (170) | :184-190 H10. The file is `#![cfg(windows)] #![cfg(feature="hwrt")]` (:67-68). | device; feature-absent → 0 tests | ok |
| A/asset_upload_reject_mesh_leak.rs (268) | :281-283 H10 | device | ok |
| A/asset_upload_reject_texture_leak.rs (181) | :194-196 H10 | device | ok |
| A/interp_smoke.rs (111) | :124-126 H10 | device | ok |
| A/room_smoke.rs (150) | :166-168 H10 | device | ok |
| A/room_smoke_catch_all_fit.rs (112) | :132-134 H10 | device | ok |
| A/sdf_room_smoke.rs (100) | :120-122 H10 | device | ok |
| A/texture_retire_lifecycle.rs (263) | :281-286 H10. Reason says "validation ON"; silent as in f6. | device; validation-off silent | ok |
| A/vb_geometry_slot_fill_reject.rs (214) | :229-231 H10; :237-242 `SKIP a_fill_rejected_mesh_upload_releases_its_vb_geometry_table_slot: the VisibilityBuffer boot built no MeshGeometryTable on this device (resolve degraded) — nothing was measured` | device, cap | ok |
| A/vb_geometry_slot_retire_churn.rs (200) | :216-218 H10; :224-229 (same cap text, its own fn name) | device, cap | ok |
| A/windowed_smoke.rs (38) | :51-53 H10 | device | ok |
| A/forward_teardown_destroys_forward_sets.rs (38) | :46-47 → H7 (label "boyko_app Forward teardown probe") | device | ok |
| A/vb_teardown_destroys_boot_resources.rs (41) | :49-50 → H7 (both routes) | device, cap | ok |
| A/hzb_build_oracle_gate.rs ×3 (1058, 1115, 1223) | H8 | device | ok |
| A/hzb_verdict_oracle_gate.rs ×4 (1510, 1591, 1722, 2082) | H9 | device | ok |
| **24 fall-through tests** (list in 1-C) | RUN only; no discrimination | device (unmarked) | **ok** |
| A/particle_lab.rs (97) | none. A bare `--ignored` panics at :100-104 (asserts BOYKO_HOST_DUMP is set); read_bmp panics (particle_scene/mod.rs:975). | — | red |
| A/particle_counters_readback.rs (72) | none (panics at :101 if the resource is missing) | — | red |
| A/vb_cull_offscreen.rs (192) | none (panics reading the file :207) | — | red |
| A/vb_inst_cull_ids.rs (83), vb_inst_cull_narrow.rs (85), vb_inst_cull_wide.rs (47) | none (asserts on the probe) | — | red |
| A/sv0_adequacy.rs (644) | none. Bare run panics at :650 (asserts BOYKO_WINDOW_FRAMES ≥ 2). After run: `!is_dirty()`, see §5. | — | probably red |
| A/vg_decidability_floor.rs (326) | :327-334 `SKIP vg_decidability_floor_measure: the sibling \`vb_p1d_cull_shade_bench\` binary is not built. Run \`cargo test -p boyko-app --test vb_p1d_cull_shade_bench --no-run\` first. NOTHING about the decidability floor is measured by this run.` Failed sessions are dropped (:261-310); with 0 artifacts the `.expect` panics at :425. | artifact-absent | ok |
| 16 workers | table 2 | worker-unspawned (+ payload) | ok |
| A/vb_sv0_produce_run_timing.rs drivers ×2 (548, 597) | `let Some(art)=run_worker(..) else { return }` :549/:598. run_worker :508-512 prints `{WORKER}: this device serves no timestamps -- SKIPPED` when the child output contains `no usable timestamps`. | cap (timestamps) | ok |
| A/vg_occ_split_timing.rs `_mixed` (1153), `_dense` (1160) | :1200-1208 `VG R3 P4-6: INSTRUMENT-DEAD -- this device reports unusable timestamps, so BOYKO_VB_ZONE arms no recorder and channel G does not exist here. Channel W is KNOWN-BLIND by itself, so this sitting produces no number. Re-run on a timestamp-capable device.` (bench_summary :1122-1128 returns None on `device timestamps are unusable`) | cap (timestamps) | ok |
| A/vg_occ_split_timing.rs `vb_occ_dense_defers_…` (2245) | none | — | red |
| A/vb_bench_query_validation.rs driver (541) | :573-586, when **both** workers fail: `vb_bench_query_validation: INSTRUMENT-DEAD -- {why}. This is not a finding about VG R3 piece 4; …` (why = timestamps, or the validation layer crashed) | cap / validation-crash | ok |
| A/vb_inst_cull_corpus.rs gate (288) | :289-296 `SKIP vb_inst_cull_corpus_gate: the gitignored corpus payload is absent (run scripts/fetch_corpus.ps1). NOTHING about per-instance culling on the corpus is observed by this run.` | payload-absent | ok |
| A/vg_r0d_census.rs gate (280) | :281-287 `SKIP vg_r0d_census_gate: the gitignored corpus payload is absent (run scripts/fetch_corpus.ps1). NOTHING about K1 is adjudicated by this run.` | payload-absent | ok |
| hzb drivers ×3 (1157, 1215, 1256); vb_occ_split_gate (605); vb_occ_mixed ×2 (851, 880); vb_cull_hzb_pairing (150); vg_density_census_gate (504); vb_mesh_occ_pins_actually_split (539) | none. `status.success()` plus a missing artifact → panic. | — | red |

**boyko_render (6 tests)**

| Test (fn line) | Routes | Class | After |
|---|---|---|---|
| R/bindless_smoke.rs (32) | H1 :33-34; H2 via :63 | device; validation-off (degrade) | ok |
| R/texture_upload_smoke.rs (28) | H1 :29-32; H2 via :65 | same | ok |
| R/orbit_camera_drives_render_gpu.rs `s35_orbit_screenshot` (603) | H1 :604-605; :610-617 `SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)` (no test name) | device; validation-off (**full skip**) | ok |
| R/p7b_world_ui_screenshot.rs (1084) | H1; :1091-1098 same string | same | ok |
| R/ui_hud_screenshot.rs `p6b_hud_screenshot` (1077), `_msdf` (1153) | H1; :1084-1091 / :1160-1167 same string | same | ok |

**boyko_rhi_vulkan (48 tests)**

| Test (fn line) | Routes | Class | After |
|---|---|---|---|
| V/compute.rs `negative_chained_barrier_hazard` (367) | local boot :74-87, `SKIP {test}: validation layer / GPU unavailable ({e:?})` | device | ok |
| V/ddgi_probe_gi_arm.rs (149) | boot :56-66 (validation off) `SKIP ddgi_probe_gi_arm: GPU / loader unavailable ({e:?})`; :156-159 `SKIP ddgi_probe_gi_arm: device lacks B10G11R11/RG16F STORAGE` | device, cap | ok |
| V/ddgi_probe_gi_cost.rs (574) | boot :127-139 `SKIP ddgi_probe_gi_cost: GPU / loader unavailable ({e:?})`; run_sweep :253-259 `SKIP ddgi_probe_gi_cost: device lacks B10G11R11/RG16F STORAGE (irr_ok={}, depth_ok={})` | device, cap | ok |
| V/ddgi_probe_gi_resolve.rs (240) | boot :70-80 `SKIP ddgi_probe_gi_resolve: GPU / loader unavailable ({e:?})` | device | ok |
| V/hwrt_blas_smoke.rs ×3 (117, 210, 474) | boot :47-57 `SKIP {test}: GPU / loader / validation unavailable ({e:?})`; :121-127 / :214-220 / :478-484 `SKIP {fn}: device '{}' does not expose ray query (non-RT GPU)`; NOTE :60-63 `NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — messenger oracle skipped`. The file is `#![cfg(feature="hwrt")]` (:21). | device, cap, validation-off, feature-absent | ok |
| V/m2_brick_atlas_smoke.rs (55) | boot :17-28 `SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})`; **:62-70 `SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)` returns before `BrickAtlas::create`**; the NOTE at :111-120 is unreachable | device; validation-off (full skip) | ok |
| V/m3_dirty_atlas.rs (499) | inline :504-512 `SKIP m3_incremental_atlas_renders_identically_to_full_on_device: no GPU ({e:?})`; validation :547-554 `if let Some(state)=ctx.debug_state()`, **prints nothing** | device; validation-off **silent** | ok |
| V/m5_scroll_atlas.rs (450) | inline :455-463 `SKIP m5a_scroll_update_renders_identically_to_rebake_all_on_device: no GPU ({e:?})`; :505-512 silent, as in m3 | device; validation-off silent | ok |
| V/particle_sim_occupancy.rs (516) | `VulkanProbe::open` :609-611 `SKIP particle_sim_occupancy: vulkan-1.dll did not load`; :616-618 `…: vkGetInstanceProcAddr not exported`; :652-654 `…: vkCreateInstance failed ({r})`; :738-745 `…: no physical device exposes VK_KHR_pipeline_executable_properties`; :781-785 `…: vkCreateDevice failed ({r})` | device ×3, cap ×2 | ok |
| V/sdf_gbuffer_hybrid.rs `a5_gpu_off_vs_on_wall_clock_ab` (4028) | H6 | device | ok |
| V/sdf_gbuffer_hybrid.rs ×7 (6668, 6785, 7261, 7328, 7402, 7450, 7538) | H4 + H5 | device; validation-off (degrade) | ok |
| V/software_ray_baseline_cost.rs (213) | boot :118-129 `SKIP software_ray_baseline_cost: GPU / loader unavailable ({e:?})`; :221-229 **println** `SKIP software_ray_baseline_cost: GPU timestamps unusable (valid_bits={}, period={} ns/tick)`; :231-235 println `SKIP software_ray_baseline_cost: device lacks B10G11R11/RG16F STORAGE for the DDGI atlas` | device, cap ×2 | ok |
| V/spec_constant_smoke.rs (159) | boot :48-58 `SKIP {test}: validation layer / GPU unavailable ({e:?})`; NOTE :63-66 (—). The file is `#![cfg(feature="spec_constant_smoke")]` (:28). | device, validation-off, feature-absent | ok |
| V/window_present_gbuffer.rs ×27 (2999, 4401…7743) | H3 (label `engine_showcase_512` for 26 of 27 tests, `p0_windowed_coarse_cull` for p0) + H3n + runtime routes in 1-D. Post-run messenger check `if ctx.validation_enabled()` :10334 / :8910 is **silent** when off. | device, cap, validation-off, runtime | ok |

**vkval additions (10 tests)**

| Test | Routes | Class | After |
|---|---|---|---|
| K/boot_validation_clean.rs `every_validated_boot_is_clean` (763) | none. Worker exit 0 = `Outcome::NoVerdict` = red (:641); a failed boot trips the assert at :494-501 (`BOOT DID NOT REACH THE FRAME LOOP`). | — | red |
| K/boot_validation_clean.rs 6 workers | table 2 | worker-unspawned | ok |
| K/unwritten_shadow_map_gate.rs 2 drivers (610, 696) | none. Exit 0 → "reached no verdict", red (:518); a missing artifact asserts at :316-324. | — | red |
| K/unwritten_shadow_map_gate.rs worker (271) | table 2 | worker-unspawned | ok |

### 1-C. The 24 silent fall-through tests (only route: RUN; nothing after `app.run()`)

A/csm_fit_eval.rs:107 · A/forward_both.rs:151 · A/forward_mesh.rs:160 · A/forwardplus_mesh.rs:170 · A/grand_showcase_2mat.rs:174 · A/grand_showcase_mvpm.rs:164 · A/pbr_material_showcase.rs:301 · A/pbr_showcase.rs:181 · A/sdf_forward_only.rs:95 · A/textured_smoke.rs:217 · A/vb_both.rs:152, :174 · A/vb_both_sdf.rs:84 · A/vb_both_sdf_tex.rs:133 · A/vb_both_ssao.rs:104 · A/vb_mesh.rs:272 · A/vb_mesh_froxel.rs:309 · A/vb_mesh_ssao.rs:181 · A/vb_mesh_tex.rs:246, :285 · A/vb_mesh_tex_froxel.rs:328 · A/vb_sdf_only.rs:92 · A/vb_p1d_cull_shade_bench.rs:585 · A/taa_jitter_eval.rs:438

When a GPU is present and neither `BOYKO_HOST_DUMP` nor `BOYKO_WINDOW_FRAMES` is set, these hang instead (runner.rs:1128-1137). `scripts/golden.ps1` is the only thing that makes a skip red for them: its fresh-BMP guard at :216-223.

### 1-D. Runtime routes (reached after a successful boot)

All are in V/window_present_gbuffer.rs:

| Where | String | After |
|---|---|---|
| p0 :3769-3771, :3792-3795, fall-through arm :3888-3892 | `NOTE p0 cull: extent changed before the readback frame — skipping` / `NOTE p0 cull: swapchain recreated on the readback frame — skipping` / `NOTE p0_windowed_coarse_cull: a readback frame did not present (swapchain kept …` | ok |
| DDGI converge :8830-8871 | `NOTE engine_showcase_512: window closed / extent changed / swapchain recreated during DDGI convergence — skipping` | ok |
| showcase dump :10270-10300, :10369-10374 | `NOTE engine_showcase_512: window closed before the dump frame — skipping` (+ extent / recreated variants); `NOTE engine_showcase_512: no readback frame presented (swapchain kept recreating); no BMP written` | ok |
| shadow_dolly :6567-6575, shadow_lag :6690-6751, shadow_ab :6818-6855 | `SKIP shadow-dolly: window closed / swapchain recreated`, `SKIP shadow-dolly: capture failed`, `SKIP shadow-lag: … / static capture failed`, `SKIP shadow-ab: window closed during {warm-up / the yaw sweep / the strafe sweep / the micro-yaw warm-up}` | ok |
| interp-smoke :6988-6999 | `SKIP interp-smoke: window closed / swapchain recreated during {warm-up / alpha=0 capture / alpha=0.5 capture}` | ok |

### 1-E. Default-leg (not ignored) device-booting tests: same helpers, same silent pass on CI

Counting only tests with a direct boot call in their body gives at least 108 tests across 44 files. Tests that boot through a helper are not counted.

| Crate | Files (count) | Message shapes |
|---|---|---|
| boyko_render | camera_drives_render_gpu 1 (validation-off full skip :350), drop_teardown 2, gpu_system 1, grow_stale 1, lifecycle 1, multi_column 1, round_trip 1, sync_validation 2, ui_rect_gpu_golden 1 (:336), ui_rect_swapchain_golden 1 (7 routes :294-374 + NOTE :678), ui_text_gpu_golden 1 (:420), ui_text_multiscale 1 (:302), zero_readback 1 | H1 / H2 / `SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)` |
| boyko_rhi_vulkan | sdf_gbuffer_hybrid 34, roundtrip 5, sdf_perspective_resolution 5, vg_block_pool_growth 5, compute 4, texture_view_capability 4, cluster_cull_hier_equiv 3 (+13 via helper), csm_inc0 3, gpu_zone_deadlines 3, sdf_editlist 3, calibrated_timestamp_probe 2 (cap `VK_EXT_calibrated_timestamps absent` :81), graphics_depth 2, sdf_vocabulary_storage_image 2, 1 each: device_local_copy, g7_validation_reporting, gpu_command_census (timestamps :153), gpu_query_availability_truth (:78), gpu_zone_label_control (:87), graphics_deferred/offscreen/sample/triangle/triangle_mvp, sdf_mesh_hybrid_depth, sdf_spheretrace, ssbo_graphics_probe, vg_extent_probe, window_present, window_present_gbuffer (the non-ignored one), window_present_hybrid, window_present_scene | local copies of H1/H2/H3; `SKIP {name}: GPU timestamps unusable` |
| Device-free, default leg: tool / payload | ~13 `*_spv_sync`/`*_edsl_sync`/`*_gate` targets, `particle_edsl_sync` ×13 sites, `vb_geo_preprocess_sync` (git :327); payload: A/vb_inst_cull_corpus.rs:462, A/vg_r0d_census.rs:232, A/vg_corpus_ingest.rs:308/:329, A/vg_cull_granularity_census.rs:399, R/vg_glb_decode.rs:300 | `…: dxc not found — SKIPPING …`, `…spirv-dis not found — SKIPPING …`, `SKIP …: no spirv-dis on this host`, `SKIP …: the gitignored corpus payload is absent …` |

### 1-F. Classes the proposed vocabulary does not cover

| Route | Where | Prints | Can a marker fire? |
|---|---|---|---|
| validation layer not installed | every `boot{validation:true}` helper, H3 | the device string with `ValidationUnavailable` in `{e:?}` | yes, if split out |
| runtime surface loss | 1-D | `NOTE…`/`SKIP…` | yes |
| tool absent (dxc / spirv-dis / git) | 1-E | `…SKIPPING…` | yes |
| sibling artifact absent | vg_decidability_floor :327 | `SKIP …` | yes |
| platform | runner.rs:1012 | `boyko-E3004…` | only inside the runner |
| silent fall-through | 1-C (24 tests) | only E3002 | only if the runner or the test emits it |
| silent validation degrade | m3/m5, window_present_gbuffer :10334/:8910, app f6/f7/texture_retire | nothing | only if code is added |
| feature-absent / `#![cfg(windows)]` on Linux | hwrt_blas_smoke, spec_constant_smoke, f7_rt_cap, 107 windows files | `running 0 tests` | **no.** The runner must check the `running N` count. |

## 2. Worker tests

| # | Worker (fn line) | Driver(s) | Env var(s) the guard checks | Guard | Later expect/panic on env | Current `#[ignore]` reason |
|---|---|---|---|---|---|---|
| 1 | A/hzb_engine_pyramid_gate.rs `hzb_engine_pyramid_dump` (383) | `…equals_the_oracle` (1157) via run_dump_worker :730-760 | BOYKO_HZB_DUMP (ENV_DUMP :131) | helper `dump_path_or_skip` :354-369, prints `{label}: {ENV_DUMP} is unset -- SKIPPED. …` | none | "needs a real windowed GPU device; the G8 driver spawns it with BOYKO_HZB_DUMP set" |
| 2 | same file `…_dump_occ` (404) | `…_oracle_occ` (1215) | same | same | none | "…; the G5 driver spawns it with BOYKO_HZB_DUMP set" |
| 3 | same file `…_dump_occ_late` (455) | `…_oracle_force_late` (1256) | BOYKO_HZB_DUMP only. `BOYKO_VG_OCC_FORCE=late` is **unguarded**: when absent, occ_fixture/mod.rs:91-92 defaults to `None`, so the run renders the wrong regime silently. | same | none | "…; the G-P3-E driver spawns it with BOYKO_HZB_DUMP and BOYKO_VG_OCC_FORCE=late" |
| 4-7 | A/vb_occ_split_gate.rs `vb_occ_probe_dump_{marked 388, unmarked 411, multi 427, marked_no_hzb 460}` | `vb_occ_split_records_two_scopes` (605) via run_worker :539 | BOYKO_VB_PROBE (:106) | helper `probe_path_or_skip` :340-357 | none | "needs a real windowed GPU device; the G2 driver spawns it with BOYKO_VB_PROBE set" |
| 8-9 | A/vb_bench_query_validation.rs `…_bench_worker` (316), `…_control_worker` (327) | `the_bench_armed_query_commands_add_no_validation_message` (541) via spawn_worker :358 | BOYKO_VB_QUERY_VALIDATION_DRIVEN (:208) + BOYKO_VB_ZONE / BOYKO_WINDOW_FRAMES | helper `skip_unless_driven` :295-305, prints `{worker}: {DRIVER_MARKER} and/or {terminating_knob} unset -- SKIPPED. …` | none | "needs a real windowed GPU device with the validation layer; the driver spawns it" |
| 10 | A/vb_occ_mixed.rs `vb_occ_mixed_capture_worker` (168) | `…_partition_matches_the_oracle` (851), `…_force_late_rasterises` (880) via run_capture :358 | literal BOYKO_VB_CULL_READBACK, BOYKO_HZB_DUMP, BOYKO_VB_PROBE (:171) | inline :171-179 | none | "…; the G-P3-B/C drivers spawn it with all three capture knobs set" |
| 11 | A/vb_cull_hzb_pairing.rs worker (113) | `both_captures_are_produced_by_one_process` (150) | literal BOYKO_VB_CULL_READBACK, BOYKO_HZB_DUMP (:116) | inline :116-124 | none | "…; the pairing driver spawns it with both knobs set" |
| 12 | A/vb_sv0_produce_run_timing.rs `vb_sv0_produce_run_worker` (415) | dp6_0b fused (548), split (597) via run_worker :472 | BOYKO_DP6_FIXTURE (:66) + BOYKO_VB_ZONE | inline let-else :416-428 | none | "spawned by the drivers below; needs a real windowed GPU device" |
| 13 | A/vg_occ_split_timing.rs worker (621) | `_mixed` (1153), `_dense` (1160), `vb_occ_dense_defers…` (2245) via base_worker_cmd :1035 | BOYKO_VG_OCC_TIMING_LEG (:340) + one exit knob | inline :622-646 | `.expect` :648, reachable only after the guard | "…; the drivers spawn it once per (leg, budget)" |
| 14 | A/vb_inst_cull_corpus.rs worker (248) | `vb_inst_cull_corpus_gate` (288) via vb_inst_cull_scene/mod.rs:695-726 | BOYKO_VG_PATH (:82) + payload | inline :253-268 | none | "needs a real windowed GPU device and the fetched corpus payload; the corpus gate spawns it per camera path" |
| 15 | A/vg_density_census.rs `vg_census_rung_dump` (194) | `vg_density_census_gate` (504) via vg_thresholds/mod.rs:288 | BOYKO_VG_FIXTURE, BOYKO_VG_RUNG, BOYKO_VG_CENSUS | `first_absent` :202-208 (e3cebe2d) | `.expect` :212, unreachable on absence | "…; the census driver spawns it once per (fixture, ladder rung)" |
| 16 | A/vg_r0d_census.rs `vg_r0d_rung_dump` (157) | `vg_r0d_census_gate` (280) | BOYKO_VG_PATH, BOYKO_VG_RUNG, BOYKO_VG_CENSUS + payload | `first_absent` :165-171 + payload :172-178 | `.expect` :182, unreachable | "…and the fetched corpus payload; the R0d driver spawns it per (path, rung)" |
| 17 | A/vb_mesh.rs `vb_mesh_screenshot_dump` (272). **Dual role, not in the report's list.** | `vb_mesh_occ_pins_actually_split` (539) via run_pin (:489-507) | BOYKO_HOST_DUMP, BOYKO_VB_PROBE, pin env | **none** (it is also a standalone dump) | none | "…; the orchestrator runs it on the GPU to dump the VisibilityBuffer mesh-only screenshot" |
| 18 | A/vb_p1d_cull_shade_bench.rs (585). **A worker spawned from another binary, not in the report's list.** | vg_decidability_floor_measure (326) via run_session :261, which runs the whole binary with `--ignored` and **no `--exact`** | BOYKO_VB_ZONE, BOYKO_PROFILE_* | **none** | none | "needs a real windowed GPU device; BOYKO_VB_ZONE=1 BOYKO_VB_BENCH_LIGHTS=<n> …" |
| 19-24 | K/boot_validation_clean.rs 6 workers (attrs 581-611) | `every_validated_boot_is_clean` (763) via run_child | BOYKO_BOOT_VALIDATION_DRIVEN (:106) | inside `run_worker` :429-437, prints `SKIP {name}: {DRIVER_MARKER} is unset. …`. Then asserts VK_LOADER_LAYERS_DISABLE; exits via `process::exit(90/91)` :577. | assert (reachable only when driven) | "gpu-windowed: worker spawned by every_validated_boot_is_clean; returns at once unless BOYKO_BOOT_VALIDATION_DRIVEN is set" |
| 25 | K/unwritten_shadow_map_gate.rs `shadow_poison_worker` (271) | `mesh_less_legs_…` (610), `mesh_legs_write_…` (696) via run :454 | BOYKO_SHADOW_GATE_DRIVEN (:126) | inline :272-278 | 6× `required_env` panics :265-267 (reachable only when driven); `process::exit(92)` :329 | "gpu-windowed: worker for unwritten_shadow_map_gate, not a standalone check" |

**Implications for a census check "the worker body has a skip-by-name guard on its driver env var":**
- Only #10 and #11 spell the variable **literally** in the body.
- #12-16 and #25 reference a `const`. #1-9 and #19-24 guard through a **helper** (dump_path_or_skip, probe_path_or_skip, skip_unless_driven, run_worker).
- The worker name the driver passes is a `const WORKER`, a literal, or a `Worker::test_name()` table (Gate A).
- So a line-grep census cannot link a driver to its worker or its env var without resolving consts and helpers.

**Where a worker's marker ends up:**
- Inherited stdio (`.status()`), so the child's output lands on the parent's console: hzb :756, occ_split :555, occ_mixed :389, pairing :172, vg_thresholds :303, vb_mesh :507, vb_inst_cull_scene :726, vg_occ :1075/:2156.
- Captured (`.output()`): sv0 :506, vg_occ :1123, bench :394, floor :289.
- Written to files: K/boot_validation_clean.rs:699-701, K/unwritten_shadow_map_gate.rs:472-474.

## 3. `D:/wt/joltab/tests/ignore_reasons_census.rs` (982 lines)

| Part | Lines | What it does | Gap for a class check |
|---|---|---|---|
| `BARE_IGNORE_WAIVERS` (empty) | 99-102 | escape hatch, one row per site | model for a migration ledger of unprefixed sites |
| `SKIP_DIRS`, `MIN_FILES`=800, `MIN_SITES`=120 | 112, 116, 130 | walk exclusions and non-vacuity floors | — |
| `IgnoreForm` / `IgnoreSpelling` / `Site{file,line,test_fn,form,spelling}` | 139-170 | site model | **there is no reason-text or class field** |
| `form_after_ignore_token` | 183-199 | reads only the first chars after `=`: `""` → EmptyReason, otherwise Reasoned | the reason string is thrown away |
| `after_ignore_token` (whole-token match) | 206-220 | cfg_attr path | — |
| `bracket_depth` | 225-248 | string-aware bracket count (escapable strings only) | — |
| `joined_attribute` (≤12 continuation lines) | 252-280 | joins multi-line attributes; `\`-continued reasons come through as `\ ` pairs | the class is taken from this joined text |
| `after_plain_ignore_attr` / `classify` | 285-323 | skips `//` and `*` lines; requires `#[` | — |
| `resolve_test_fn` (24-line lookahead) | 135, 329-350 | first `fn ` at a word boundary → name | **gives the name only; no body extent** |
| Lexer `Scan` / `string_open_at` / `char_literal_len` / `lines_beginning_in_a_string` | 354-506 | per-char state machine (strings, raw strings, char literals, nested block comments), but it **outputs only a per-line "begins in a string" flag** | a body extractor needs its per-char state exposed: bodies contain `"{label}"` braces |
| `collect_rs` / `sites_in_text` / `census` / `crate_of` | 509-581 | walk → sites; `sites_in_text` is shared with the fixtures | new fields get filled here (:549-555) |
| Tests | 584, **638-731** (`every_ignore_attribute_states_a_reason`), 738, 764, 816, 905, 949 | join fixtures, main clause, stale waivers, classifier table, string fixtures, phantom pins, resolver control | a vocabulary test and a worker-guard test would sit next to :638, and need their own positive-control tables like :764 and :949 |

**Reason prefixes in use**, from my scan of the joltab tree. It gives 312 sites = the census's 310 plus the 2 phantoms the census rejects.

| Spelling | Prefix | Count | In CLAUDE.md vocabulary (:284-289)? |
|---|---|---|---|
| plain | (none) | 147 | — |
| plain | deferred | 17 | yes |
| plain | gpu-windowed | 2 (A/forward_teardown…:37, A/vb_teardown…:40) | yes |
| plain | generator | 1 | yes |
| plain | slow | 1 (A/app12:157) | yes |
| cfg_attr | miri-slow | 85 | yes |
| cfg_attr | instrument | 19 (boyko_threadpool block.rs ×18, block_allocation_receipts.rs:663) | **no** |
| cfg_attr | tractability | 9 | **no** |
| cfg_attr | miri-unsupported | 6 (one reads `miri-unsupported: instrument: …`) | yes (prose only) |
| cfg_attr | slow | 5 | yes |
| cfg_attr | miri-arm | 1 | **no** |
| cfg_attr | (none) | 17 | — |
| vkval delta | +10 `gpu-windowed` (7 of them are workers) | | |

In the three device crates on joltab, **141 of the 145 plain sites carry no prefix**. CLAUDE.md's `gpu`, `gpu-cap`, `feature`, `solo`, `flaky` classes are used at no site.

## 4. What runs the device leg today

| Runner | Exact command / location | Sees skips? | Plug-in point for a marker-failing runner |
|---|---|---|---|
| CLAUDE.md recipe | `D:/wt/joltab/CLAUDE.md:214-224`: prose only, "per-binary with `--test-threads=1`"; env protocols live in module headers; the count is stale (135 vs 141) | **No.** Without `--nocapture`, libtest captures `eprintln!` from passing tests, so every `SKIP` line is swallowed. | the recipe must require `--nocapture` (or emit the marker via `std::io::stderr()` directly), plus a `running N` check |
| Sweep list (not in the repo) | `D:/tmp/runlist.txt` (75 binaries), `D:/tmp/ignore_sites.tsv` | depended on a human reading the output | would become a derived list (census → classes) |
| `D:/wt/joltab/scripts/golden.ps1` | :196 `cargo test -p $crate [--features hwrt] --test $bin $name -- --ignored --test-threads=1`; `-ValidationOn` :197-208 captures merged output via `cmd /c "cargo … --nocapture > log 2>&1"` (PS 5.1 stderr workaround) | indirectly: the missing/stale BMP guard :216-223 throws | scan for the marker next to the `[vk-validation]` scan (:225-236); reuse the `cmd /c` capture idiom |
| `D:/wt/joltab/.github/workflows/ci.yml` | every job is `ubuntu-latest`; `test` :96-117 = `cargo test --workspace --all-targets [--release] --features boyko-ecs/profiling-analysis --exclude boyko_demo --exclude bench-bevy-vs-boyko`; no `--ignored` anywhere | no. The ≥108 default-leg device tests (1-E) skip silently there; ci.yml:281 itself says boyko-app's lib does not compile on Linux, so workspace jobs cannot exit 0 | a default-leg allow-list (the `device` class is expected on CI), or a GPU-less assertion |
| In-binary drivers (table 2) | `<exe> <worker> --ignored --exact --test-threads=1 --nocapture` | Gate A (K/…:636-648) and the shadow gate (K/…:515-521) already make exit 0 red; others use `status.success()` + artifact | drivers with `.output()` or files can fail on a child marker; inherited-stdio drivers pass it through to the outer runner |
| Other scripts | paradigm-matrix / run-scene / run-vb-lab: `cargo run --example` only | — | — |
| `.claude/agents/tester.md` | no `--ignored` recipe at all | — | — |

## 5. What I could not determine

- Whether `std::thread::current().name()` returns the test fn name under `--test-threads=1`. This decides whether the `<test>` field can be filled automatically: today several labels are not fn names (H3, H8, H9, H7, and every `SKIP: validation disabled` line).
- The runtime of anything above. I ran nothing, so every "ok/red" comes from reading the source.
- The behaviour of `A/sv0_adequacy.rs:664` on boot failure. It depends on the default dirtiness of `SdfEditStaging`, which I did not read.
- Whether E3002 reaches stderr in test processes. That depends on `flush() == NoConsumer` at diag.rs:141 under the test log sink.
- The 1-E counts are a lower bound. Tests that boot through a helper were not counted, and two false positives (particle_containment, sdf_edit_gather) were removed by hand.
- The report's "8th triage item".
- Final line numbers in K/. Those files were being edited during this session.