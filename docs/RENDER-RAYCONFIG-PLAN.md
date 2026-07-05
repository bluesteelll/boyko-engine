# Configurable Ray-System — plan + as-built

Owner directive (2026-07-05): the three HW-RT shadow follow-ups, **all
configurable**, and **static where a dynamic knob would cost hot-path perf**
("если динамика дорогая → статические параметры"). Denoise: owner said **"все
варианты"** ⇒ denoise itself becomes a configurable choice, default None
(byte-identical), TAA/spatial as additive opt-in capabilities.

## The spine (already existed)

`crates/boyko_render/src/ray_backend.rs`: `RayBackendConfig { table[Workload][Geom],
budget[Workload] }` ECS Resource, resolved by `resolve_ray_backend(tier)` in
`RayResolveSet`. `RayWorkload` = Shadow/Ao/GiProbe/Reflection. `RayGeom` =
Mesh/Sdf. `RayBackend` = Software/HardwareTri/HardwareMixed. Consumer selection
lives in `present/passes/gbuffer.rs` (`hwrt_triple`).

## Rungs

| # | Rung | Status |
|---|------|--------|
| 1a | Spec-constant support in the RHI (first-class primitive) | **SHIPPED** `c22bac5` |
| 1b | RayShadowConfig + tunable shadow params (count=spec-const, cone/tmax/tmin/bias=UBO) | **SHIPPED** `31fd66a` |
| 2 | Runtime backend toggle (consumer reads resolved cell + force_software) | **SHIPPED** `8d6bcc5` |
| 3a | Spatial bilateral shadow denoise (opt-in) | next |
| 3b | TAA subsystem (opt-in) | pending |
| 4a | Unified mesh+SDF TLAS (procedural AABB → HardwareMixed) | pending |
| 4b–d | HW-RT workloads: AO → Reflection → GiProbe (SDFDDGI→HW-RT) | pending |
| 4e | Per-GPU self-calibration (boot micro-bench → RayBackendConfig.table) | pending |

## Key decisions (locked)

- **Perf split (owner directive):** a `[loop]` trip count must be a **spec-const**
  so the driver unrolls it (a UBO trip count forces a dynamic loop → ~16× branch
  cost). In-loop **scalars** (cone/tmax/tmin/bias) have no loop-bound impact ⇒
  plain **UBO fields** (one hoisted uniform load, live-tunable, zero rebuild).
- **Spec-const RHI (1a):** `SpecConstant { id: u32, value: u32 }` (float ⇒
  `x.to_bits()`) + `ComputePipelineDesc.spec_constants: &[SpecConstant]`. EMPTY ⇒
  literal `p_specialization_info = null` ⇒ every existing pipeline byte-identical.
  Blob assembled as same-scope stack locals across `vkCreateComputePipelines`.
- **RayShadowConfig (1b):** `Config → Resolved → cold policy → plugin` (mirrors
  CSM/DDGI/SSAO). `ray_count` baked into spec-const id 0 at boot (retune =
  relaunch — the resolve pipeline is boot-built, no live rebuild path); the 4 UBO
  scalars flow every frame through a **true per-FIF ring** (mirrors
  `csm_cascade_ring`, bound per-`[slot]`).
- **Enablement is NOT in RayShadowConfig** (no `enabled` field) — it lives in
  `RayBackendConfig.table[Shadow][Mesh]` (the Rung-2 toggle). RayShadowConfig is
  always-on TUNING subordinate to that.
- **Denoise = a configurable choice** (Rung 3), default None = byte-identical =
  the engine's documented no-TAA "SPATIAL, not temporal" convention preserved as
  the default; spatial + TAA are additive opt-in.

## The golden gate (reusable, every rung)

Software byte-identity (RT disarmed in `grand_showcase` ⇒ hwrt-OFF == hwrt-ON):

```powershell
$env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-gnu'; $env:BOYKO_DISABLE_VALIDATION='1'
# hwrt-OFF (omit --features hwrt) AND hwrt-ON (--features hwrt):
cargo test -p boyko_rhi_vulkan [--features hwrt] --test window_present_gbuffer `
  engine_grand_showcase_512_screenshot_dump -- --ignored --test-threads=1
Get-FileHash D:\tmp\engine_grand_showcase.bmp -SHA256
# must == 58f6c6c3d986f7a393ea53b01c5021e7360cf6f1b32bf9db05d4d8bb98999dd5
```

RT-armed owner-eval render (the live soft-shadow path):
`BOYKO_HOST_DUMP=D:\tmp\<x>.bmp cargo run -p boyko-app --example showcase --features hwrt`
(approved R2a-4b render sha = `95ae0f5b…`).

Shader recompile (orchestrator only; subagents can't run fresh GPU exes):
`dxc -spirv -T cs_6_5 -E main -D HWRT=1 "-fspv-target-env=vulkan1.3" deferred_pbr.hlsl -Fo deferred_pbr_hwrt.comp.spv`
(software variant `-T cs_6_0`, no `-D HWRT` — MUST stay 65456 B; recompile to a
temp path + sha-diff to prove `#else` byte-invariance, never overwrite the frozen
software `.spv`). PowerShell mangles `-fspv-target-env=vulkan1.3` unless quoted.

## As-built notes

- **Noise reshuffle (1b, expected, accepted):** making `ray_count` a spec-const
  changes DXC's FP lowering of `1.0/SHADOW_RAY_COUNT` vs the folded literal, so
  marginal rays in the **grainy** single-frame penumbra flip hit/miss. RT render
  sha `af934c50` ≠ approved `95ae0f5b`, but **structurally identical**: 947/576000
  px differ full-res (max 57), collapsing to ≤3 levels after 16× downsample /
  gauss-blur — the shadow shape/softness/position is unchanged; only the noise
  seed reshuffled. Owner already accepted the grain as parameter-tunable; Rung 3
  denoises it. This is inherent to spec-const configurability and cannot be
  avoided without giving up the tunable count.
- **Seam — live ray_count retune:** needs a later rung to device-idle + destroy
  (`gpu_scene.rs` resolve_pipeline_hwrt teardown) + rebuild the HWRT resolve
  pipeline. Boot-bake suffices now (retune = relaunch).
- **Latent (out of scope, pre-existing):** `shadow_atlas_ubo` uses the same
  single-slot-bound pattern 1b's W1 fixed for `ray_shadow_ubo` — a dormant
  cross-frame WAR hazard only under runtime atlas-param retune mid-motion. Worth
  a follow-up ticket; ships today and passes the golden.
