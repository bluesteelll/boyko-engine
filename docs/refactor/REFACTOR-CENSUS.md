# Refactor census — sequencing the oversized-file campaign

**Tree:** `D:/wt/refactor`, branch `refactor/split-oversized-files`, HEAD
`d552be05be4b4f063b6cb39ddfd63eb4688f83fd`, working tree clean at the time of writing
(`git status --short` empty). Read-only analysis; nothing in the tree was modified.

**Inputs.** `D:/wt/_graph/extract/{modules,uses,crates}.json` (extractor provenance:
root `D:/wt/joltab`, head `d552be05`, dirty `false`, `tree_sitter_rust 0.24.2` — i.e. the SAME
commit this worktree is on, so the counts apply verbatim), `D:/wt/_graph/graph.json` (role,
summary, key items), `D:/wt/_graph/owner-dirty-files.txt`, and the tree itself.

**What was re-derived rather than trusted.** The anchor counts in section 3 come from a Python
port of `tests/internal_docs_anchors.rs::scan_doc`'s *binding* half (sticky `**File:**` targets,
fence margin notes, the `(N)` bare form, the `~` waiver). The port reproduces the gate's own
per-document totals exactly — 223 / 336 / 6 / 177 for `FEATURE_MAP.md` / `SYSTEMS.md` /
`ARCHITECTURE.md` / `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` — verified by running the gate:

```
cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture
  -> 5 passed; FEATURE_MAP 223 anchors / 0 stale, SYSTEMS 336 / 0, ARCHITECTURE 6 / 0,
     MESHLET-VIRTUAL-GEOMETRY-PLAN 177 / 0 (87 of them `~`-waived)
```

So "gated anchors into this file" below is the same number the gate will re-check after a split,
not an approximation of it.

---

## 0. Scale, in one paragraph

The workspace is 1,589 source files / 696,762 lines; 715 files / 401,549 lines of those are
lib+bin targets. **58 lib/bin files are ≥ 1500 lines and they hold 171,250 lines — 42.6 % of all
production lines in 8.1 % of the production files.** 20 are ≥ 3000, 39 are ≥ 2000. The
test/bench side adds 15 files ≥ 1500 (53,403 lines), of which 4 are ≥ 3000 and are tabled
separately below.

---

## 1. The census

### 1.1 Source files (lib/bin targets) of 1500 lines or more — 58 files, 171,250 lines

`pub items` is the extractor's own count of items declared `pub` in the file's own module scope
(`s`truct `e`num `t`rait `f`n `c`onst `t`ype `m`od, `use` = `pub use` re-exports); `pub fn/meth`
is free `pub fn` / `pub fn` inside `impl`; `unsafe (SAFETY)` is `unsafe {}` blocks and the count
of `// SAFETY:` comments in the same file; `importers (w)` is modules that `use`/path into this
module and the total referring-path weight, from `uses.json`. `status` is section 2.

| # | lines | crate | file | pub items | pub fn/meth | unsafe (SAFETY) | importers (w) | status |
|---|------:|-------|------|-----------|------------:|-----------------|--------------:|--------|
| 1 | 10209 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/present/targets.rs` | s2 | 0/9 | 114 (109) | 12 (22) | owner |
| 2 | 8292 | `boyko_app` | `crates/boyko_app/src/gpu_scene/mod.rs` | - | 0/0 | 57 (57) | 7 (10) | unmerged fix/ddgi-host-hook |
| 3 | 6784 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | - | 0/0 | 6 (6) | 3 (5) | free |
| 4 | 6139 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | s1 | 0/0 | 95 (121) | 2 (2) | free |
| 5 | 6127 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/compute.rs` | s16 e4 f125 c139 use1 | 125/42 | 12 (12) | 66 (913) | free |
| 6 | 5333 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/goldens.rs` | s7 f47 c10 | 47/33 | 0 (0) | 24 (136) | unmerged feat/golden-edsl-p0 |
| 7 | 4470 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | s29 c20 | 0/21 | 3 (3) | 15 (103) | unmerged feat/ui-advanced |
| 8 | 4251 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/device.rs` | s6 e2 | 0/32 | 73 (70) | 94 (153) | owner; unmerged chore/msvc-host |
| 9 | 4106 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | s1 | 0/22 | 48 (45) | 14 (14) | unmerged fix/ke13-ke14 |
| 10 | 4010 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/ffi.rs` | s96 e1 c209 t112 u2 m1 use3 | 0/5 | 0 (5) | 67 (428) | free |
| 11 | 3884 | `boyko_physics` | `crates/boyko_physics/src/resources.rs` | s9 e4 c6 | 0/47 | 14 (11) | 68 (209) | lane A4/A5 physics |
| 12 | 3816 | `boyko_app` | `crates/boyko_app/src/runner.rs` | - | 0/0 | 36 (37) | 2 (4) | lane A6 pool/schedule; unmerged fix/ddgi-host-hook |
| 13 | 3542 | `aether_lang` | `crates/aether_lang/src/expand.rs` | f1 | 1/0 | 0 (0) | 1 (1) | free |
| 14 | 3476 | `boyko_physics` | `crates/boyko_physics/src/solver/colored.rs` | s1 | 0/6 | 46 (50) | 4 (4) | lane A4/A5 physics |
| 15 | 3448 | `boyko_render` | `crates/boyko_render/src/render_path_config.rs` | s8 e4 f4 c1 | 4/17 | 0 (0) | 9 (70) | free |
| 16 | 3415 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | - | 0/0 | 50 (68) | 0 (0) | free |
| 17 | 3343 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | s12 t2 | 0/2 | 35 (32) | 44 (80) | free |
| 18 | 3334 | `boyko_physics` | `crates/boyko_physics/src/solver/colored_tests.rs` | - | 0/0 | 2 (2) | 0 (0) | lane A4/A5 physics |
| 19 | 3267 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/compute/tests.rs` | - | 0/0 | 0 (0) | 0 (0) | free |
| 20 | 3009 | `boyko_shaderdsl` | `crates/boyko_shaderdsl/src/emit/shaders.rs` | f45 | 45/0 | 0 (0) | 1 (1) | unmerged feat/ui-advanced |
| 21 | 2944 | `boyko_shaderdsl` | `crates/boyko_shaderdsl/src/emit/mod.rs` | s14 use2 | 0/0 | 0 (0) | 25 (115) | unmerged feat/ui-advanced |
| 22 | 2662 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` | - | 0/0 | 51 (13) | 5 (13) | unmerged feat/reflection; unmerged fix/ke13-ke14 |
| 23 | 2634 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/asset/assets.rs` | s3 c1 | 0/25 | 17 (19) | 5 (8) | free |
| 24 | 2588 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | - | 0/9 | 48 (49) | 0 (0) | free |
| 25 | 2581 | `boyko_render` | `crates/boyko_render/src/light.rs` | s11 e2 f3 c29 | 3/31 | 0 (0) | 21 (118) | free |
| 26 | 2574 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | s2 e1 | 0/25 | 5 (2) | 35 (45) | unmerged fix/ke13-ke14 |
| 27 | 2536 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | s2 | 0/5 | 11 (1) | 5 (7) | lane A6 pool/schedule |
| 28 | 2515 | `boyko_render` | `crates/boyko_render/src/csm_config.rs` | s7 e2 f5 c2 use1 | 5/3 | 0 (0) | 4 (29) | free |
| 29 | 2354 | `boyko_threadpool` | `crates/boyko_threadpool/src/scope.rs` | s1 | 0/2 | 24 (21) | 4 (10) | lane A6 pool/schedule |
| 30 | 2340 | `boyko_ui` | `crates/boyko_ui/src/layout.rs` | f2 | 2/0 | 0 (0) | 8 (16) | unmerged feat/ui-advanced |
| 31 | 2243 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` | s1 | 0/4 | 4 (0) | 11 (11) | free |
| 32 | 2235 | `boyko_render` | `crates/boyko_render/src/hzb.rs` | s4 e3 f7 c2 | 7/19 | 0 (0) | 9 (42) | free |
| 33 | 2166 | `boyko_threadpool` | `crates/boyko_threadpool/src/block.rs` | - | 0/0 | 14 (15) | 3 (3) | lane A6 pool/schedule |
| 34 | 2090 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/profiling/tests.rs` | - | 0/0 | 1 (1) | 0 (0) | free |
| 35 | 2088 | `boyko_render` | `crates/boyko_render/src/mesh_draw.rs` | s4 f3 c2 | 3/9 | 0 (0) | 9 (22) | unmerged fix/asset-validate-prereqs |
| 36 | 2067 | `boyko_macros` | `crates/boyko_macros/src/component.rs` | - | 0/0 | 0 (3) | 2 (2) | unmerged feat/reflection |
| 37 | 2066 | `boyko_shaderdsl` | `crates/boyko_shaderdsl/src/cf.rs` | s1 e1 t1 t1 | 0/0 | 0 (0) | 30 (65) | unmerged feat/ui-advanced |
| 38 | 2053 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | s2 e1 c1 | 0/14 | 0 (0) | 5 (11) | lane A6 pool/schedule |
| 39 | 2015 | `boyko_app` | `crates/boyko_app/src/gpu_scene/particle.rs` | - | 0/0 | 10 (10) | 1 (7) | free |
| 40 | 1979 | `boyko_shaderdsl` | `crates/boyko_shaderdsl/src/bin/emit_particles.rs` | - | 0/0 | 0 (0) | 0 (0) | free |
| 41 | 1918 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | s1 f1 | 1/16 | 16 (6) | 402 (418) | free |
| 42 | 1865 | `boyko_sdf_math` | `crates/boyko_sdf_math/src/brick/tests.rs` | - | 0/0 | 0 (0) | 0 (0) | free |
| 43 | 1830 | `boyko_sdf_math` | `crates/boyko_sdf_math/src/brick.rs` | s1 f25 c15 | 25/4 | 0 (0) | 13 (138) | free |
| 44 | 1826 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | s4 | 0/0 | 83 (4) | 3 (10) | free |
| 45 | 1789 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs` | s1 e2 f22 c1 t1 use5 | 22/4 | 9 (9) | 157 (764) | free |
| 46 | 1784 | `boyko_render` | `crates/boyko_render/src/shadow_atlas.rs` | s7 f7 c8 | 7/3 | 0 (0) | 5 (42) | free |
| 47 | 1719 | `aether_lang` | `crates/aether_lang/src/parse.rs` | - | 0/0 | 0 (0) | 1 (1) | free |
| 48 | 1654 | `boyko_physics` | `crates/boyko_physics/src/solver/simd.rs` | f6 | 6/0 | 9 (9) | 4 (22) | lane A4/A5 physics |
| 49 | 1633 | `boyko_physics` | `crates/boyko_physics/src/systems.rs` | f10 | 10/0 | 2 (2) | 6 (23) | lane A4/A5 physics |
| 50 | 1630 | `boyko_log` | `crates/boyko_log/src/codes.rs` | s5 e3 f9 c3 m3 | 9/6 | 0 (0) | 68 (245) | owner |
| 51 | 1617 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | s1 | 0/35 | 2 (0) | 7 (8) | unmerged fix/ke13-ke14 |
| 52 | 1605 | `boyko_render` | `crates/boyko_render/src/gpu_column.rs` | s4 c1 | 0/31 | 15 (15) | 5 (9) | unmerged feat/ui-advanced |
| 53 | 1595 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/profiling/store.rs` | s6 e3 f2 c10 | 2/24 | 21 (22) | 6 (62) | free |
| 54 | 1592 | `boyko_render` | `crates/boyko_render/src/light_system.rs` | s5 f7 c2 t1 | 7/6 | 2 (2) | 12 (34) | free |
| 55 | 1589 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` | s2 | 0/6 | 24 (14) | 7 (7) | free |
| 56 | 1554 | `boyko_ecs` | `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | s1 f2 | 2/23 | 42 (5) | 22 (24) | owner; unmerged fix/ke13-ke14 |
| 57 | 1548 | `boyko_rhi_vulkan` | `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | s4 e2 f1 c39 | 1/16 | 9 (9) | 13 (92) | free |
| 58 | 1517 | `boyko_render` | `crates/boyko_render/src/upload.rs` | f21 | 21/0 | 23 (23) | 1 (21) | unmerged fix/ddgi-host-hook |

**Four of these 58 are `#[cfg(test)]` modules that live inside a lib target** and so are
production files only by target kind: `solver/colored_tests.rs` (3334), `compute/tests.rs`
(3267), `profiling/tests.rs` (2090), `brick/tests.rs` (1865) — 10,556 lines. Three of them are
already the *product* of an earlier split (section 4); they are listed because they are still
oversized, not because they are unsplit god-files.

### 1.2 Test / bench files of 3000 lines or more — 4 files, 28,764 lines

| # | lines | crate | target | file | macros | unsafe (SAFETY) | status |
|---|------:|-------|--------|------|--------|-----------------|--------|
| 1 | 10434 | `boyko_rhi_vulkan` | test | `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | 0 | 55 (56) | free |
| 2 | 8342 | `boyko_rhi_vulkan` | test | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs` | 0 | 27 (27) | free |
| 3 | 6672 | `boyko_rhi_vulkan` | test | `crates/boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | 0 | 0 (0) | free |
| 4 | 3316 | `boyko_ecs` | bench | `crates/boyko_ecs/benches/cull_diagnostic.rs` | 0 | 0 (0) | free |

Lower priority by the task's own ruling, and correctly so: none of them is imported by anything
(0 importers each), so a split cannot break a downstream path. But note the two `boyko_rhi_vulkan`
harness tests are the **two largest files in the repository**, larger than any production file,
and `docs/REFACTORING-PLAN.md` §B6 already diagnosed why (each test clones the whole
boot/present/BMP-readback setup). `window_present_gbuffer.rs` also carries `#![cfg(windows)]` at
L54 and is the test binary behind 60-odd golden pins (section 3.4).

### 1.3 Role and summary, per census file

From `graph.json` (`desc_source` is `written` for the large files, `auto` for the rest).

- **`crates/boyko_rhi_vulkan/src/present/targets.rs`** (10209 lines, role `gpu`, group `rhi-core`)  
  The per-extent render-target and descriptor-set allocator — `GBufferTargets` (+ `ForwardTargets`, `VbTargets`, `VbClassifyTargets`, `HzbTargets`), the per-frame `GBufferFrame` holder, and the fence-safe `sync_gbuffer` recreate.  
  key items: GBufferTargets, GBufferFrame, GBufferTargets::sync_gbuffer, GBufferTargets::create, TargetsProfile, AaArm, ForwardTargets, VbTargets
- **`crates/boyko_app/src/gpu_scene/mod.rs`** (8292 lines, role `gpu`, group `app`)  
  `GpuSceneBundles` — the static GPU resource boot for the windowed G-buffer scene: every pipeline, layout, sampler and seeded buffer the frame recorder needs, plus the per-frame scene assembler.  
  key items: GpuSceneBundles, GpuSceneBundles::boot, GpuSceneBundles::scene, GpuSceneBundles::destroy, DrawListScratch, VbCullReadback, MotionVecResources, INSTANCE_CAPACITY
- **`crates/boyko_rhi_vulkan/src/present/graph_bridge.rs`** (6784 lines, role `glue`, group `rhi-core`)  
  The bridge between the in-house Render Dependency Graph and raw Vulkan: the per-path pass plans, the three `BarrierSink` implementations that resolve a derived barrier to a physical handle, and the per-frame declarators.  
  key items: GbufferPassPlan, ForwardPassPlan, VbPassPlan, GbufferBarrierSink, ForwardBarrierSink, VbBarrierSink, declare_frame_graph, declare_deferred_graph
- **`crates/boyko_rhi_vulkan/src/present/passes/vb.rs`** (6139 lines, role `gpu`, group `rhi-core`)  
  `Renderer::record_vb` — the VisibilityBuffer render path: sky, GPU-culled indirect id-raster (optionally split early/late around an HZB occlusion test), optional material classification, fused resolve or classified shade, TAA, particles, present-blit.  
  key items: Renderer::record_vb, VbRecordProbe, record_hzb_poison_build, record_vb_viewt_dispatch, TsWitness
- **`crates/boyko_rhi_vulkan/src/compute.rs`** (6127 lines, role `data`, group `rhi-core`)  
  The shader-asset registry: ~110 committed SPIR-V blobs behind named accessors, the push-constant PODs, and the CPU bakers.  
  key items: write_pattern_spirv, gbuffer_mrt_fs_spirv, vb_resolve_spirv, vb_shade_split_spirv, cluster_cull_spirv, hzb_build_spirv, bake_brick_atlas, rebake_dirty_brick_atlas
- **`crates/boyko_rhi_vulkan/src/goldens.rs`** (5333 lines, role `test`, group `rhi-core`)  
  The CPU host oracles: ~5.3 kLOC of `golden_*` functions mirroring the marcher / lighting / SSAO / cluster-cull shader math bit-for-bit.  
  key items: golden_write_pattern, golden_chained, GoldenLight, GoldenMaterial, MarcherAttributes, HierCullStats, DdgiProbeTap
- **`crates/boyko_rhi_vulkan/src/present/scene_types.rs`** (4470 lines, role `data`, group `rhi-core`)  
  The render-input data bundles and push-constant PODs the host fills every frame — `Scene`, `SampledComposite`, `UiPass`, `GBufferScene` and its ~20 per-feature activation sub-bundles.  
  key items: GBufferScene, Scene, SampledComposite, UiPass, GBufferMeshDraw, ResolvedRenderPathGpu, HzbPlan, HzbDumpLayout
- **`crates/boyko_rhi_vulkan/src/device.rs`** (4251 lines, role `ffi`, group `rhi-core`)  
  The boot path and the owning `VulkanContext`: loader → instance → physical device + caps → logical device + queue + fn tables, plus the growable memory pools.  
  key items: VulkanContext, VulkanContext::boot, VulkanContext::boot_singleton, VulkanContext::destroy_singleton, DeviceCaps, DeviceFns, InstanceConfig, RtTier
- **`crates/boyko_ecs/src/ecs/memory/component_pool.rs`** (4106 lines, role `data`, group `ecs-storage`)  
  `ComponentPool` - one type-erased SoA column of component rows plus its two per-row tick sub-regions; the largest file in the crate (4106 lines, 48 `unsafe` blocks).  
  key items: ComponentPool, PoolBacking, UNTRACKED_TICKS, grow_rows, swap_remove, write_at, take_at, commit_units
- **`crates/boyko_rhi_vulkan/src/ffi.rs`** (4010 lines, role `ffi`, group `rhi-core`)  
  The hand-declared raw Vulkan FFI surface: 96 `#[repr(C)]` structs, 112 PFN typedefs, 209 constants, and the Win32 `os` submodule.  
  key items: VkDevice, VkImageMemoryBarrier, VkGraphicsPipelineCreateInfo, VkWriteDescriptorSet, PfnVkCmdPipelineBarrier, os::MSG, os::WNDCLASSEXW
- **`crates/boyko_physics/src/resources.rs`** (3884 lines, role `data`, group `physics`)  
  The step data plane: every preallocated, capacity-reused physics resource from `PhysicsConfig` to `SolverScratch`.  
  key items: PhysicsConfig, SolverScratch, BodyState, TouchedMask, ContactPairs, BroadphaseGrid, Manifolds, ConstraintGraph
- **`crates/boyko_app/src/runner.rs`** (3816 lines, role `logic`, group `app`)  
  The windowed G-buffer runner — the whole app lifecycle in order: device-singleton boot, window-host boot, World GPU residents, `finish()`, the frame loop, and the teardown.  
  key items: run_windowed, frame_loop, teardown, destroy_host_gpu_chain, record_teardown_stats, dump_diagnostics, ingest_captured, parse_hier_cull_env
- **`crates/aether_lang/src/expand.rs`** (3542 lines, role `logic`, group `scene-lang-apps`)  
  The expander: emits the canonical hand-written Rust surface for every construct, with `boyko_macros` left as the single codegen authority.  
  key items: expand, recovered, expand_v1, component, system_fn, plugin_impl, machine_items, material_fn
- **`crates/boyko_physics/src/solver/colored.rs`** (3476 lines, role `logic`, group `physics`)  
  `ColoredSoftStepSolver` — the SoA, graph-colored, parallel + AVX2 TGS-Soft solver with island sleeping.  
  key items: ColoredSoftStepSolver, ContactColumns, ColorSolvePtrs, ContactSolveView, ContactBuildView, solve_colored, solve_colored_sleeping, MIN_PARALLEL_SLOTS_PER_COLOR
- **`crates/boyko_render/src/render_path_config.rs`** (3448 lines, role `logic`, group `render`)  
  The render-path config surface and its boot-locked resolver — the largest module in the crate.  
  key items: RenderPathConfig, ResolvedRenderPath, RenderPath, GeometryLegs, RenderPathDegrade, RenderPathFrozenConsumers, resolve_render_path, resolve_rules
- **`crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs`** (3415 lines, role `gpu`, group `rhi-core`)  
  `Renderer::record_gbuffer` — the Deferred render path's whole command stream: interp/raster → coarse cull → SDF march → SSAO + a-trous → DDGI → light cull → shadow depth + RT denoise → deferred resolve → particles → present-blit.  
  key items: Renderer::record_gbuffer, GbufWitness, W2104
- **`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs`** (3343 lines, role `data`, group `ecs-storage`)  
  `QueryFilter`: archetype-level and per-row filtering, and every built-in filter term.  
  key items: QueryFilter, ArchetypalQueryFilter, With, Without, Or, Added, Changed, DenseFilterFetch
- **`crates/boyko_physics/src/solver/colored_tests.rs`** (3334 lines, role `test`, group `physics`)  
  Pure-function unit tests for the colored solver, driven without a schedule or threadpool so they run under Miri.  
  key items: -
- **`crates/boyko_rhi_vulkan/src/compute/tests.rs`** (3267 lines, role `test`, group `rhi-core`)  
  The crate's in-tree `#[cfg(test)]` unit suite for the compute assets and the CPU bakers (3267 lines, 106 test fns).  
  key items: -
- **`crates/boyko_shaderdsl/src/emit/shaders.rs`** (3009 lines, role `tool`, group `shader-math`)  
  The 45 `emit_hlsl_*` entry points — the public functions that trace a generic body over `Emit`/`EmitCf` and return the HLSL text.  
  key items: emit_hlsl_field, emit_hlsl_normal, emit_hlsl_sdf_soft_shadow, emit_hlsl_ssao, emit_hlsl_m2_brick_cubic_hit, emit_hlsl_probe_march, emit_hlsl_particle_integrate, emit_hlsl_transform_interp
- **`crates/boyko_shaderdsl/src/emit/mod.rs`** (2944 lines, role `tool`, group `shader-math`)  
  The `Emit` backend: a build-time SSA arena, the statement IR, and the HLSL printer that walks them into shader text.  
  key items: Emit, EmitCf, Names, Node, EmitTy, ARENA, Var, BufParam
- **`crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs`** (2662 lines, role `logic`, group `ecs-storage`)  
  Shared low-level archetype-migration scaffolding for insert, remove, tag and clone paths.  
  key items: -
- **`crates/boyko_ecs/src/ecs/core/asset/assets.rs`** (2634 lines, role `data`, group `ecs-storage`)  
  `Assets<T>` — the world-global, per-asset-type storage table: a standalone `ComponentPool` + `LiveBitmap` + LIFO free list, with a packed `{generation, state}` word and the refcount lifetime driver.  
  key items: Assets, RetireTicket, AssetKind, GEN_UNSYNCED, inc_ref, dec_ref, reserve, fill
- **`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs`** (2588 lines, role `gpu`, group `rhi-core`)  
  The `RhiDevice for VulkanContext` body: every resource-creation and destruction verb, lowered to raw FFI.  
  key items: RhiDevice::create_buffer, RhiDevice::create_texture, RhiDevice::create_graphics_pipeline, RhiDevice::create_bind_group, RhiDevice::create_query_pool, RhiDevice::sample_device_clock
- **`crates/boyko_render/src/light.rs`** (2581 lines, role `data`, group `render`)  
  The light component vocabulary and the std430 GPU light table records.  
  key items: GpuLight, LightHeaderGpu, LightingConfig, ClusterConfig, DirectionalLight, PointLight, SpotLight, SkyLight
- **`crates/boyko_ecs/src/ecs/core/archetype/archetype.rs`** (2574 lines, role `data`, group `ecs-storage`)  
  `Archetype` - one table: a fixed component signature, one `ComponentPool` per signature component, the entity-id column, the enable-bit columns and the per-row tick plumbing.  
  key items: Archetype, Column, RemoveOutcome, create_entity, remove_entity, move_out_entity, filtered_signature_mask, entity_ids_slice
- **`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`** (2536 lines, role `logic`, group `ecs-storage`)  
  `Schedule` — the runnable artefact, and the parallel executor that runs it.  
  key items: Schedule, run, executor_main_loop, apply_window_drain, try_dispatch_ready, evaluate_ready_conditions, run_state_transitions, check_change_ticks
- **`crates/boyko_render/src/csm_config.rs`** (2515 lines, role `logic`, group `render`)  
  The cascade-fit policy for Cascaded Shadow Maps — cold, CPU, unit-testable.  
  key items: CsmConfig, ResolvedCsm, CascadeData, CsmFit, CsmFitMode, CsmFitState, CsmPcfKernel, CsmResolveSet
- **`crates/boyko_threadpool/src/scope.rs`** (2354 lines, role `logic`, group `runtime-infra`)  
  `Scope` - borrow-erased fork/join - and `ScopeShared`, the pending counter whose last decrement releases the joiner.  
  key items: Scope, Scope::spawn, Scope::spawn_batch, Scope::drop, ScopeShared, register_task, complete_task, capture_panic
- **`crates/boyko_ui/src/layout.rs`** (2340 lines, role `logic`, group `ui-input`)  
  The in-house flexbox-style layout solver: a change-detecting discovery system plus an exclusive apply system.  
  key items: ui_layout_discovery, ui_layout_apply, layout_root, measure_node, position_node, write_rect, refresh_roots, collect_stretch_and_measure
- **`crates/boyko_ecs/src/ecs/core/iters/query/state.rs`** (2243 lines, role `data`, group `ecs-storage`)  
  `QueryDataState<D, F>` — the per-system query state cache the drivers run on.  
  key items: QueryDataState, post_filter_matched, matched_ids_pre_terms
- **`crates/boyko_render/src/hzb.rs`** (2235 lines, role `logic`, group `render`)  
  The host oracle for the hierarchical-Z pyramid and the two-pass occlusion test.  
  key items: HzbLayout, HzbAxis, OcclusionVerdict, KeepReason, ScreenRect, TexelSelection, prev_pow2
- **`crates/boyko_threadpool/src/block.rs`** (2166 lines, role `data`, group `runtime-infra`)  
  `ScopeBlock`, the per-scope bump allocator whose chunks hold task cells, and `BlockPtr`, the only pointer type they come back as.  
  key items: ScopeBlock, ScopeBlock::emplace, ScopeBlock::free_all, BlockPtr, BlockPtr::erase, CHUNK0, MAX_CHUNKS
- **`crates/boyko_ecs/src/ecs/core/profiling/tests.rs`** (2090 lines, role `test`, group `ecs-storage`)  
  The profiling subsystem's in-crate rung-2 mechanism suite (`#[cfg(test)]`).  
  key items: test_serial
- **`crates/boyko_render/src/mesh_draw.rs`** (2088 lines, role `logic`, group `render`)  
  The bucketed instance gather and per-mesh draw batches — the heart of the mesh draw path.  
  key items: gather_mesh_draws, sync_vb_instance_ring_system, MeshRenderScratch, DrawBatch, PerInstanceMaterial, PerInstanceMaterialTex
- **`crates/boyko_macros/src/component.rs`** (2067 lines, role `tool`, group `runtime-infra`)  
  `#[derive(Component)]` - the largest macro in the crate, and the one that folds in six optional feature axes.  
  key items: expand, ComponentHookPaths
- **`crates/boyko_shaderdsl/src/cf.rs`** (2066 lines, role `api`, group `shader-math`)  
  The `Cf` control-flow backend axis plus its host instantiation `EvalCf` — what lets a whole marcher (loops, branches, early return, typed locals) be one generic Rust body.  
  key items: Cf, EvalCf, Flow, LoopOp, unroll_for, if_, cont, ret
- **`crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs`** (2053 lines, role `logic`, group `ecs-storage`)  
  `ScheduleBuilder` — the config-time constructor that lowers systems and hints into a runnable `Schedule`.  
  key items: ScheduleBuilder, build, try_build, ConfigureSet, ScheduleBuildError, MAX_SYSTEMS_PER_SCHEDULE, tarjan_scc, kahn_topo_sort
- **`crates/boyko_app/src/gpu_scene/particle.rs`** (2015 lines, role `gpu`, group `app`)  
  `ParticleGpuBundle` — the host-owned GPU resources of the particle system, built only when the owner armed the subsystem at boot.  
  key items: ParticleGpuBundle, ParticleCountersRaw, build_particle_bundle, particle_upload_slots, read_particle_counters
- **`crates/boyko_shaderdsl/src/bin/emit_particles.rs`** (1979 lines, role `tool`, group `shader-math`)  
  Binary (`--features emit`): generates and WRITES all eight committed GPU-particle shaders.  
  key items: main
- **`crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`** (1918 lines, role `api`, group `ecs-storage`)  
  `EcsMaster` - the world: entities, archetypes, dense stores, resources, events, ticks, caches, hooks/observer stores.  
  key items: EcsMaster, QueryStateCache, QueryCacheSlot, world_id, change_tick, current_tick, should_run_check_ticks, query
- **`crates/boyko_sdf_math/src/brick/tests.rs`** (1865 lines, role `test`, group `shader-math`)  
  In-crate `cfg(test)` module for `brick.rs` — 1865 lines, 48 test functions pinning classification, the snorm round-trip, the trilinear bound and the clip-map/toroidal math.  
  key items: -
- **`crates/boyko_sdf_math/src/brick.rs`** (1830 lines, role `logic`, group `shader-math`)  
  The CPU brick reference — the bit-exact oracle for the GPU brick atlas: classification, snorm fill, trilinear reconstruct, clip-map level math and the cubic surface hit.  
  key items: PointerGrid, classify_brick, fill_brick, trilinear_reconstruct, build_dirty_pointer_grid, dirty_world_aabb, toroidal_slot, for_each_revealed_cell
- **`crates/boyko_ecs/src/ecs/core/iters/query/iter.rs`** (1826 lines, role `logic`, group `ecs-storage`)  
  `QueryIter` / `QueryIterMut`: the hot-path per-row cursors.  
  key items: QueryIter, QueryIterMut, QueryIterEntities, QueryIterEntitiesMut
- **`crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs`** (1789 lines, role `data`, group `ecs-storage`)  
  The process-global component registry: id minting, `ComponentLayout`, storage/residency classes, hooks-by-id, and the root of five cold sub-registries.  
  key items: ComponentLayout, DropFn, StorageKind, ResidencyKind, MAX_COMPONENTS, register_new, register_layout, is_signature_storage
- **`crates/boyko_render/src/shadow_atlas.rs`** (1784 lines, role `logic`, group `render`)  
  The cold slot-assignment policy for sparse perspective (spot/point) shadow maps.  
  key items: resolve_shadow_atlas, ShadowConfig, ResolvedShadowAtlas, PunctualSlotAssignment, PunctualResolveSet, FaceTransform, SpotShadowInput, PointShadowInput
- **`crates/aether_lang/src/parse.rs`** (1719 lines, role `logic`, group `scene-lang-apps`)  
  The block parser: an optional version header, then dispatch on the leading contextual keyword, one `Parse` impl per construct — with speculative per-construct recovery.  
  key items: AetherBlock::parse, parse_version_header, parse_component, parse_system, parse_machine, parse_material, parse_scene, MATERIAL_KEYS
- **`crates/boyko_physics/src/solver/simd.rs`** (1654 lines, role `logic`, group `physics`)  
  O1 width-only AVX2 SoA kernels for the inertia refresh and the gravity/position/quaternion integrate, bit-identical to scalar.  
  key items: refresh_inertia_x8, integrate_x8, cross8, dot8, mat3mulvec8, effective_mass_x8, apply_impulse_blend_x8
- **`crates/boyko_physics/src/systems.rs`** (1633 lines, role `logic`, group `physics`)  
  The nine pipeline stages as ordinary ECS systems: integrate, gather, broadphase, narrowphase (+SDF), build-graph, solve (x2), apply.  
  key items: physics_integrate, physics_gather, physics_broadphase, physics_narrowphase, physics_narrowphase_sdf, physics_build_graph, physics_solve_colored, physics_solve_step
- **`crates/boyko_log/src/codes.rs`** (1630 lines, role `data`, group `runtime-infra`)  
  The diagnostic-code registry: one `codes! { .. }` invocation generating a dense table, a dense index per code, per-code delivery policy and `explain`.  
  key items: codes!, WarnCode, ErrorCode, PanicCode, CodeIdx, CodeStatus, RatePolicy, DiagInfo
- **`crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs`** (1617 lines, role `logic`, group `ecs-storage`)  
  `ArchetypeMaster` - the world's archetype manager: the creation funnel, mask lookup, the generation counters, the observer registry and the EnableTag presence oracle.  
  key items: ArchetypeMaster, create_archetype, get_or_create_archetype, archetype_generation, structural_generation, enable_generation, add_observer, iter_archetypes_mut
- **`crates/boyko_render/src/gpu_column.rs`** (1605 lines, role `gpu`, group `render`)  
  The GPU-resident column manager and the !Send RHI context that owns it.  
  key items: GpuColumnManager, GpuColumnMeta, ResolvedColumn, RhiContext
- **`crates/boyko_ecs/src/ecs/core/profiling/store.rs`** (1595 lines, role `data`, group `ecs-storage`)  
  `Profiler` — the durable frame-major SoA on a process-lifetime `VmReservation`, plus the `arm`/`disarm` pair that is the whole enable path.  
  key items: Profiler, FrameRecord, FrameState, Cell, CellLabel, DropCounters, ProfilerConfig, ArmOutcome
- **`crates/boyko_render/src/light_system.rs`** (1592 lines, role `logic`, group `render`)  
  The L0 collection fold: ECS light components into the GPU light table staging slice.  
  key items: collect_lights, LightTableStaging, LightTableGeneration, LightCollectSet, LightSeedState, SetLightEnabledById, LightChanged
- **`crates/boyko_ecs/src/ecs/core/commands/command_queue.rs`** (1589 lines, role `data`, group `ecs-storage`)  
  `CommandQueue` — the type-erased, packed byte arena every `Commands` param writes into.  
  key items: CommandQueue, CommandMark, push, apply, mark, rewind
- **`crates/boyko_ecs/src/ecs/core/iters/query/query.rs`** (1554 lines, role `api`, group `ecs-storage`)  
  `Query<'w, 's, D, F>` — the typed query `SystemParam` every gameplay system takes.  
  key items: Query, iter, iter_mut, par_iter, for_each_chunk, with_tag
- **`crates/boyko_rhi_vulkan/src/present/gpu_zone.rs`** (1548 lines, role `gpu`, group `rhi-core`)  
  The GPU zone recorder: a ring of timestamp query-pool slots, a host-side per-pair witness, and the 2x2 label that says whether a measured number exists at all.  
  key items: GpuZoneRecorder, GpuLabel, PairResult, RetiredFrame, RetireCause, RetireScratch, MAX_GPU_PAIRS, GPU_RING_DEPTH
- **`crates/boyko_render/src/upload.rs`** (1517 lines, role `gpu`, group `render`)  
  Token-typed per-slot ring uploads — the WHAT of the host's per-frame GPU writes.  
  key items: upload_camera_ring, FrameWriteToken (consumed), upload_resolved_*
- **`crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`** (10434 lines, role `test`, group `rhi-core`)  
  Render P1c GPU gate — the first image-based hybrid frame on screen; at 10.4 kLOC the largest test binary in the crate.  
  key items: -
- **`crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs`** (8342 lines, role `test`, group `rhi-core`)  
  Render P1b GPU gate — the offscreen image-based SDF + mesh hybrid composite writing an MRT G-buffer; at 8.3 kLOC one of the two largest test binaries.  
  key items: -
- **`crates/boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs`** (6672 lines, role `test`, group `rhi-core`)  
  VG R3 P2 gate G4 — the derived barrier stream for the VisibilityBuffer frame asserted field by field, per configuration; 6.7 kLOC.  
  key items: -
- **`crates/boyko_ecs/benches/cull_diagnostic.rs`** (3316 lines, role `bench`, group `ecs-storage`)  
  [benchmarks-misc] EnableTag positive-term archetype-cull DIAGNOSTIC (task #5) -- profile-first probe against the CURRENT (NO-OP cull) build.  
  key items: cull_diagnostic

---

## 2. Blocked sets, with evidence

Three independent blockers. A file is **free** only if none applies.

### 2.1 Blocked by the owner (uncommitted edits in the main checkout)

`D:/wt/_graph/owner-dirty-files.txt` lists 44 paths the owner has uncommitted edits to in
`D:/claude/BoykoEngine` (branch `feat/multi-paradigm-render`, HEAD `d1e77f4f`). **14 are Rust
sources and exactly 4 of those are census files:**

| census file | lines |
|---|---:|
| `crates/boyko_rhi_vulkan/src/present/targets.rs` | 10209 |
| `crates/boyko_rhi_vulkan/src/device.rs` | 4251 |
| `crates/boyko_log/src/codes.rs` | 1630 |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | 1554 |

The other 10 owner-dirty Rust files (`boyko_app/src/plugins.rs`, `boyko_physics/src/lib.rs`,
`boyko_physics/src/plugin.rs`, `boyko_render/src/loaders/glb.rs`, `boyko_render/src/mesh_assets.rs`,
`boyko_rhi/src/enums.rs`, `boyko_ecs`'s `par_chunk.rs` / `query_type_registry.rs` / `query_view.rs`,
`boyko_render/tests/vg_glb_decode.rs`) are not census files themselves, but `boyko_physics/src/lib.rs`
is the *declaring* file of four physics census files (section 2.5), and the three sibling query files
sit in the same directory as three census files — a split that adds files there will diff-collide with
the owner's in-flight work even though it does not edit his files.

⚠️ `crates/boyko_rhi_vulkan/src/present/targets.rs` is simultaneously the **largest production
file in the repo** and owner-dirty. It is also `docs/REFACTORING-PLAN.md` §B1's "WORST offender
(do first)" — written when it was 2689 lines. It is now 10209. It cannot be touched by this
campaign until the owner's edits land.

### 2.2 Blocked by an open lane

Declared by the task: `crates/boyko_physics/**` (lane A4/A5 in `D:/wt/joltab`) and
`crates/boyko_threadpool/**` + `crates/boyko_ecs/src/ecs/core/schedule/**` +
`crates/boyko_app/src/runner.rs` (lane A6 in `D:/wt/uploadleak`). Refined by reading both trees:

```
git -C D:/wt/joltab status --short     -> ?? crates/boyko_physics/tests/apply_row_alignment.rs
                                          ?? crates/boyko_physics/tests/support_loss_wakes_sleepers.rs
   (branch merge/ke16-into-ecsnative, HEAD d552be05 — same commit as this tree)
git -C D:/wt/uploadleak status --short -> ?? crates/boyko_ecs/tests/a6_schedule_panic_propagation.rs
                                          ?? crates/boyko_threadpool/tests/a6_panic_propagation.rs
   (branch fix/pool-panic-and-reservoir-loom, HEAD d552be05 — same commit as this tree)
```

**Neither lane has modified a tracked file yet** — each has only new, untracked test files, and
both sit on the same commit as this worktree. That is a *snapshot*, not a guarantee: both lanes
are open and both are expected to edit their crates' lib sources (A6's subject is pool panic
propagation and the reservoir, A4/A5's is the physics solver). The lane block therefore stands on
the declared scope, not on today's `git status`. Census files caught by it:

| census file | lines | lane |
|---|---:|---|
| `crates/boyko_physics/src/resources.rs` | 3884 | A4/A5 (also owner-dirty) |
| `crates/boyko_physics/src/solver/colored.rs` | 3476 | A4/A5 |
| `crates/boyko_physics/src/solver/colored_tests.rs` | 3334 | A4/A5 |
| `crates/boyko_physics/src/solver/simd.rs` | 1654 | A4/A5 |
| `crates/boyko_physics/src/systems.rs` | 1633 | A4/A5 |
| `crates/boyko_app/src/runner.rs` | 3816 | A6 |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | 2536 | A6 |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | 2053 | A6 |
| `crates/boyko_threadpool/src/scope.rs` | 2354 | A6 |
| `crates/boyko_threadpool/src/block.rs` | 2166 | A6 |

### 2.3 Blocked by an unmerged lane

The task named three (`D:/wt/ui`, `D:/wt/reflect`, `D:/wt/merge`). Those three alone understate
the blockage, so **all 22 non-merged branches in `git worktree list` were diffed**
(`git diff --name-only d552be05...<branch>`, i.e. the branch's own side since the merge base) and
intersected with the census. Ten branches change Rust files; six of them hit census files:

| branch | worktree | last commit | ahead | .rs changed | census files it changes |
|---|---|---|---:|---:|---|
| `feat/ui-advanced` | `D:/wt/ui` | 2026-08-29 | 16 | 69 | `boyko_ui/src/layout.rs`, `boyko_shaderdsl/src/{emit/mod.rs,emit/shaders.rs,cf.rs}`, `boyko_render/src/gpu_column.rs`, `boyko_rhi_vulkan/src/present/scene_types.rs` |
| `feat/reflection` | `D:/wt/reflect` | 2026-08-30 | 20 | 82 | `boyko_ecs/.../commands/migration_helpers.rs`, `boyko_macros/src/component.rs` |
| `fix/ke13-ke14` | `D:/wt/kernel` | 2026-09-10 | 1 | 25 | `boyko_ecs/.../archetype/archetype.rs`, `.../archetype_master.rs`, `.../commands/migration_helpers.rs`, `.../iters/query/query.rs`, `boyko_ecs/src/ecs/memory/component_pool.rs` |
| `fix/ddgi-host-hook` | `D:/wt/ddgi` | 2026-09-10 | 7 | 17 | `boyko_app/src/gpu_scene/mod.rs`, `boyko_app/src/runner.rs`, `boyko_render/src/upload.rs` |
| `fix/asset-validate-prereqs` | `D:/wt/assets` | 2026-09-10 | 5 | 8 | `boyko_render/src/mesh_draw.rs` |
| `feat/golden-edsl-p0` | `D:/wt/golden` | 2026-09-10 | 5 | 4 | `boyko_rhi_vulkan/src/goldens.rs` |
| `chore/msvc-host` | `D:/wt/msvc` | 2026-09-13 | 3 | 17 | `boyko_rhi_vulkan/src/device.rs` (also owner-dirty) |
| `merge/ke16-into-render` | `D:/wt/merge` | 2026-09-10 | 9 | 1 | **none** — 17 files, 16 of them docs + `tests/gaia_ruled_vs_open_census.rs` |
| `chore/doc-gates`, `docs/ab-register-sync`, `fix/census-post-lto-object`, `feat/ecs-native-storage` | — | 2026-09-10..13 | 1-10 | 1-3 | none |

Branches whose diff against `d552be05` contains **no** `.rs` change at all (already contained in
this base, or docs-only): `feat/aether-v2`, `feat/threadpool-ke16`, `fix/rejected-gpu-upload-leak`,
`feat/multi-paradigm-render`, `feat/shadow-denoise-and-ssao-blur`, `phase5-gpucolumn`,
`serialize-perf`, `fix/inherited-red-gates`, `fix/miri-protector-arming`, `timing/per-stage-pyramid`.

⚠️ Two caveats on this table, both in the conservative direction. (1) `d552be05` is **not** an
ancestor of any of these branches (`git merge-base --is-ancestor` = false for all three named
lanes; the merge bases are `5ec1699f` for ui/reflect and `02325b01` for merge), so a file listed
here may in principle carry a change this base already has by another route — the list
over-approximates. (2) `D:/wt/merge`'s branch is the one the memory index says is cleared for the
owner's final step; it blocks **no** census file, so it need not gate anything here.

### 2.4 Verdict per census file

29 of the 58 source files are free; 29 are blocked. All 4 test/bench files are free.

| file | lines | classification | unblocked by |
|---|---:|---|---|
| `crates/boyko_rhi_vulkan/src/present/targets.rs` | 10209 | blocked-by-owner | owner commits `feat/multi-paradigm-render` |
| `crates/boyko_app/src/gpu_scene/mod.rs` | 8292 | blocked-by-unmerged(fix/ddgi-host-hook) | `fix/ddgi-host-hook` merges |
| `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | 6784 | **free** | - |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | 6139 | **free** | - |
| `crates/boyko_rhi_vulkan/src/compute.rs` | 6127 | **free** | - |
| `crates/boyko_rhi_vulkan/src/goldens.rs` | 5333 | blocked-by-unmerged(feat/golden-edsl-p0) | `feat/golden-edsl-p0` merges |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | 4470 | blocked-by-unmerged(feat/ui-advanced) | `feat/ui-advanced` merges |
| `crates/boyko_rhi_vulkan/src/device.rs` | 4251 | blocked-by-owner + blocked-by-unmerged(chore/msvc-host) | owner commits `feat/multi-paradigm-render`; `chore/msvc-host` merges |
| `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | 4106 | blocked-by-unmerged(fix/ke13-ke14) | `fix/ke13-ke14` merges |
| `crates/boyko_rhi_vulkan/src/ffi.rs` | 4010 | **free** | - |
| `crates/boyko_physics/src/resources.rs` | 3884 | blocked-by-lane(A4/A5 physics (D:/wt/joltab)) | that lane lands |
| `crates/boyko_app/src/runner.rs` | 3816 | blocked-by-lane(A6 pool/schedule (D:/wt/uploadleak)) + blocked-by-unmerged(fix/ddgi-host-hook) | that lane lands; `fix/ddgi-host-hook` merges |
| `crates/aether_lang/src/expand.rs` | 3542 | **free** | - |
| `crates/boyko_physics/src/solver/colored.rs` | 3476 | blocked-by-lane(A4/A5 physics (D:/wt/joltab)) | that lane lands |
| `crates/boyko_render/src/render_path_config.rs` | 3448 | **free** | - |
| `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | 3415 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | 3343 | **free** | - |
| `crates/boyko_physics/src/solver/colored_tests.rs` | 3334 | blocked-by-lane(A4/A5 physics (D:/wt/joltab)) | that lane lands |
| `crates/boyko_rhi_vulkan/src/compute/tests.rs` | 3267 | **free** | - |
| `crates/boyko_shaderdsl/src/emit/shaders.rs` | 3009 | blocked-by-unmerged(feat/ui-advanced) | `feat/ui-advanced` merges |
| `crates/boyko_shaderdsl/src/emit/mod.rs` | 2944 | blocked-by-unmerged(feat/ui-advanced) | `feat/ui-advanced` merges |
| `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` | 2662 | blocked-by-unmerged(feat/reflection) + blocked-by-unmerged(fix/ke13-ke14) | `feat/reflection` merges; `fix/ke13-ke14` merges |
| `crates/boyko_ecs/src/ecs/core/asset/assets.rs` | 2634 | **free** | - |
| `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | 2588 | **free** | - |
| `crates/boyko_render/src/light.rs` | 2581 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | 2574 | blocked-by-unmerged(fix/ke13-ke14) | `fix/ke13-ke14` merges |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | 2536 | blocked-by-lane(A6 pool/schedule (D:/wt/uploadleak)) | that lane lands |
| `crates/boyko_render/src/csm_config.rs` | 2515 | **free** | - |
| `crates/boyko_threadpool/src/scope.rs` | 2354 | blocked-by-lane(A6 pool/schedule (D:/wt/uploadleak)) | that lane lands |
| `crates/boyko_ui/src/layout.rs` | 2340 | blocked-by-unmerged(feat/ui-advanced) | `feat/ui-advanced` merges |
| `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` | 2243 | **free** | - |
| `crates/boyko_render/src/hzb.rs` | 2235 | **free** | - |
| `crates/boyko_threadpool/src/block.rs` | 2166 | blocked-by-lane(A6 pool/schedule (D:/wt/uploadleak)) | that lane lands |
| `crates/boyko_ecs/src/ecs/core/profiling/tests.rs` | 2090 | **free** | - |
| `crates/boyko_render/src/mesh_draw.rs` | 2088 | blocked-by-unmerged(fix/asset-validate-prereqs) | `fix/asset-validate-prereqs` merges |
| `crates/boyko_macros/src/component.rs` | 2067 | blocked-by-unmerged(feat/reflection) | `feat/reflection` merges |
| `crates/boyko_shaderdsl/src/cf.rs` | 2066 | blocked-by-unmerged(feat/ui-advanced) | `feat/ui-advanced` merges |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | 2053 | blocked-by-lane(A6 pool/schedule (D:/wt/uploadleak)) | that lane lands |
| `crates/boyko_app/src/gpu_scene/particle.rs` | 2015 | **free** | - |
| `crates/boyko_shaderdsl/src/bin/emit_particles.rs` | 1979 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | 1918 | **free** | - |
| `crates/boyko_sdf_math/src/brick/tests.rs` | 1865 | **free** | - |
| `crates/boyko_sdf_math/src/brick.rs` | 1830 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | 1826 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs` | 1789 | **free** | - |
| `crates/boyko_render/src/shadow_atlas.rs` | 1784 | **free** | - |
| `crates/aether_lang/src/parse.rs` | 1719 | **free** | - |
| `crates/boyko_physics/src/solver/simd.rs` | 1654 | blocked-by-lane(A4/A5 physics (D:/wt/joltab)) | that lane lands |
| `crates/boyko_physics/src/systems.rs` | 1633 | blocked-by-lane(A4/A5 physics (D:/wt/joltab)) | that lane lands |
| `crates/boyko_log/src/codes.rs` | 1630 | blocked-by-owner | owner commits `feat/multi-paradigm-render` |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | 1617 | blocked-by-unmerged(fix/ke13-ke14) | `fix/ke13-ke14` merges |
| `crates/boyko_render/src/gpu_column.rs` | 1605 | blocked-by-unmerged(feat/ui-advanced) | `feat/ui-advanced` merges |
| `crates/boyko_ecs/src/ecs/core/profiling/store.rs` | 1595 | **free** | - |
| `crates/boyko_render/src/light_system.rs` | 1592 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` | 1589 | **free** | - |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | 1554 | blocked-by-owner + blocked-by-unmerged(fix/ke13-ke14) | owner commits `feat/multi-paradigm-render`; `fix/ke13-ke14` merges |
| `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | 1548 | **free** | - |
| `crates/boyko_render/src/upload.rs` | 1517 | blocked-by-unmerged(fix/ddgi-host-hook) | `fix/ddgi-host-hook` merges |
| `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | 10434 | **free** | - |
| `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs` | 8342 | **free** | - |
| `crates/boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | 6672 | **free** | - |
| `crates/boyko_ecs/benches/cull_diagnostic.rs` | 3316 | **free** | - |

### 2.5 The declaration site — a fourth, quieter blocker

A split does not have to touch the parent module file **if** it uses the route the tree already
uses (section 4): the oversized file stays put and becomes the parent of a new directory named
after it, so only that file gains `mod` lines. Where the file is already a `mod.rs`, the new
children are its siblings and again only that `mod.rs` is edited. The table below records, per
census file, who declares it and whether that declaring file is itself contested — relevant the
moment a split wants to add a *sibling* instead of a child (the `ecs_master.rs` case in
section 5).

| file | declared by | declaring file status | already owns a dir | lowest-risk route |
|------|-------------|----------------------|--------------------|-------------------|
| `crates/boyko_rhi_vulkan/src/present/targets.rs` | `crates/boyko_rhi_vulkan/src/present/mod.rs` | clean | no | child dir `targets/`, `mod` lines in this file |
| `crates/boyko_app/src/gpu_scene/mod.rs` | `crates/boyko_app/src/lib.rs` | clean | no | siblings in own dir, `mod` lines in this mod.rs |
| `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | `crates/boyko_rhi_vulkan/src/present/mod.rs` | clean | no | child dir `graph_bridge/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | `crates/boyko_rhi_vulkan/src/present/passes/mod.rs` | clean | no | child dir `vb/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/compute.rs` | `crates/boyko_rhi_vulkan/src/lib.rs` | clean | yes | child dir `compute/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/goldens.rs` | `crates/boyko_rhi_vulkan/src/lib.rs` | clean | no | child dir `goldens/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | `crates/boyko_rhi_vulkan/src/present/mod.rs` | clean | no | child dir `scene_types/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/device.rs` | `crates/boyko_rhi_vulkan/src/lib.rs` | clean | no | child dir `device/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | `crates/boyko_ecs/src/ecs/memory/mod.rs` | clean | no | child dir `component_pool/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/ffi.rs` | `crates/boyko_rhi_vulkan/src/lib.rs` | clean | no | child dir `ffi/`, `mod` lines in this file |
| `crates/boyko_physics/src/resources.rs` | `crates/boyko_physics/src/lib.rs` | OWNER-DIRTY | no | child dir `resources/`, `mod` lines in this file |
| `crates/boyko_app/src/runner.rs` | `crates/boyko_app/src/lib.rs` | clean | no | child dir `runner/`, `mod` lines in this file |
| `crates/aether_lang/src/expand.rs` | `crates/aether_lang/src/lib.rs` | clean | no | child dir `expand/`, `mod` lines in this file |
| `crates/boyko_physics/src/solver/colored.rs` | `crates/boyko_physics/src/solver/mod.rs` | clean | no | child dir `colored/`, `mod` lines in this file |
| `crates/boyko_render/src/render_path_config.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `render_path_config/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | `crates/boyko_rhi_vulkan/src/present/passes/mod.rs` | clean | no | child dir `gbuffer/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | unmerged fix/ke13-ke14 | no | child dir `filter/`, `mod` lines in this file |
| `crates/boyko_physics/src/solver/colored_tests.rs` | `crates/boyko_physics/src/solver/colored.rs` | clean | no | child dir `colored_tests/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/compute/tests.rs` | `crates/boyko_rhi_vulkan/src/compute.rs` | clean | no | child dir `tests/`, `mod` lines in this file |
| `crates/boyko_shaderdsl/src/emit/shaders.rs` | `crates/boyko_shaderdsl/src/emit/mod.rs` | unmerged feat/ui-advanced | no | child dir `shaders/`, `mod` lines in this file |
| `crates/boyko_shaderdsl/src/emit/mod.rs` | `crates/boyko_shaderdsl/src/lib.rs` | unmerged feat/golden-edsl-p0; unmerged feat/ui-advanced | no | siblings in own dir, `mod` lines in this mod.rs |
| `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` | `crates/boyko_ecs/src/ecs/core/commands/mod.rs` | clean | no | child dir `migration_helpers/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/asset/assets.rs` | `crates/boyko_ecs/src/ecs/core/asset/mod.rs` | clean | no | child dir `assets/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | `crates/boyko_rhi_vulkan/src/rhi_impl/mod.rs` | clean | no | child dir `device/`, `mod` lines in this file |
| `crates/boyko_render/src/light.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `light/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | `crates/boyko_ecs/src/ecs/core/archetype/mod.rs` | clean | no | child dir `archetype/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | `crates/boyko_ecs/src/ecs/core/schedule/mod.rs` | clean | no | child dir `schedule/`, `mod` lines in this file |
| `crates/boyko_render/src/csm_config.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `csm_config/`, `mod` lines in this file |
| `crates/boyko_threadpool/src/scope.rs` | `crates/boyko_threadpool/src/lib.rs` | clean | no | child dir `scope/`, `mod` lines in this file |
| `crates/boyko_ui/src/layout.rs` | `crates/boyko_ui/src/lib.rs` | unmerged feat/ui-advanced | no | child dir `layout/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` | `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | unmerged fix/ke13-ke14 | no | child dir `state/`, `mod` lines in this file |
| `crates/boyko_render/src/hzb.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `hzb/`, `mod` lines in this file |
| `crates/boyko_threadpool/src/block.rs` | `crates/boyko_threadpool/src/lib.rs` | clean | no | child dir `block/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/profiling/tests.rs` | `crates/boyko_ecs/src/ecs/core/profiling/mod.rs` | clean | no | child dir `tests/`, `mod` lines in this file |
| `crates/boyko_render/src/mesh_draw.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `mesh_draw/`, `mod` lines in this file |
| `crates/boyko_macros/src/component.rs` | `crates/boyko_macros/src/lib.rs` | unmerged feat/reflection | no | child dir `component/`, `mod` lines in this file |
| `crates/boyko_shaderdsl/src/cf.rs` | `crates/boyko_shaderdsl/src/lib.rs` | unmerged feat/golden-edsl-p0; unmerged feat/ui-advanced | no | child dir `cf/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | `crates/boyko_ecs/src/ecs/core/schedule/mod.rs` | clean | no | child dir `schedule_builder/`, `mod` lines in this file |
| `crates/boyko_app/src/gpu_scene/particle.rs` | `crates/boyko_app/src/gpu_scene/mod.rs` | unmerged fix/ddgi-host-hook | no | child dir `particle/`, `mod` lines in this file |
| `crates/boyko_shaderdsl/src/bin/emit_particles.rs` | `(target root)` | clean | no | child dir `emit_particles/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | `crates/boyko_ecs/src/ecs/core/ecs_master/mod.rs` | unmerged feat/reflection | no | child dir `ecs_master/`, `mod` lines in this file |
| `crates/boyko_sdf_math/src/brick/tests.rs` | `crates/boyko_sdf_math/src/brick.rs` | clean | no | child dir `tests/`, `mod` lines in this file |
| `crates/boyko_sdf_math/src/brick.rs` | `crates/boyko_sdf_math/src/lib.rs` | clean | yes | child dir `brick/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | unmerged fix/ke13-ke14 | no | child dir `iter/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs` | `crates/boyko_ecs/src/ecs/core/component/mod.rs` | clean | no | siblings in own dir, `mod` lines in this mod.rs |
| `crates/boyko_render/src/shadow_atlas.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `shadow_atlas/`, `mod` lines in this file |
| `crates/aether_lang/src/parse.rs` | `crates/aether_lang/src/lib.rs` | clean | no | child dir `parse/`, `mod` lines in this file |
| `crates/boyko_physics/src/solver/simd.rs` | `crates/boyko_physics/src/solver/mod.rs` | clean | no | child dir `simd/`, `mod` lines in this file |
| `crates/boyko_physics/src/systems.rs` | `crates/boyko_physics/src/lib.rs` | OWNER-DIRTY | no | child dir `systems/`, `mod` lines in this file |
| `crates/boyko_log/src/codes.rs` | `crates/boyko_log/src/lib.rs` | clean | no | child dir `codes/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | `crates/boyko_ecs/src/ecs/core/archetype/mod.rs` | clean | no | child dir `archetype_master/`, `mod` lines in this file |
| `crates/boyko_render/src/gpu_column.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `gpu_column/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/profiling/store.rs` | `crates/boyko_ecs/src/ecs/core/profiling/mod.rs` | clean | no | child dir `store/`, `mod` lines in this file |
| `crates/boyko_render/src/light_system.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `light_system/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` | `crates/boyko_ecs/src/ecs/core/commands/mod.rs` | clean | no | child dir `command_queue/`, `mod` lines in this file |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | `crates/boyko_ecs/src/ecs/core/iters/query/mod.rs` | unmerged fix/ke13-ke14 | no | child dir `query/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | `crates/boyko_rhi_vulkan/src/present/mod.rs` | clean | no | child dir `gpu_zone/`, `mod` lines in this file |
| `crates/boyko_render/src/upload.rs` | `crates/boyko_render/src/lib.rs` | unmerged feat/ui-advanced; unmerged fix/asset-validate-prereqs; unmerged fix/ddgi-host-hook | no | child dir `upload/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | `(target root)` | clean | no | child dir `window_present_gbuffer/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs` | `(target root)` | clean | no | child dir `sdf_gbuffer_hybrid/`, `mod` lines in this file |
| `crates/boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | `(target root)` | clean | no | child dir `vb_barrier_stream_baseline/`, `mod` lines in this file |
| `crates/boyko_ecs/benches/cull_diagnostic.rs` | `(target root)` | clean | no | child dir `cull_diagnostic/`, `mod` lines in this file |

Two declaring files are contested by many lanes at once: `crates/boyko_render/src/lib.rs` (78
child modules, one giant `pub use` re-export block; changed by `feat/ui-advanced`,
`fix/asset-validate-prereqs` and `fix/ddgi-host-hook`) and `crates/boyko_physics/src/lib.rs`
(owner-dirty). Neither needs editing under the child-directory route.

---

## 3. Line-sensitive and path-sensitive artifacts

### 3.0 Which of these can actually fail a build

Three tiers, and only the first two are mechanical:

1. **Mechanically gated, line-sensitive.** The four docs in `tests/internal_docs_anchors.rs`
   `GATED_DOCS` = `docs/{FEATURE_MAP.md, SYSTEMS.md, ARCHITECTURE.md,
   MESHLET-VIRTUAL-GEOMETRY-PLAN.md}`. Every `file.rs:N` and every bare `(N)` under a sticky
   `**File:**` header must still land on a definition — and, where the line's backticked symbol
   pairs one-to-one with its anchor, on *that* symbol. **195 such anchors point into census files
   (42 of them `~`-waived, which check only "line N is inside the file").** A split moves lines,
   so every one of these must be re-derived in the same change.
2. **Mechanically gated, path-sensitive (not line-sensitive).** `scripts/check_hotpath_exceptions.py`
   (wired at `.github/workflows/ci.yml:60`) keys on `(file, count of #[allow(clippy::disallowed_types)])`,
   deliberately *not* on line numbers; `tests/production_reachability_census.rs` carries
   `defined_in: "crates/..."` literals; `tests/gpu_blocking_reader_census.rs` is a path allow-list;
   `crates/boyko_log/tests/code_registry.rs` names `#[path]`-included test files;
   `goldens/PINS.toml` keys on `(crate, test_binary, test_name)`. Moving an item out of a file
   changes the first three even though no line number is involved.
3. **Ungated prose.** Anchors in the other 119 `.md` files (3,207 tree-wide, **1,018 of them into
   census files**) and 116 `file.rs:N` citations inside `.rs`/`.toml`/`.ps1`/`.hlsl` comments.
   Nothing checks these; per the standing rule they drift and that is *recorded*, not repaired.
   They are listed so the record can be made.

### 3.1 Per-file artifact table

`gated anchors (waived)` — anchors from the four `GATED_DOCS` that resolve to this file, and how
many carry `~`. `ungated anchors` — same resolution over every other `.md` in the tree.
`code line-pins` — `<file>.rs:N` citations inside non-markdown files, matched on the file's
minimal unique path suffix with an identifier boundary (so `enable_store.rs:299` is not counted
against `store.rs`). `cfg(feature) sites` counts `feature = "…"` occurrences, i.e. how much of
the file is behind a non-default feature.

| file | gated anchors (waived) | ungated anchors | code line-pins | `macro_rules!` | file-top `#![..]` | `cfg(feature)` sites | `#[path]` | clippy-allow sites | path-keyed gate |
|------|----:|----:|----:|---|---|----:|---|----:|---|
| `crates/boyko_rhi_vulkan/src/present/targets.rs` | 11 (11) | 61 | 0 | - | - | 157 | - | 0 | - |
| `crates/boyko_app/src/gpu_scene/mod.rs` | 0 (0) | 72 | 1 | - | - | 112 | - | 0 | gpu-blocking-reader |
| `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | 0 (0) | 14 | 2 | - | - | 74 | - | 0 | - |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | 0 (0) | 95 | 2 | - | - | 19 | - | 0 | vb_sv0_produce_run_timing |
| `crates/boyko_rhi_vulkan/src/compute.rs` | 0 (0) | 22 | 0 | embed_spirv | - | 49 | - | 0 | - |
| `crates/boyko_rhi_vulkan/src/goldens.rs` | 0 (0) | 27 | 1 | - | - | 3 | - | 0 | - |
| `crates/boyko_rhi_vulkan/src/present/scene_types.rs` | 0 (0) | 21 | 0 | - | - | 53 | - | 0 | - |
| `crates/boyko_rhi_vulkan/src/device.rs` | 0 (0) | 81 | 1 | fail | - | 31 | - | 6 | gpu-blocking-reader + hot-path registry |
| `crates/boyko_ecs/src/ecs/memory/component_pool.rs` | 11 (1) | 17 | 9 | - | - | 0 | - | 0 | - |
| `crates/boyko_rhi_vulkan/src/ffi.rs` | 0 (0) | 2 | 0 | dispatchable_handle, non_dispatchable_handle | L27 allow(non_snake_case); L28 allow(non_camel_case_types) | 14 | - | 0 | gpu-blocking-reader |
| `crates/boyko_physics/src/resources.rs` | 0 (0) | 35 | 0 | - | - | 1 | resources_tests.rs | 0 | - |
| `crates/boyko_app/src/runner.rs` | 2 (2) | 148 | 5 | - | - | 23 | - | 0 | - |
| `crates/aether_lang/src/expand.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_physics/src/solver/colored.rs` | 0 (0) | 19 | 1 | - | - | 28 | colored_tests.rs | 0 | - |
| `crates/boyko_render/src/render_path_config.rs` | 6 (6) | 38 | 2 | - | - | 3 | - | 0 | reachability |
| `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | 0 (0) | 9 | 0 | - | - | 35 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | 15 (0) | 2 | 0 | impl_query_filter_tuple_and, impl_or_filter_tuple, impl_query_filter_tuple_and_too_large, impl_or_filter_tuple_too_large, impl_or_composable_tuple, impl_archetypal_filter_tuple | - | 0 | - | 0 | - |
| `crates/boyko_physics/src/solver/colored_tests.rs` | 0 (0) | 0 | 0 | - | - | 20 | - | 0 | log code-registry |
| `crates/boyko_rhi_vulkan/src/compute/tests.rs` | 0 (0) | 10 | 0 | - | L4 allow(clippy::disallowed_types) | 0 | - | 1 | - |
| `crates/boyko_shaderdsl/src/emit/shaders.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_shaderdsl/src/emit/mod.rs` | 0 (0) | 0 | 0 | - | L29 allow(clippy::disallowed_types) | 3 | - | 1 | hot-path registry (blanket) |
| `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` | 20 (0) | 8 | 4 | src, tgt | L21 allow(dead_code) | 0 | - | 1 | - |
| `crates/boyko_ecs/src/ecs/core/asset/assets.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 1 | - |
| `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | 0 (0) | 7 | 2 | - | - | 11 | - | 0 | gpu-blocking-reader |
| `crates/boyko_render/src/light.rs` | 0 (0) | 77 | 6 | - | - | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` | 16 (5) | 11 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` | 2 (0) | 20 | 28 | - | - | 0 | - | 1 | - |
| `crates/boyko_render/src/csm_config.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 1 | - |
| `crates/boyko_threadpool/src/scope.rs` | 0 (0) | 9 | 2 | - | - | 0 | - | 0 | - |
| `crates/boyko_ui/src/layout.rs` | 0 (0) | 9 | 1 | - | - | 0 | - | 1 | reachability |
| `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` | 8 (2) | 2 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_render/src/hzb.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_threadpool/src/block.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/profiling/tests.rs` | 0 (0) | 0 | 0 | - | - | 6 | - | 0 | reachability |
| `crates/boyko_render/src/mesh_draw.rs` | 3 (0) | 7 | 6 | - | - | 10 | - | 0 | - |
| `crates/boyko_macros/src/component.rs` | 10 (10) | 40 | 1 | - | - | 0 | - | 0 | - |
| `crates/boyko_shaderdsl/src/cf.rs` | 0 (0) | 0 | 0 | - | - | 6 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` | 13 (0) | 28 | 0 | - | L34 allow(clippy::disallowed_types) | 0 | - | 1 | hot-path registry (blanket) |
| `crates/boyko_app/src/gpu_scene/particle.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_shaderdsl/src/bin/emit_particles.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | 11 (0) | 27 | 16 | - | - | 0 | - | 0 | - |
| `crates/boyko_sdf_math/src/brick/tests.rs` | 0 (0) | 0 | 0 | - | - | 0 | - | 0 | log code-registry |
| `crates/boyko_sdf_math/src/brick.rs` | 0 (0) | 2 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | 6 (0) | 3 | 4 | - | L57 allow(dead_code) | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs` | 30 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_render/src/shadow_atlas.rs` | 0 (0) | 1 | 1 | - | - | 0 | - | 0 | - |
| `crates/aether_lang/src/parse.rs` | 0 (0) | 8 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_physics/src/solver/simd.rs` | 0 (0) | 0 | 0 | - | - | 28 | - | 0 | - |
| `crates/boyko_physics/src/systems.rs` | 1 (1) | 5 | 3 | - | - | 5 | - | 0 | - |
| `crates/boyko_log/src/codes.rs` | 2 (0) | 8 | 0 | code_eq, code_newtype, codes, code_class_new, code_class_ty, declare_codes!(exported), codes_tidy!(exported), code_class_ty_pub!(exported) | - | 1 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` | 17 (4) | 13 | 1 | - | - | 0 | - | 0 | - |
| `crates/boyko_render/src/gpu_column.rs` | 0 (0) | 5 | 0 | - | - | 10 | - | 0 | reachability |
| `crates/boyko_ecs/src/ecs/core/profiling/store.rs` | 0 (0) | 0 | 4 | - | - | 22 | - | 2 | - |
| `crates/boyko_render/src/light_system.rs` | 0 (0) | 14 | 1 | - | - | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` | 0 (0) | 9 | 7 | - | L17 allow(dead_code) | 0 | - | 0 | - |
| `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` | 11 (0) | 0 | 0 | - | - | 0 | - | 0 | - |
| `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | 0 (0) | 4 | 0 | - | - | 0 | - | 0 | gpu-blocking-reader |
| `crates/boyko_render/src/upload.rs` | 0 (0) | 0 | 0 | - | - | 9 | - | 0 | reachability |
| `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | 0 (0) | 6 | 0 | sweep, capture_or_skip, present_interp, capture_at | L54 cfg(windows); L62 allow(clippy::chunks_exact_to_as_c | 96 | - | 0 | - |
| `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs` | 0 (0) | 22 | 5 | - | L39 allow(clippy::chunks_exact_to_as_c | 0 | - | 0 | - |
| `crates/boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | 0 (0) | 0 | 0 | pass | - | 0 | - | 0 | - |
| `crates/boyko_ecs/benches/cull_diagnostic.rs` | 0 (0) | 0 | 0 | spawn_bundle | - | 1 | - | 0 | - |

### 3.2 Where the gated anchors concentrate

195 gated anchors reach census files, and they are not spread evenly — **12 files carry 165 of
them, and 11 of those 12 are `boyko_ecs`**:

```
 30 (0 waived)  component_registry/mod.rs   FEATURE_MAP 13 + SYSTEMS 14 + ARCHITECTURE 3
 20 (0)         commands/migration_helpers.rs           FM  6 + SYS 14
 17 (4)         archetype/archetype_master.rs           FM  2 + SYS 15
 16 (5)         archetype/archetype.rs                  FM  3 + SYS 13
 15 (0)         iters/query/filter.rs                   FM  8 + SYS  7
 13 (0)         schedule/schedule_builder.rs            FM  6 + SYS  7
 11 (11)        rhi_vulkan present/targets.rs           MESHLET 11   (every one waived)
 11 (1)         memory/component_pool.rs                FM  2 + SYS  9
 11 (0)         iters/query/query.rs                    FM  5 + SYS  6
 11 (0)         ecs_master/ecs_master.rs                FM  5 + SYS  6
 10 (10)        boyko_macros/src/component.rs           FM  4 + SYS  6   (every one waived)
  8 (2)         iters/query/state.rs                    FM  1 + SYS  7
```

Two asymmetries matter for sequencing. **The ECS kernel files carry un-waived, identity-asserted
anchors** — `SYSTEMS.md` alone puts 14-15 of them on `migration_helpers.rs` and
`archetype_master.rs` — so a split there must re-derive every citation *and* keep the cited symbol
on the cited line. **The render-side files are waived** (`targets.rs` 11/11, `component.rs` 10/10,
`render_path_config.rs` 6/6), because the meshlet plan's anchors are mostly `~`; the gate then
checks only that line N is inside the file, so a split there is far cheaper to repair — but note
the gate has a per-document cap on *unnecessary* waivers (`MESHLET…: 7 waived anchors on
definition-shaped lines (cap 7)`), so re-pointing a waived anchor at a definition line can red a
different assertion.

### 3.3 Code line-pins (ungated, but the loudest prose)

116 `<file>.rs:N` citations sit in non-markdown files. Concentrations:

```
 28  schedule/schedule.rs        (ke16_in_system_depth_balance.rs x10, ke17_apply_window.rs x4,
                                  boyko_render/src/light_system.rs x3, asset_refcount.rs x2, ...)
 16  ecs_master/ecs_master.rs    (boyko_ui reload/reconcile.rs x3, boyko_ui layout.rs x2, ...)
  9  memory/component_pool.rs    (dense/views.rs x3, scratch/views.rs x3, comparison_v2.rs x2)
  7  commands/command_queue.rs   (miri_phase19.rs x5)
  6  render/mesh_draw.rs         (vb_batch_cull.comp.hlsl x2, passes/vb.rs x2)
  6  render/light.rs             (rhi_vulkan goldens.rs x3, ke17_apply_window.rs x2)
  5  app/runner.rs   5  tests/sdf_gbuffer_hybrid.rs   4  profiling/store.rs   4  query/iter.rs
```

These are doc-comment citations inside test and source files, not assertions: nothing fails when
they rot. Two observations anyway. (1) `schedule.rs`'s 28 are the densest cross-file citation set
in the repo and they come from the KE16/KE17 campaign — a split of `schedule.rs` silently
invalidates a campaign's own record, which is the failure mode this repository catalogues.
(2) Three of them are *cross-crate* (`boyko_render` citing `schedule.rs`, `boyko_ui` citing
`ecs_master.rs`), so a split's blast radius in prose crosses crate boundaries even when its code
does not.

### 3.4 Path-keyed gates — the ones that break without any line moving

| gate | keyed on | census files it names | what a split does to it |
|---|---|---|---|
| `scripts/check_hotpath_exceptions.py` (CI `ci.yml:60`) + `docs/HOT-PATH-EXCEPTIONS.md` | `(file, count of #[allow(clippy::disallowed_types)])`, plus a separate **blanket** list for file-top `#![allow]` | item-level: `rhi_vulkan/src/device.rs` (6), `profiling/store.rs` (2), and one each in `compute/tests.rs`, `emit/mod.rs`, `migration_helpers.rs`, `asset/assets.rs`, `schedule/schedule.rs`, `csm_config.rs`, `ui/layout.rs`, `schedule_builder.rs` | moving an allowed item into a child file moves its `#[allow]` with it, so the per-file count changes on **both** files and the registry must be re-pathed in the same change. The script's own doc explains why it is a count and not a line: "line numbers churn on every edit above the site". |
| `tests/production_reachability_census.rs` | `defined_in: "crates/…"` literals (18 crate paths) | `render_path_config.rs`, `render/gpu_column.rs`, `render/upload.rs`, `ui/layout.rs`, `ecs/core/profiling/tests.rs` | a symbol that moves out of the named file makes the census's own claim false |
| `tests/gpu_blocking_reader_census.rs` | a path allow-list (8 paths) | `rhi_vulkan/src/device.rs`, `rhi_impl/device.rs`, `ffi.rs`, `present/gpu_zone.rs`, `app/gpu_scene/mod.rs` | a blocking-read call that lands in a new child file is *outside* the allow-list |
| `crates/boyko_log/tests/code_registry.rs` + `tests/walker/mod.rs` | file paths of the `#[path]`-included in-`src` test files | `physics/solver/colored_tests.rs`, `sdf_math/src/brick/tests.rs` | renaming/moving those two files edits the registry |
| `goldens/PINS.toml` (64 pins) + `scripts/golden.ps1` | `(crate, test_binary, test_name)` — **not** a source path | none directly | unaffected by a split *by construction*, which is exactly why it is the campaign's behaviour gate. Its own header says so: "Any logical no-op (**god-file split**, `embed_spirv!`, gated-OFF path, host-Rust render diff) MUST keep this byte-identical (Tier-0)." |
| `tests/internal_docs_anchors.rs` path clause | every `](…)` and bare `crates/…` mention must exist on disk (515 in FEATURE_MAP, 503 in SYSTEMS) | all of them | **safe under the campaign's own rule**: the original file survives as the parent module, so no cited path disappears |

### 3.5 Macros, inner attributes, `#[path]`, `include_*!`, features

**`macro_rules!` (textual scoping is the hazard).** 22 definitions across 9 census files. Only
three are `#[macro_export]` — `declare_codes` (`boyko_log/src/codes.rs:1162`), `codes_tidy`
(`:1273`), `code_class_ty_pub` (`:1348`) — and those live at the crate root regardless of which
file defines them, so a move inside the crate cannot break them. Every other one is **file-local
and used below its own definition**, which is what a split must preserve:

```
compute.rs:84    embed_spirv           103 in-file uses  (the 38 SPIR-V embeds)
ffi.rs:266/299   dispatchable_handle / non_dispatchable_handle   4 + 17 in-file uses
filter.rs        6 tuple-impl macros (impl_query_filter_tuple_and:1547, impl_or_filter_tuple:1822,
                 …_too_large:2133/2211, impl_or_composable_tuple:2513, impl_archetypal_filter_tuple:2611)
                 ~11-12 in-file uses each
device.rs:1043   fail                  8 in-file uses
codes.rs         code_eq / code_newtype / codes / code_class_new / code_class_ty (file-local)
migration_helpers.rs:531/538  src / tgt (inside the test module)
```

`crates/boyko_rhi_vulkan/src/lib.rs` has **no `#[macro_use]`**, and the apparent external users of
`embed_spirv!` / `non_dispatchable_handle!` / `codes!` are prose mentions in doc comments
(`accel_ffi.rs:41` even says "kept local"), verified by reading each site. So the rule for these
files is concrete: a child module that uses one of these macros needs either the macro moved with
it, or a `pub(crate) use <macro>;` line — the one text addition the campaign's rules allow.

**File-top inner attributes (`#![…]` above all items).** Only eight census files have any:

```
ffi.rs:27,28                 #![allow(non_snake_case)]  #![allow(non_camel_case_types)]
emit/mod.rs:29               #![allow(clippy::disallowed_types)]   <- registered BLANKET exemption
schedule_builder.rs:34       #![allow(clippy::disallowed_types)]   <- registered BLANKET exemption
migration_helpers.rs:21      #![allow(dead_code)]
iter.rs:57                   #![allow(dead_code)]
command_queue.rs:17          #![allow(dead_code)]
window_present_gbuffer.rs:54 #![cfg(windows)]     :62 #![allow(clippy::chunks_exact_to_as_chunks)]
sdf_gbuffer_hybrid.rs:39     #![allow(clippy::chunks_exact_to_as_chunks)]
```

Every other `#![…]` found in a census file sits deep inside it (`component_pool.rs:2928/3128`,
`assets.rs:1715/1836/2593`, `compute/tests.rs:1230`, `device.rs:3906`, `schedule.rs:1600`,
`layout.rs:1744`, `csm_config.rs:1336`, `assets.rs:1123`, `migration_helpers.rs:2197`) and belongs
to a `proptest! { … }` body or a `#[cfg(test)] mod` — it moves with its block. Lint levels are
inherited by child modules, so a child directory under `ffi.rs` still gets its two `allow`s; the
campaign's ban on adding a new file-top attribute is therefore satisfiable **only** if no moved
item needs a lint level its new parent does not already carry.

**`#[path]`.** Exactly two in the whole census, both in the physics lane and both the same
idiom — `#[cfg(test)] #[path = "resources_tests.rs"] mod tests;` in `resources.rs` and
`#[path = "colored_tests.rs"]` in `solver/colored.rs`. They pull a *sibling* file (not a child
directory) and they are named by `boyko_log`'s code registry (3.4). Both are lane-blocked anyway.

**`include_*!` — checked because it is the classic split-breaker, and it is clean here.** Every
embed in the census resolves against `CARGO_MANIFEST_DIR`, not against the containing file:

```
compute.rs:87   static $name: SpirvBlob<{ include_bytes!($path).len() }> = SpirvBlob(*include_bytes!($path));
compute.rs:92   embed_spirv! { …, concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/write_pattern.comp.spv") }
window_present_gbuffer.rs:176/184/191/198, sdf_gbuffer_hybrid.rs:438/447 — same concat! shape
```

So moving the 38 embeds out of `compute.rs` does **not** break a path. The constraint on
`compute.rs` is the macro's textual scope (above), not the includes.

**`cfg(feature)` density.** How much of a file is conditionally compiled decides how much of a
split has to be re-verified under a second feature arm:

```
targets.rs 157   gpu_scene/mod.rs 112   graph_bridge.rs 74   scene_types.rs 53   compute.rs 49
gbuffer.rs 35    device.rs 31   colored.rs 28   solver/simd.rs 28   runner.rs 23   store.rs 22
vb.rs 19   ffi.rs 14   gpu_column.rs 10   upload.rs 9
```

Features involved: `hwrt` (dominant, and 3 of the 4 feature-gated ignored tests need it),
`profiling-census`, `profiling-analysis`, `spec_constant_smoke`, `emit`, `nightly`, `avx2`, `fma`,
`bench-alloc`, `test-probe`. A child module declared behind `#[cfg(feature = "hwrt")]` compiles in
a different configuration from its parent — items and their `#[cfg]`s move verbatim, but the
*declaration* the split adds is itself a new `#[cfg]` decision, and `cargo check --workspace`
without the feature will not compile the moved code at all.

**Doctests.** Almost absent from the census: one `compile_fail` doctest and one plain one in
`filter.rs`; `state.rs`, `ecs_master.rs` and `codes.rs` use ```` ```ignore ````; the rest of the
fenced blocks in doc comments are ```` ```text ````. No census file's doctest names a module path
that a split would move.

### 3.6 Re-export chains

Every crate but `boyko_ecs`/`boyko_app`/`boyko_input` re-exports from its `lib.rs` rather than a
`prelude` module, and `crates/boyko_render/src/lib.rs` is the extreme case — 78 child modules and
a `pub use <module>::{…}` block for nearly all of them (`light.rs:575`, `hzb.rs:518`,
`csm_config.rs:457`, `render_path_config.rs:490`, `mesh_draw.rs:566`, `shadow_atlas.rs:608`,
`gpu_column.rs:573`, `upload.rs:647/661/664`, …). `crates/boyko_ecs/src/prelude.rs` names census
modules by full path (`…::ecs_master::ecs_master::EcsMaster` at :25,
`…::component_registry::TagId` at :35, `…::iters::query::{Query, …}`, the `schedule` block at
:52ff). All of these are **compiler-enforced**: if a split moves an item without re-exporting it
from the same path, the build fails immediately and loudly. They are a cost, not a risk.

### 3.7 Baseline gate state (measured on this tree, before any split)

| gate | command | result at `d552be05` |
|---|---|---|
| internal doc anchors | `cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture` | **green** — 5 passed; 742 anchors, 0 stale; 1,205 path mentions, 0 dead |
| hot-path exceptions | `python scripts/check_hotpath_exceptions.py` | **RED, pre-existing** — exit 1: `crates/boyko_ecs/src/ecs/core/entity/entity_reservoir.rs:405: unregistered crate/module-level #![allow]`. 34 exceptions across 12 files. Not a census file, not caused by this campaign — **record it now so a split is not blamed for it later.** |

⚠️ That second row is exactly the trap the repository's own memory calls "a gate repaired into a
wrong red": if the campaign runs the CI gate after its first split and sees red, the first
hypothesis will be the split. It is not. The red is `entity_reservoir.rs:405` and it predates the
branch.

### 3.8 trybuild `.stderr` goldens cite the *library* file a moved item lives in

This is the artifact class that the previous campaign actually tripped over, and it is not in the
task's list, so it is recorded here with its evidence. `git log --all --grep="god-file"` finds two
prior split commits; the earlier one had to re-bless three committed `.stderr` files:

```
e6ac6a48 refactor(ecs): split the query/data god-file into a directory module
  crates/boyko_ecs/src/ecs/core/iters/query/data.rs     | 2595 +------------
  .../query/data/{anyof,mut_,option,read,ref_,tuple_impls,write}.rs   (7 new files)
  .../option_anyof_compile_fail/anyof_arm_option_rejected.stderr      |  4 +-
  .../option_anyof_compile_fail/anyof_arm_unit_rejected.stderr        |  4 +-
  .../option_anyof_compile_fail/anyof_empty_rejected.stderr           |  2 +-

  -  --> src/ecs/core/iters/query/data.rs
  +  --> src/ecs/core/iters/query/data/anyof.rs
```

rustc's `help: the following other types implement trait …` and `::: <path>` notes name the
**source file** of the cited impl, trybuild records that verbatim, and so a pure move re-paths the
golden. (Path only — no line number appears in these notes — so this is path-sensitive, not
line-sensitive.) Its commit message states the discipline: *"Three trybuild .stderr goldens are
refreshed path-only (the diagnostic cites the source file that moved)."*

Scanning all **138** committed `.stderr` files, seven census files are cited:

| census file | citations | fixtures |
|---|---:|---|
| `boyko_ecs/.../iters/query/query.rs` | 16 | 16 (`compile_fail_chunk/*`, `enable_filter_compile_fail/*`, `option_anyof_compile_fail/*`) |
| `boyko_threadpool/src/block.rs` | 16 | 4 (`docs/threadpool/receipts/tb-neg-m2w-{0,1,7,15}.stderr`) |
| `boyko_log/src/codes.rs` | 15 | 3 (`compile_fail_codes/*`) |
| `boyko_ecs/.../ecs_master/ecs_master.rs` | 8 | 4 (`query_change_detection_compile_fail/*`) |
| `boyko_ecs/.../iters/query/filter.rs` | 4 | 4 (`enable_filter_compile_fail/*`) |
| `boyko_ecs/.../iters/query/state.rs` | 4 | 2 (`enable_filter_compile_fail/*`) |
| `boyko_threadpool/src/scope.rs` | 4 | 4 (`docs/threadpool/receipts/*`) |

⚠️ The `boyko_threadpool` ones are **not** trybuild fixtures — they are committed receipts under
`docs/threadpool/receipts/`, i.e. a *doc* that quotes a compiler diagnostic. Nothing re-runs them,
so they will rot silently rather than red. Both files are lane-blocked anyway.

---

## 4. The module-file convention

Counted over every lib/bin module node (715 files):

| style | count |
|---|---:|
| `foo/mod.rs` (a directory module whose parent file is `mod.rs`) | **69** |
| `foo.rs` + `foo/` (the 2018 style: a non-`mod.rs` file owning its own directory) | **4** |
| leaf `foo.rs` with no directory | 642 |

By crate, the `mod.rs` directories are overwhelmingly `boyko_ecs` (37), then `boyko_ui` (5),
`boyko_rhi_vulkan` (4), `boyko_utils` (4), `boyko_demo` (4), `boyko_input` (3), `boyko_physics` (3),
`boyko_app` (2), `boyko_render` (2), one each in `boyko_shaderdsl`, `boyko_log`, `boyko_macros`,
`boyko_fontbake`, `boyko_threadpool`.

**But the 4 and the 69 are different things, and the 4 are the ones this campaign inherits.**
All four `foo.rs` + `foo/` cases are the *product of a split*, where the god-file survived and
grew a directory beneath it:

```
crates/boyko_ecs/src/ecs/core/iters/query/data.rs  (980)  + data/{anyof,mut_,option,read,ref_,
                                                              tuple_impls,write}.rs  (7 children,
                                                              `mod` lines at data.rs:437-443)
crates/boyko_rhi_vulkan/src/compute.rs            (6127) + compute/tests.rs  (cfg(test), decl on
                                                              the file's LAST line)
crates/boyko_sdf_math/src/brick.rs                (1830) + brick/tests.rs    (same idiom)
crates/boyko_diag/src/profiling_abi.rs             (874) + profiling_abi/dyn_registry.rs
```

The 69 `mod.rs` directories are, with two exceptions, modules that were *born* as directories:
`ecs/core/` (22 children, `mod.rs` is 23 lines), `iters/query/` (18, 84 lines), `commands/` (14,
70), `system/` (14, 39), `ecs_master/` (13, 20), `text/` (13, 51), `schedule/` (12, 41),
`present/passes/` (12, 24) — a thin declaration-and-re-export hub with responsibility-named
siblings.

The two exceptions are the other prior split route, and they are worth naming because the campaign
must choose between them:

```
3ac3047d refactor(app): split the gpu_scene god-file into a directory module
   R070  crates/boyko_app/src/gpu_scene.rs -> crates/boyko_app/src/gpu_scene/mod.rs
         + gpu_scene/{csm,tlas,interp}.rs
```

i.e. the god-file was **renamed** into `mod.rs`. `component_registry/mod.rs` (1789 lines + 5
children) is the same shape. Under this campaign's standing rule — *"the original file survives as
the parent module"* — the rename route is not available, so **the model to copy is
`query/data.rs`**: keep the file, keep its shared traits/docs/tests in it, add `mod` lines, move
each coherent cluster verbatim into a responsibility-named child. It is also the route that leaves
every `crates/…/foo.rs` path mention in the gated docs still resolving (section 3.4, last row).

⚠️ Note what `gpu_scene` shows about durability: that split shipped at 3036 lines, moved out ~975,
and the parent is **8292 lines today**. A split is not a terminus; whatever the campaign leaves in
the parent will accrete again unless the seam it creates is the one new features naturally land in.

---

## 5. Ordered candidate list

Ordering rule as instructed: free files first, largest first. `leg` is the verification a
behaviour-preservation claim needs: **CPU** = the split can be proven by `cargo test` on this box;
**GPU** = the only behaviour gate is the golden pin set, which needs the device the owner is using.

### 5.1 Free source files, largest first

| # | lines | file | leg | gated/ungated anchors | pins | .stderr | macros | unsafe | cfg(feat) |
|---:|---:|---|---|---|---:|---:|---:|---:|---:|
| 1 | 6784 | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs` | GPU | 0/14 | 2 | 0 | 0 | 6 | 74 |
| 2 | 6139 | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs` | GPU | 0/95 | 2 | 0 | 0 | 95 | 19 |
| 3 | 6127 | `crates/boyko_rhi_vulkan/src/compute.rs` | GPU | 0/22 | 0 | 0 | 1 | 12 | 49 |
| 4 | 4010 | `crates/boyko_rhi_vulkan/src/ffi.rs` | GPU | 0/2 | 0 | 0 | 2 | 0 | 14 |
| 5 | 3542 | `crates/aether_lang/src/expand.rs` | CPU | 0/0 | 0 | 0 | 0 | 0 | 0 |
| 6 | 3448 | `crates/boyko_render/src/render_path_config.rs` | GPU + CPU | 6/38 | 2 | 0 | 0 | 0 | 3 |
| 7 | 3415 | `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` | GPU | 0/9 | 0 | 0 | 0 | 50 | 35 |
| 8 | 3343 | `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` | CPU | 15/2 | 0 | 4 | 6 | 35 | 0 |
| 9 | 3267 | `crates/boyko_rhi_vulkan/src/compute/tests.rs` | GPU | 0/10 | 0 | 0 | 0 | 0 | 0 |
| 10 | 2634 | `crates/boyko_ecs/src/ecs/core/asset/assets.rs` | CPU | 0/0 | 0 | 0 | 0 | 17 | 0 |
| 11 | 2588 | `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` | GPU | 0/7 | 2 | 0 | 0 | 48 | 11 |
| 12 | 2581 | `crates/boyko_render/src/light.rs` | GPU + CPU | 0/77 | 6 | 0 | 0 | 0 | 0 |
| 13 | 2515 | `crates/boyko_render/src/csm_config.rs` | GPU + CPU | 0/0 | 0 | 0 | 0 | 0 | 0 |
| 14 | 2243 | `crates/boyko_ecs/src/ecs/core/iters/query/state.rs` | CPU | 8/2 | 0 | 4 | 0 | 4 | 0 |
| 15 | 2235 | `crates/boyko_render/src/hzb.rs` | GPU + CPU | 0/0 | 0 | 0 | 0 | 0 | 0 |
| 16 | 2090 | `crates/boyko_ecs/src/ecs/core/profiling/tests.rs` | CPU | 0/0 | 0 | 0 | 0 | 1 | 6 |
| 17 | 2015 | `crates/boyko_app/src/gpu_scene/particle.rs` | GPU | 0/0 | 0 | 0 | 0 | 10 | 0 |
| 18 | 1979 | `crates/boyko_shaderdsl/src/bin/emit_particles.rs` | CPU | 0/0 | 0 | 0 | 0 | 0 | 0 |
| 19 | 1918 | `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` | CPU | 11/27 | 16 | 8 | 0 | 16 | 0 |
| 20 | 1865 | `crates/boyko_sdf_math/src/brick/tests.rs` | CPU | 0/0 | 0 | 0 | 0 | 0 | 0 |
| 21 | 1830 | `crates/boyko_sdf_math/src/brick.rs` | CPU | 0/2 | 0 | 0 | 0 | 0 | 0 |
| 22 | 1826 | `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs` | CPU | 6/3 | 4 | 0 | 0 | 83 | 0 |
| 23 | 1789 | `crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs` | CPU | 30/0 | 0 | 0 | 0 | 9 | 0 |
| 24 | 1784 | `crates/boyko_render/src/shadow_atlas.rs` | GPU + CPU | 0/1 | 1 | 0 | 0 | 0 | 0 |
| 25 | 1719 | `crates/aether_lang/src/parse.rs` | CPU | 0/8 | 0 | 0 | 0 | 0 | 0 |
| 26 | 1595 | `crates/boyko_ecs/src/ecs/core/profiling/store.rs` | CPU | 0/0 | 4 | 0 | 0 | 21 | 22 |
| 27 | 1592 | `crates/boyko_render/src/light_system.rs` | GPU + CPU | 0/14 | 1 | 0 | 0 | 2 | 0 |
| 28 | 1589 | `crates/boyko_ecs/src/ecs/core/commands/command_queue.rs` | CPU | 0/9 | 7 | 0 | 0 | 24 | 0 |
| 29 | 1548 | `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` | GPU | 0/4 | 0 | 0 | 0 | 9 | 0 |

### 5.2 Free test / bench files (lower priority by ruling)

| # | lines | file | leg | gated/ungated anchors | pins | .stderr | macros | unsafe | cfg(feat) |
|---:|---:|---|---|---|---:|---:|---:|---:|---:|
| 1 | 10434 | `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` | GPU | 0/6 | 0 | 0 | 4 | 55 | 96 |
| 2 | 8342 | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs` | GPU | 0/22 | 5 | 0 | 0 | 27 | 0 |
| 3 | 6672 | `crates/boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs` | GPU | 0/0 | 0 | 0 | 1 | 0 | 0 |
| 4 | 3316 | `crates/boyko_ecs/benches/cull_diagnostic.rs` | CPU | 0/0 | 0 | 0 | 1 | 0 | 1 |

⚠️ **Read the `leg` column before reading the size column.** The four largest free source files are
all `boyko_rhi_vulkan`; 7 of the top 12 are, and 9 of the top 12 counting the two `boyko_render`
entries — all of them files whose only behaviour gate is
`goldens/PINS.toml` + `scripts/golden.ps1`, i.e. a GPU the owner is currently using, and whose
tests are 135 of the 164 `#[ignore]`d ones. "Largest first" and "verifiable first" point in
opposite directions here, and that is a scheduling decision for the orchestrator, not a fact this
census can settle.

### 5.3 Recommended pilot: `crates/aether_lang/src/expand.rs` (3542 lines)

**The choice is forced, not preferred.** Applying the task's four pilot criteria to every crate:

| criterion | survivors |
|---|---|
| lib crate with a file ≥ 2500 lines | `boyko_rhi_vulkan`, `boyko_app`, `boyko_ecs`, `boyko_physics`, `aether_lang`, `boyko_render`, `boyko_shaderdsl`, `boyko_ui` (8) |
| …and no owner-dirty file anywhere in the crate | `boyko_physics` ✗ (lib.rs, plugin.rs), `boyko_rhi_vulkan` ✗, `boyko_ecs` ✗, `boyko_render` ✗, `boyko_app` ✗ → `aether_lang`, `boyko_shaderdsl`, `boyko_ui` |
| …and no open lane | `boyko_physics`/`boyko_threadpool` already out → unchanged |
| …and no unmerged branch touching any of its files | `boyko_shaderdsl` ✗ (`feat/ui-advanced` + `feat/golden-edsl-p0`), `boyko_ui` ✗ (`feat/ui-advanced`) → **`aether_lang` alone** |

`aether_lang` is the only crate in the workspace that passes all four. Its 6 files are touched by
**zero** of the 22 branches, by zero lanes, and by zero owner edits.

**It is also, independently, the cleanest split target in the census.** Every artifact class in
section 3 is *empty* for `expand.rs`, which is true of no other file ≥ 2500 lines:

```
gated-doc anchors          0        ungated-doc anchors        0
code line-pins             0        .stderr citations          0
macro_rules!               0        file-top #![...]           0
#[path]                    0        include_*!                 0
cfg(feature) sites         0        unsafe blocks              0
clippy allow sites         0        importers                  1  (aether_lang::lib)
```

**Verification strength.** `expand.rs` carries **54 inline `#[test]`s** of its own, and the crate's
integration half is `crates/aether_tests` — 17 test targets over ~56 committed trybuild fixtures
(`tests/ui/*.rs` + `*.stderr`). A trybuild `.stderr` is the byte-identity gate the brief asks for:
it pins the exact diagnostic text *and* the span coordinates inside the user's fixture, so a
faithful move leaves every byte identical, while a dropped rule, a reordered emission or a changed
span reds immediately and names the construct. `a5_material.rs` / `a6_scene.rs` / `a7_dx.rs` add a
second, different gate — the emitted engine paths must still *compile* against the real
`boyko_ecs` / `boyko_render` / `boyko_app`.

**Seams are already named by the graph description** (`desc_source: written`), so the split does
not need a design pass: `expand` → `recovered` (the failure path that emits per-failure
`compile_error!` + name-resolving stubs + the survivors) and `expand_v1`; then one emitter per
construct — `component`/`tag`/`bundle`/`event`, `system_fn` (+ `query_type`, `param_ty_and_mut`,
`type_mentions_mut`), `plugin_impl` (+ `bucket_stmts`, `resolve_order`), `machine_items`,
`material_fn`, `scene_fn`. Four to six children of 400-900 lines each, with the dispatcher and the
shared helpers staying in `expand.rs`.

**Two costs, both stated up front.** (1) `aether-tests` dev-depends on `boyko-render` and
`boyko-app` — its own `Cargo.toml` records the blast radius and a measured cold
`cargo check -p aether-tests` of ~31.7 s — so the integration leg goes red whenever the render lane
is red, for reasons unrelated to the DSL. Run `cargo test -p aether-lang --lib` (the 54 unit tests,
no engine deps) as the fast leg first, then `-p aether-tests`. (2) The 54 inline tests live in an
inline `#[cfg(test)] mod` that reaches private helpers; they stay in `expand.rs` and their targets
must become `pub(super)` when moved — every such widening is to be listed, per the standing rules.

**Runner-up, if the owner wants the pilot inside the kernel instead:**
`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` (3343, free, CPU leg, 191 test targets +
miri + proptest behind it). It is a harder pilot on purpose — 6 file-local `macro_rules!` whose
invocations must stay textually below them, 15 un-waived gated anchors, 4 `.stderr` citations, 44
importers — so it exercises every repair class the campaign will need later, on a file whose test
net is the strongest in the repo. `data.rs`, its sibling, is the split precedent to copy verbatim.

**Do NOT pilot on:** `ffi.rs`, `goldens.rs`, `component_pool.rs` — `docs/REFACTORING-PLAN.md`
records them as explicit non-goals ("large but cohesive"), and `component_pool.rs` additionally
carries 11 gated anchors and is `fix/ke13-ke14`-blocked.

### 5.4 Blocked files, grouped by what unblocks them

29 source files, **91,240 lines** (against 80,010 lines in the 29 free ones), wait on one of
eight events. Ordered by the lines each event releases — which is also the order in which landing
a lane buys the campaign the most work.

| unblocked when | census files (lines) |
|---|---|
| `feat/ui-advanced` merges | `crates/boyko_rhi_vulkan/src/present/scene_types.rs` (4470)<br>`crates/boyko_shaderdsl/src/emit/shaders.rs` (3009)<br>`crates/boyko_shaderdsl/src/emit/mod.rs` (2944)<br>`crates/boyko_ui/src/layout.rs` (2340)<br>`crates/boyko_shaderdsl/src/cf.rs` (2066)<br>`crates/boyko_render/src/gpu_column.rs` (1605) |
| lane A4/A5 physics (D:/wt/joltab) lands | `crates/boyko_physics/src/resources.rs` (3884)<br>`crates/boyko_physics/src/solver/colored.rs` (3476)<br>`crates/boyko_physics/src/solver/colored_tests.rs` (3334)<br>`crates/boyko_physics/src/solver/simd.rs` (1654)<br>`crates/boyko_physics/src/systems.rs` (1633) |
| owner commits (or reverts) his `feat/multi-paradigm-render` edits | `crates/boyko_rhi_vulkan/src/present/targets.rs` (10209)<br>`crates/boyko_log/src/codes.rs` (1630) |
| `fix/ddgi-host-hook` merges | `crates/boyko_app/src/gpu_scene/mod.rs` (8292)<br>`crates/boyko_render/src/upload.rs` (1517) |
| lane A6 pool/schedule (D:/wt/uploadleak) lands | `crates/boyko_ecs/src/ecs/core/schedule/schedule.rs` (2536)<br>`crates/boyko_threadpool/src/scope.rs` (2354)<br>`crates/boyko_threadpool/src/block.rs` (2166)<br>`crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs` (2053) |
| `fix/ke13-ke14` merges | `crates/boyko_ecs/src/ecs/memory/component_pool.rs` (4106)<br>`crates/boyko_ecs/src/ecs/core/archetype/archetype.rs` (2574)<br>`crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs` (1617) |
| `feat/golden-edsl-p0` merges | `crates/boyko_rhi_vulkan/src/goldens.rs` (5333) |
| `chore/msvc-host` merges AND owner commits (or reverts) his `feat/multi-paradigm-render` edits | `crates/boyko_rhi_vulkan/src/device.rs` (4251) |
| `fix/ddgi-host-hook` merges AND lane A6 pool/schedule (D:/wt/uploadleak) lands | `crates/boyko_app/src/runner.rs` (3816) |
| `feat/reflection` merges AND `fix/ke13-ke14` merges | `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` (2662) |
| `fix/asset-validate-prereqs` merges | `crates/boyko_render/src/mesh_draw.rs` (2088) |
| `feat/reflection` merges | `crates/boyko_macros/src/component.rs` (2067) |
| `fix/ke13-ke14` merges AND owner commits (or reverts) his `feat/multi-paradigm-render` edits | `crates/boyko_ecs/src/ecs/core/iters/query/query.rs` (1554) |

Reading that table as a schedule: **merging `fix/ke13-ke14` (1 commit ahead, 25 `.rs` files)
releases 8,297 lines of ECS-kernel census across three files** — and two more (`migration_helpers.rs`,
`query.rs`) once `feat/reflection` and the owner follow — while `feat/ui-advanced` (16 commits)
releases 16,434 lines across four crates. The single largest file in the repo,
`present/targets.rs` (10,209), is released by nothing but the owner committing.

---

## 6. What the previous campaign already did, and what it implies

`docs/REFACTORING-PLAN.md` (Part B, 2026-07) is the direct predecessor of this branch and its
verdicts should be inherited rather than re-litigated. Status of its Part B items, re-derived
from the tree:

| plan item | then | now | state |
|---|---|---|---|
| B1 `present/targets.rs` — "WORST offender (do first)" | 2689 | **10209** | **not done; grew 3.8×**; now owner-dirty |
| B2 `ecs_master.rs` — "safest, most mechanical seam", 103-method `impl` | 5318 | 1918 + 12 `*_api.rs` siblings | **done** (`ecs_master/` directory, 13 children) |
| B3 `component_registry.rs` — five fused registries | 3676 | `component_registry/mod.rs` 1789 + `{serialize,required,clone,tags,flags}.rs` | **done**, though the `mod.rs` remainder is still a census file |
| B4 `app/gpu_scene.rs` — one `*Resources` per feature | 3036 | **8292** + `{csm,tlas,interp,particle}.rs` | **partly done** (`3ac3047d`), parent grew 2.7× since |
| B5 `query/data.rs` | 3535 | 980 + 7 children | **done** (`e6ac6a48`) — the model split |
| B5 `rhi_impl.rs`, `device.rs`, `emit.rs`, `brick.rs` | 3568 / 3381 / 6014 / 3696 | 2588 / 4251 / 2944 / 1830+1865 | mixed; `brick` test-move done, the rest open |
| B6 harness-test fixture extraction | 9767 / 7724 | **10434 / 8342** | **not done; both grew** |
| non-goals `ffi.rs`, `goldens.rs`, `component_pool.rs`, `DeviceFns` | — | 4010 / 5333 / 4106 | "large but cohesive — leave" |

Three inferences for this campaign:

1. **The plan's own top-two priorities are the two files it is now least able to touch** — B1 is
   owner-dirty and B4 is `fix/ddgi-host-hook`-blocked. Whatever this campaign does first, it will
   not be the 2026-07 "do first".
2. **A split that leaves a fat parent does not hold.** `gpu_scene` and `targets.rs` both grew past
   their pre-split size. The seam has to be where the *next* feature lands (one file per
   `*Resources` / per target bundle), not merely a removal of the largest blocks.
3. **The behaviour-preservation argument the plan relied on is still valid and still costly**: the
   committed `.spv` bytes, `goldens/PINS.toml` (64 pins, header: a god-file split "MUST keep this
   byte-identical (Tier-0)") and `framegraph_gbuffer_equiv` make a render-side split *provably*
   behaviour-preserving — but only by running the device leg.

---

## 7. Residuals — what this census does not establish

* **Lane blocks are declared scope, not observed diffs.** Both `D:/wt/joltab` and
  `D:/wt/uploadleak` sit on `d552be05` with only untracked new test files today (2.2). If a lane
  is abandoned or finishes without touching its lib sources, its 5 + 4 census files become free
  without any merge.
* **The unmerged-branch table over-approximates** (2.3): `d552be05` is not an ancestor of any of
  those branches, so a listed file may already carry the same change here by another route.
* **Ungated anchor counts are produced by the gate's own binding algorithm applied to documents
  the gate never reads.** The sticky `**File:**` convention may be used differently outside
  `GATED_DOCS`, so the 1,018 figure is an upper bound on what a reader would call a real citation.
* **Nothing here measures how hard a given file is to split** — only what a split must not break.
  Cohesion (one 1070-line `create()` vs 103 independent methods) is a design question for the
  architect, and `graph.json`'s written descriptions (section 1.3) are the input to it.
* **`cargo` was run exactly twice** (the anchors gate, the hot-path script) and no build,
  benchmark or timing was taken; the machine is the owner's.
