# Refactoring Plan — structural debt + shader exponential growth

Branch `ecs`. Two problem classes, both owner-flagged:
1. **Shaders grow exponentially** ("неграмотная структуризация шейдеров") — the variant/`.spv` count and the hand-maintained registry balloon with every feature.
2. **Oversized files / god-objects** — 3k+-line files that grew by accretion.

This plan is **evidence-based** (line counts + a per-file/per-shader analysis) and **prioritized by pain × safety**. The governing safety property throughout: **every committed `.spv` + the `grand_showcase` golden + the `framegraph_gbuffer_equiv` pins make a refactor provably behaviour-preserving** — a change that reproduces the frozen `.spv` bytes (or the golden `58f6c6c3`) cannot alter runtime behaviour. Refactoring here is *low-risk by construction*, which is why it is worth doing now.

---

## Part A — Shader exponential growth

### A0. The growth mechanism (why it compounds)

One shipped shader variant costs, TODAY, all of this by hand:
1. author a `-D` axis in the monolithic `.hlsl` (`deferred_pbr.hlsl` carries `#if HWRT`, `#if SHADOW_STAGE==…`, `#ifdef MOTION_VECTORS` in one 1494-line file);
2. run `dxc` offline by hand (the command lives in a header comment — **there is no `build.rs` and no compile script in the repo**; every `.spv` is human-produced + git-committed);
3. hand-write a `compute.rs` registry entry: `static NAME_SPV: SpirvBlob<N>` with a **hand-counted byte length `N`**, a `pub fn name_spirv()` accessor, and a 10-20 line doc block;
4. the `SpirvBlob<N>` const-size guard trips if the `.spv` length ≠ `N` — a good tripwire, but `N` is maintained by hand.

Result: **51 `.spv`, 38 `SpirvBlob<N>` embeds + 38 accessors** in a **6776-line `compute.rs`** (44 % of which is an inline `#[cfg(test)]` module). Cost per variant ≈ 15-20 registry lines + a hand-counted size + an offline compile + a committed blob + a golden. The owner's "exponential" is accurate for the *reachable* variant subset — each new axis multiplies the combinations it interacts with, and each combination is full hand-maintenance.

Three distinct, separable root causes — with three distinct fixes:

### A1. Root cause 1 — copy-paste variant FILES (the acute, highest-ROI fix)

Near-identical `.hlsl` files differing in **one baked constant**:
- `sdf_probe_update_it{32,64,96,128}.comp.hlsl` — 4 × ~427 lines, diff = **2 lines** (`static const uint GI_MAX_IT = Nu;` + the compile comment).
- `sdf_ssao_{low,medium,high}.comp.hlsl` — 3 × 370 lines, diff = **3 lines** (`SSAO_SLICES/SLICES_F/STEPS`).
- `sdf_ssao.comp.hlsl` — a **4th, byte-identical dead duplicate** of `_medium` (`diff` is empty).

That is **8 redundant source files + 8 `.spv` + 8 registry embeds** (~3 200 duplicated shader lines) encoding what one source + a compile-time constant expresses.

**Why they were authored as files** (documented rationale, `compute.rs:429-440`): "Mechanism C — a variant is selected by binding a different pipeline, NEVER by a dynamic loop bound, so every `[unroll]`/`[loop]` trip count stays fully unrolled at ZERO per-pixel cost." The baked const IS an unroll/trip count.

**The rationale is sound but does NOT justify FILE duplication.** A **specialization constant** delivers the *identical* property — the trip count resolves at *pipeline-create*, so the driver still specializes/unrolls, from **one** `.hlsl`. The engine already proves this in the *same* deferred family: `SHADOW_RAY_COUNT` is a spec-const (`deferred_pbr.hlsl:317`, `[[vk::constant_id(0)]]`) whose Vogel loop unrolls. Rung-1a added `SpecConstant{id,value}` (`descriptor.rs:172`) + `ComputePipelineDesc.spec_constants` (`descriptor.rs:220`) **explicitly "to collapse SSAO/DDGI variant-explosion"** — the lever exists and is unused for these families.

**Fix (A1):** collapse `GI_MAX_IT` (4→1) and SSAO quality (4→1) to one source each via spec-const; delete the dead `sdf_ssao.comp.hlsl`. **−8 `.hlsl`, −8 `.spv`, −~120 registry lines.**

**VERIFIED (investigation 2026-07): NO perf risk — the "measure the unroll" caveat is MOOT for `GI_MAX_IT`.** The `probe_march` trip loop is already `[loop]` (`sdf_probe_update_it64.comp.hlsl:186` — an explicit DYNAMIC loop, driver forbidden to unroll), exactly like the shipped `SHADOW_RAY_COUNT` spec-const (`deferred_pbr.hlsl:1195` `[loop]`). A spec-const bound on a `[loop]` is structurally identical to a baked-const bound — same dynamic loop, same 64 iterations. **The compute.rs:429 "keeps every `[unroll]` fully unrolled at zero cost" rationale for the 4 files was FACTUALLY WRONG** (the loop is `[loop]`, never unrolled either way); the 4 files were pure waste. (SSAO: confirm its slice/step loops' attribute the same way — if `[unroll]`, a spec-const may not fold; if `[loop]`, same no-risk.)

**Scope note:** the `probe_march` body is eDSL-generated (`boyko_shaderdsl::probe_march` + the `emit_probe_gi` bin) + spliced into all 4 files, guarded by `ddgi_probe_gi_sync`. So the collapse ALSO simplifies the eDSL (emit → ONE file) but touches: the emit bin (target 1 file), the sync test (1 file), the 4 `.hlsl` (rename it64→`sdf_probe_update.comp.hlsl`, delete 3), `compute.rs` (4 embeds+selector → 1 embed + `sdf_probe_update_spirv()` no-arg), `gpu_scene.rs:519` + the bench (create the pipeline with `spec_constants=[SpecConstant{id:0, value:gi_max_it}]`; `GI_MAX_IT` at `sdf_probe_update_it64.comp.hlsl:98` becomes `[[vk::constant_id(0)]] const uint GI_MAX_IT = 64;` — OUTSIDE the splice, so the eDSL body is untouched). Verify: golden `58f6c6c3` (GI-OFF, byte-identical) + the `ddgi_probe_gi_arm` #[ignore] smoke (GI-ON, atlas non-zero at default 64). A focused multi-file unit — best done with dedicated context.

**TURNKEY (investigation 2026-07):** the pipeline infra is ALREADY wired — `gpu_scene.rs:536-547`
creates the probe-update pipeline via `ComputePipelineDesc { …, spec_constants: &[] }`. And because
the spec-const carries a **default of 64** (`[[vk::constant_id(0)]] const uint GI_MAX_IT = 64;`), a
pipeline built with `spec_constants: &[]` resolves GI_MAX_IT to 64 — **byte-identical to the baked
`static const 64u`**. So the DEFAULT callers just drop the arg (`sdf_probe_update_spirv(GI_MAX_IT_DEFAULT)`
→ `sdf_probe_update_spirv()`, `spec_constants` stays `&[]`); ONLY the bench sweep needs the override.
Exact edit set (5 callers): `gpu_scene.rs:519` (drop arg), `ddgi_probe_gi_arm.rs:260` (drop arg),
`window_present_gbuffer.rs:7883` (drop arg), `software_ray_baseline_cost.rs:345` (drop arg), and
`ddgi_probe_gi_cost.rs:387-390` — the ONLY spec-const site: `sdf_probe_update_spirv()` +
`spec_constants: &[SpecConstant{id:0, value:gi_max_it}]` in its per-value pipeline build. Plus
`emit_probe_gi.rs:51` (emit ONE `sdf_probe_update.comp.hlsl`, GI_MAX_IT as `constant_id(0)`),
`ddgi_probe_gi_sync.rs:88` (read the 1 file), `compute.rs` (4 embeds+selector → 1
`SDF_PROBE_UPDATE_SPV` + `sdf_probe_update_spirv()`), delete the 4 `it{N}` `.hlsl`+`.spv`. Same recipe
for SSAO (`SpecConstant` id per slice/step). **Executes on a CLEAN tree after B1 (both touch the
render golden — serialize for attribution).**

### A2. Root cause 2 — the hand-maintained embed REGISTRY (`compute.rs`)

`compute.rs` fuses three responsibilities + a giant test module:
- the 38 `SpirvBlob`+accessor registry (each with a hand-counted `N` + a paragraph of docs);
- std430/push param structs + encoders (`SsaoParams`, `CompositePushConstants`, `FineMarcherPush`, `ClusterCullPush`, `M2GridParams`, `MeshSdfParams`, `TileBound`, …);
- host-golden helpers;
- a **3018-line inline `#[cfg(test)]` module** (44 %).

**Fix (A2) — a build-time registry codegen; the hermetic constraint does NOT block it.** The two concerns are separable:
- **compile `.hlsl`→`.spv`** needs dxc ⇒ stays offline + committed (CI has no SDK — keep it);
- **embed `.spv`→Rust** needs no SDK ⇒ a `build.rs` (or a declarative manifest + codegen) reads each committed `.spv`, takes its length via `std::fs::metadata` (**killing the hand-counted `N`**), and emits the `static`+accessor from a table `{ name, path, cfg, doc }`.

The committed `.spv` and the generated registry coexist. **−600-750 lines of boilerplate + the entire hand-counted-size failure mode.**

### A3. Root cause 3 — monolithic `-D` variants that MUST stay separate (registry-only fix)

`SHADOW_STAGE` {INLINE,VIS,DENOISED}, `HWRT` {0,1}, `MOTION_VECTORS` {0,1} **cannot** collapse to one `.spv` — each changes the interface (VIS strips lighting + writes `gShadowVis`; DENOISED reads the vis image + declares no AS; HWRT adds `OpCapability RayQueryKHR` + descriptor @19; MOTION_VECTORS adds bindings/MRTs). A spec-const cannot add/remove a descriptor or delete half a shader. **These stay N `.spv` from 1 source** (already single-source-via-`-D` — the good pattern, e.g. `gbuffer_mrt_mv.*.spv` compiled from the one `gbuffer_mrt.*.hlsl`). The registry de-dup (A2) is the only lever here — plus a **variant-manifest doc** declaring the reachable `SHADOW_STAGE×HWRT×MOTION_VECTORS` matrix in ONE table, so a new variant is a table row, not a scavenger hunt through `compute.rs` docs.

### A4. The eDSL (`boyko_shaderdsl`) — keep, do NOT scale

The eDSL emits **HLSL fragments** (math bodies) pasted into committed `.hlsl` between `=== GENERATED … ===` sync-pins, byte-identity-gated against a Rust host oracle (`eval_byte_identity.rs`). It correctly single-sources the *math shared with a golden* (marcher/field/oct/pack/soft-shadow/ssao) — Principle-0 aligned. **Do NOT grow it into a whole-shader author**: the resolve/denoise variants differ by *bindings and data-flow*, not math (which the eDSL doesn't model), and it would not reduce the artifact count. Correct scope = math single-sourcing; leave it.

### A5. Shader remediation — ordered

| # | Step | Mechanism | Δ | Safety |
|---|---|---|---|---|
| A-1 | Collapse `GI_MAX_IT` (4→1) + SSAO quality (4→1); delete dead `sdf_ssao.comp.hlsl` | spec-const (infra ready) | −8 files, −8 `.spv`, −~120 reg lines | per-value GPU-diff vs the current `.spv` = bit-identical |
| A-2 | Table/manifest-driven registry codegen (`build.rs`, read `.spv` len) | codegen (no SDK) | −600-750 lines in `compute.rs` | generated bytes == committed `.spv` |
| A-3 | Split `compute.rs`: `std430_params.rs` + move the 3018 test lines to `tests/` | mechanical | −~3 000 apparent lines | compile + golden |
| A-4 | Variant-manifest doc for the stay-separate `SHADOW_STAGE×HWRT×MV` matrix | doc | — | — |
| A-5 | Keep eDSL for math bodies (no change) | — | — | — |

---

## Part B — Oversized files / god-objects

Top production files, with verdicts. **Legit-large-leave**: `ffi.rs` (3360, raw Vulkan bindings), `goldens.rs` (4229, flat host oracles — optional light split), `component_pool.rs` (3701, perf-critical column store — just move inline tests), `DeviceFns` (69-field FFI fn-pointer table — legitimately flat).

### B1. `present/targets.rs` (2689) — WORST offender (do first)

`GBufferTargets` (`:47`) is a textbook accretion god-struct: a growing image-ring list (`depth…shadow_temporal_hist, temporal_out`) + a growing descriptor-set-ring list. **Every render feature appends one image + one set field here + one block to `create()`** — which is now **1070 lines** (`:1316`), the single largest function in the codebase. This session alone appended `motion_vec`/`shadow_temporal_hist`/`temporal_out` + `shadow_vis_mv`/`shadow_temporal` sets. The extraction pattern is *already begun* (`build_shadow_denoise_sets` :509, `build_shadow_temporal_sets` :910) — push it to completion:
- split the struct into `CoreGBuffer` (depth/albedo/normal/material/lit/viewt), `AoTargets`, `ShadowDenoiseTargets` (`#[cfg(hwrt)]`), `TemporalTargets` (`#[cfg(hwrt)]`);
- `create()` becomes a ~100-line orchestrator calling `CoreGBuffer::build`, `AoTargets::build`, … The next feature becomes a new sub-bundle, not another append.

### B2. `ecs_master.rs` (5318) — safest, most mechanical seam

One `EcsMaster` struct, one `impl` with **103 public methods** (spawn/despawn, component, query, resource, command, event, hierarchy). The split pattern **already exists** (siblings `tag_api.rs`, `enable_tag_api.rs` hold `impl EcsMaster` groups). Extend: `entity_api.rs`, `component_api.rs`, `query_api.rs`, `resource_api.rs`, `command_api.rs`, `event_api.rs`, `hierarchy_api.rs`. A pure move (Rust allows `impl` across files in a module), no logic change — highest mechanical value.

### B3. `component_registry.rs` (3676) — five fused registries

Layout (`ComponentLayout`/`StorageKind`/`ResidencyKind`), tags (`TagId`/`EnableTagId`), required-components (`RequiredEntry`/`RequiredPlan`), cloning (`Cloneability`/`CloneProbe`), serialize (`Serializability`/`SerializeProbe`/`WireFnProbe`). Split `component_registry/{layout,tags,required,clone,serialize}.rs`.

### B4. `app/gpu_scene.rs` (3036) — accretion (one `*Resources` per feature)

`CsmResources`, `TlasResources`, `MotionVecResources`, `InterpGpuProd`, `GpuSceneBundles`, each `create()` ~300 lines. Split `gpu_scene/{csm,tlas,motion_vec,interp,bundles}.rs`.

### B5. Splittable-but-lower-priority

- `rhi_impl.rs` (3568): split `{pipeline,bind_group,buffer_image,command}.rs`; extract sub-builders from the 448-line `create_graphics_pipeline`.
- `query/data.rs` (3535): mechanical split `{read,write,ref,mut,option,anyof,tuple_impls}.rs`.
- `device.rs` (3381): split `{boot,caps,fn_tables}.rs`.
- `emit.rs` (6014): split `emit/{core,hlsl_lib}.rs`.
- `brick.rs` (3696, 50 % test): move the 1868 test lines to `tests/`.

### B6. Test-file fixture extraction (test-only, lower risk, high line-count pain)

`tests/window_present_gbuffer.rs` (**9767**) and `tests/sdf_gbuffer_hybrid.rs` (**7724**) are the two largest files in the repo — larger than any production file — because each test copies the whole boot/present/BMP-readback/scene-build setup (this session added 4 `GBufferScene`-literal sites × the field churn). Extract a shared harness (`boot`, `present`, `readback_bmp`, scene builders) so each test is a scenario, not a setup clone. Also move the inline `#[cfg(test)]` modules out of `compute.rs` (3018) + `brick.rs` (1868) — that alone roughly halves their apparent size.

---

## Unified roadmap (pain × safety)

1. **A-1** — spec-const collapse `GI_MAX_IT` + SSAO (−8 files/`.spv`; provable byte-identity; infra ready). *Highest ROI, lowest risk.*
2. **B1 `targets.rs`** — decompose `GBufferTargets` + the 1070-line `create()` (worst single offender; pattern begun).
3. **B2 `ecs_master.rs`** — split the 103-method `impl` (safest mechanical move; sibling pattern exists).
4. **A-2** — manifest-driven registry codegen (−600-750 lines; no SDK needed).
5. **B3/B4/B5** — per-cluster splits (`component_registry`, `gpu_scene`, `query/data`, `rhi_impl`, `device`, `emit`).
6. **B6** — test-fixture extraction + move inline tests out of `compute.rs`/`brick.rs`.

Each step is independently shippable behind the byte-identity gate. Recommended cadence: one item per commit, golden/equiv-verified, so a refactor never rides with a behaviour change.

### Explicit non-goals (leave as-is)
`ffi.rs`, `goldens.rs`, `component_pool.rs`, `DeviceFns` — large but cohesive. The eDSL is correctly scoped; do **not** grow it to solve variant explosion. `SHADOW_STAGE/HWRT/MOTION_VECTORS` stay N-`.spv`-from-1-source — the registry (A-2), not a collapse, is their lever.
