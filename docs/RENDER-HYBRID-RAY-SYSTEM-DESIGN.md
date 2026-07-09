# Architecture: Hybrid mesh+SDF hardware-adaptive ray/query system (`boyko-engine`)

**Status:** design / pre-implementation, critic-revised (all P0/P1 folded). Supersedes the SDF-only `docs/RENDER-HWRT-OPTIONAL-ANALYSIS.md` (retained as ONE input; its SDF-tiny verdict becomes a single routed cell here). No implementation code — seam, structures, algorithms, adaptive policy, roadmap, honest gates. Owner decisions flagged **[VALUES CALL]**.

---

## Changes from critique

**P0-1 (R2 is not cheap; value rests on M3 mesh gather).** FOLDED. R2 is split: **R2a** lands + measures the AS builder in isolation on **hard** RT shadows (1 ray, no denoiser) vs shadow-maps; **R2b** adds the denoiser as a separate gated *quality* rung. The M3 mesh-instance dependency is now stated explicitly as an R2a precondition — R2a cannot prove mesh value until a real multi-mesh instanced scene exists. The "cheap first rung" claim is corrected: the genuinely cheap value-proving rungs are **R0 (instrument, no HW)** and **R1 (dormant seam)**; R2a is the *first HW rung* and is honestly costed.

**P0-2 (calibration is a flaky oracle).** FOLDED as a specified algorithm, not adjectives. Added: (1) a **decision margin** — within N% → pick SW (byte-golden, zero-risk); (2) a **plausibility band** — absolute ns/ray outside a sane window rejects the run → falls back to `rt_tier` prior; (3) a **cache-trust predicate** — a cheap boot spot-check re-validates a loaded cache; (4) a **force-override** escape hatch. See §4.

**P0-3 (variant-matrix under-costed).** FOLDED. The matrix is written down with explicit `.spv` counts per rung, and — critically — **collapsed at the source**: traversal control flow is a **single parameterized hand-HLSL body switched by a compile-time preprocessor/specialization constant**, so backends are *generated build products of one authored body*, not N hand-maintained bodies. Artifact count and validator named per rung. See §6.

**P1-1 (config can't express geometry-split routing).** FOLDED. Routing is a **per-pass variant-selection concern**, not a flat per-workload field. `RayBackendConfig` is redefined as a **(workload × geometry) eligibility+backend table**; the mesh pass and SDF pass each read their own cell. See §2, §5.

**P1-2 (AS-build→trace ordering not pinned to a named set).** FOLDED. Added a named **`AsBuildSet`** that all ray-consuming passes join with `.after_set(...)`, mirroring the verified `DdgiResolveSet` cross-plugin precedent. GPU barrier orders GPU execution; the set orders system recording. See §3, §Multithreading.

**P1-3 (`RayBackend::COUNT` undefined; calibration times impossible backends).** FOLDED. `RayBackend::COUNT` defined; calibration times only **built, device-eligible** cells via an eligibility mask; unbuilt/faulted cells = `f32::INFINITY` sentinel so min-selection never picks them. See §2, §4.

**P2-1 / P2-2.** FOLDED. Physics row labeled "(not a seam workload)"; unmeasured cells = `f32::INFINITY`.

---

## 1. Executive summary + layers

**One seam, many backends, resolved once.** Every ray/query consumer (primary, shadow, AO, GI/SDFDDGI, reflections; physics is deliberately *not* on the seam) calls two abstract intrinsics — `trace_closest`, `trace_visibility` — that are **emitted into a prebuilt shader variant**, selected at setup from a cold Resource. No per-ray `dyn`, no hot-path uniform branch. The 0%-gate default is **all-software, byte-identical to today's renderer**.

**Hardware-adaptive without a dev-box bench.** Backend choice = `DeviceCaps` (RT present/absent) → coarse vendor/generation `rt_tier` prior → a **cached first-launch micro-calibration** on the user's actual GPU (with a specified robustness algorithm) → a cold `RayBackendConfig` table. Vulkan exposes no RT-throughput property, so per-GPU measurement is the *only* generalizable selector — this is the bench replacement the owner mandated.

**Hybrid, lean-core + extension points.** The buildable core is small: a shared-TLAS mesh-triangle path (the canonical HW win) + the existing software SDF march (the byte-golden authority). "Comprehensive" is delivered by **routed extension rungs** (AO, reflections, mixed mesh+SDF TLAS), each gated behind a measured win — not by building an unfinishable monolith up front.

### Layer diagram

```
┌─ CONSUMERS (shaders) ─ shadow · AO · GI-probe · reflection ── (physics: CPU, off-seam)
│      call: trace_closest() / trace_visibility()   ← emitted into a prebuilt variant
├─ RESOLUTION (cold, setup-only) ──────────────────────────────────────────────
│   RayBackendConfig[workload][geom] ← resolve_ray_backend( RayCalibration, DeviceCaps )
│   RayResolveSet → AsBuildSet → (consumer passes .after_set)
├─ BACKENDS (build-time variants of ONE parameterized traversal body) ─────────
│   SoftwareSdfMarch(golden) · HardwareTriBvh · HardwareMixed · [SoftwareMeshDf?]
├─ ACCELERATION (Principle-0, RHI-owned, derived from ECS columns) ────────────
│   mesh BLAS ← MeshRegistry │ SDF-AABB BLAS ← SdfPrimitive col │ TLAS ← instance affine col
├─ RHI (raw-FFI, in-house, feature="hwrt", VK_KHR_ray_query) ──────────────────
│   AS create/build/refit/compact · build→trace barrier (framegraph auto-barrier)
└─ HARDWARE-ADAPTIVE SELF-TUNING ─────────────────────────────────────────────
    DeviceCaps.ray_query + rt_tier prior + cached micro-calibration (robust, override-able)
```

---

## Owner decisions (RESOLVED 2026-07-04)

The four §9 VALUES calls are answered; this section is authoritative over §9's original framing.

1. **Reframed guarantee — ACCEPTED.** "Byte-identical software-always; HW-RT is a bounded-tolerance, owner-eval-only opt-in." The software path stays the deterministic byte-golden authority.

2. **Mesh foundation FIRST.** The **M3 mesh-instance gather** (a real multi-mesh instanced `MeshRegistry`, replacing the harness-single-mesh state) is built **before** any HW-RT rung — it is both the R2a precondition and a core hybrid-engine capability in its own right. Roadmap order is therefore **M3 (mesh foundation) → R0 (instrument) → R1 (dormant seam) → R2a (mesh RT shadows) → …**.

3. **BOTH backends are always shipped; the GAME DEVELOPER chooses (owner correction).** The engine is NOT an auto-picks-one system. Every routed workload exposes a per-workload selector the game developer sets: **`software | hardware | auto`**. `software` and `hardware` are explicit, always-available, always-maintained choices (both paths ship). **`auto`** is a convenience default that runs the per-GPU calibration (§4) and picks for the developer. The **decision margin** is ONLY the tiebreak *inside* `auto` (on a near-tie it prefers the safe software path); it NEVER overrides an explicit `software`/`hardware` selection. So: want HW always → get HW; want SW always → get SW; want "do the right thing per GPU" → `auto`. `RayBackendConfig` (§2) is the developer-facing table; calibration only fills cells the developer left on `auto`.

4. **Static budget + compile-time-max.** Backend selection and the hot path resolve at **compile time** (the prebuilt-variant switch — zero per-ray overhead; already core, §2/§6). Tuning VALUES (ray count, subset, GI_MAX_IT) stay **runtime config** so game developers tune without a rebuild. The ray **budget is static** (calibrate-once); the optional per-frame frame-time controller (R6) is NOT built by default — matching the compile-time / no-runtime-controller preference.

---

## 2. The unified ray abstraction + backend set + zero-overhead resolution

### The intrinsics (emitted, not runtime-dispatched)
- `trace_closest(origin, dir, tmax) -> Hit { t, geom_id, normal }`
- `trace_visibility(origin, dir, tmax) -> f32` (any-hit early-out)

These are **shader-side names resolved to a prebuilt SPIR-V variant** at pipeline-selection time. The "choice" is a cold enum read once; the hot loop has no `use_hwrt` branch. This is the standout-correct seam (critic-affirmed): the abstraction lives at the **cache/integrator boundary** (variant selection + the shared radiance cache), never per ray. Consumers write into the shared cache (SDFDDGI octahedral atlas / G-buffer); readers are backend-ignorant — exactly Lumen's "unify at the surface cache" shape, and our octahedral atlas already *is* a DDGI-class radiance cache.

**Two variants, not one über-shader — a correctness argument (not just perf):** a software-only device may **reject the `RayQuery` SPIR-V capability at pipeline creation** (DXC #4113 / `SPV_KHR_ray_tracing`). Separate variants are mandatory, not stylistic.

### Backend set

```rust
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayBackend {
    SoftwareSdfMarch = 0,  // byte-golden default; ≤16-edit fold, no traversal cost
    SoftwareMeshDf   = 1,  // OPTIONAL, may never build; only if a bench beats HW on RT-weak
    HardwareTriBvh   = 2,  // mesh triangle BLAS, inline rayQuery, NO intersection shader
    HardwareMixed    = 3,  // shared TLAS: tri + SDF-AABB (pipeline path on AMD); deferred rung
}
impl RayBackend { pub const COUNT: usize = 4; }   // P1-3: defined
```

### Config — a (workload × geometry) table (P1-1 fix)

The routing matrix (§5) keys on BOTH workload and geometry (Shadow-mesh→HW, Shadow-SDF→SW). A flat `[RayBackend; WORKLOAD]` cannot hold that. Geometry routing is a **per-pass variant-selection concern**: the mesh pass reads `[Shadow][Mesh]`, the SDF pass reads `[Shadow][Sdf]`.

```rust
#[repr(usize)] #[derive(Clone, Copy)]
pub enum RayWorkload { Shadow = 0, Ao = 1, GiProbe = 2, Reflection = 3 }
impl RayWorkload { pub const COUNT: usize = 4; }   // Primary=raster/march always, not routed
                                                   // PhysicsQuery deliberately absent (CPU-only)
#[repr(usize)] #[derive(Clone, Copy)]
pub enum RayGeom { Mesh = 0, Sdf = 1 }
impl RayGeom { pub const COUNT: usize = 2; }

#[repr(C)] #[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayBackendConfig {
    /// The resolved backend per (workload, geometry). DISABLED => every cell SoftwareSdfMarch.
    pub table: [[RayBackend; RayGeom::COUNT]; RayWorkload::COUNT],
    /// Per-workload dynamic ray budget (rays/probe for DDGI; ray count for shadow/AO).
    pub budget: [u16; RayWorkload::COUNT],
    pub _pad: [u8; 8],
}
impl RayBackendConfig {
    /// 0%-gate: every cell SoftwareSdfMarch — byte-identical to today. Mirrors ResolvedDdgi::DISABLED.
    pub const DISABLED: Self = /* all SoftwareSdfMarch, default budgets */;
}
impl Default for RayBackendConfig { fn default() -> Self { Self::DISABLED } }
```

Cold POD singleton, read once at setup to pick variants — no cache-line / false-sharing concern (never hot multi-threaded).

---

## 3. Hybrid mesh+SDF acceleration structure

**Two BLAS families, ONE TLAS** (the documented Vulkan hybrid pattern): mesh BLAS = `VK_GEOMETRY_TYPE_TRIANGLES_KHR`; SDF BLAS = `VK_GEOMETRY_TYPE_AABBS_KHR` (one AABB per SDF primitive/instance). Per-instance `instanceCustomIndex` + `instanceShaderBindingTableRecordOffset` disambiguate the leaf. One `rayQuery` sees the whole hybrid scene; the SDF intersection-shader occupancy penalty is paid **only by rays that pierce SDF-AABB instances** — the triangle scene traces at full RT-core throughput in the same call.

**Principle-0 derivation (critic-affirmed clean):** the AS is an **RHI-owned GPU resource derived from ECS columns** — mesh BLAS ← `MeshRegistry` model-space vertex/index buffers; SDF-AABB BLAS ← the `SdfPrimitive` column; TLAS ← the per-instance 3×4 affine column. It is the `GpuColumn`/`MeshRegistry` FFI/GPU exception class, **never** a side `Vec`/`HashMap`.

```rust
#[repr(C)]
pub struct BoundAccelStruct {         // boyko_rhi_vulkan, feature="hwrt", !Send (device-bound)
    handle: VkAccelerationStructureKHR,
    buffer: BoundBuffer,              // DeviceLocal, from the existing suballocator
    device_address: u64,             // packed into a bindless GPU buffer derived from columns
    kind: AsKind,                    // Blas | Tlas
}
```

**Build→trace ordering — a named set, not just a barrier (P1-2 fix):** the GPU barrier (`ACCELERATION_STRUCTURE_WRITE→READ`, one new access-mask pair folded into the committed framegraph auto-barriers) orders **GPU execution**; it does NOT order the *system* that records the build vs. the systems that record the traces — which may live in different plugins (the exact cross-plugin invisibility that motivated `DdgiResolveSet`, verified `ddgi_config.rs` L219-235). Therefore a named **`AsBuildSet`** is introduced; every ray-consuming pass joins `.after_set(AsBuildSet)`, and TLAS-refit-writes-this-frame → trace-reads-this-frame is a **set edge**, not merely a GPU barrier.

---

## 4. Hardware-adaptive self-tuning (the bench replacement)

Three resolution layers, mirroring in-tree `DeviceCaps` + `resolve_ddgi`:

**(1) `DeviceCaps` boot query (RECORDED-vs-fail-fast).**
```rust
#[repr(C)] pub struct DeviceCaps { /* ...existing... */
    pub ray_query: bool,       // KHR_ray_query + acceleration_structure enabled; false => silent SW
    pub ray_reorder: bool,     // EXT_ray_tracing_invocation_reorder (SER) — recent-generation signal
    pub vendor_id: u32, pub device_id: u32, pub driver_version: u32,  // calibration cache key
}
impl DeviceCaps { pub const fn rt_tier(&self) -> RtTier { /* Absent | Weak | Strong */ } }
```
`ray_query=false` is the graceful-degrade anchor — absent extensions → all-software, never boot-fail. `rt_tier` is a **prior**, not a throughput number (Vulkan exposes none).

**(2) Cached first-launch micro-calibration** — the linchpin, now specified as an algorithm (P0-2):

```rust
#[repr(C)] #[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayCalibration {
    pub key: CalibKey,          // {vendor_id, device_id, driver_version, probe_version}
    /// Median ns/ray per (workload, backend). Unbuilt/faulted/ineligible = f32::INFINITY (P1-3,P2-2).
    pub ns_per_ray: [[f32; RayBackend::COUNT]; RayWorkload::COUNT],
    pub eligible: u16,          // bitmask of built+device-eligible backends actually measured
    pub inline_vs_pipeline_ratio: f32,   // Decision-6 fork, MEASURED (AMD ~1.65), not assumed
    pub blas_refit_vs_rebuild_ratio: f32,
    pub state: CalibState,      // Uncalibrated | InProgress | Done | Faulted
}
```

Calibration algorithm (once per `{vendor,device,driver,probe_version}`; else load cache):
1. **Warm-up** discard dispatch (boost clocks, cache fill).
2. Render a canonical mini-scene (one mesh-BLAS + one SDF-AABB instance) with each **built, device-eligible** backend (`eligible` mask; ineligible cells stay `INFINITY` so they can never be selected).
3. Each leg sized ≥ tens of µs (clear the sub-10 µs timer floor); **median-of-N**, reject outliers.
4. **Plausibility band (P0-2.2):** if a leg's absolute ns/ray falls outside a sane per-tier window (detects thermal throttle / background compile / power-save clocks) → **reject the whole run**, fall back to the `rt_tier` prior (or SW). A rejected run is *not* cached as `Done`.
5. Persist via `boyko_serialize`, keyed on the IDs + `probe_version`.
6. **Fault path:** a HW leg that faults (validation-crash-prone box) → `Faulted` → permanent SW for that `device_id`, never boot-fail.

**(3) `resolve_ray_backend` — single writer, the selection rules (P0-2.1/.3):**
- **Decision margin:** if the best HW and SW cells are within **N%** (owner-set, **[VALUES CALL #3]**, default e.g. 15%) → pick **SW** (byte-golden, zero-risk). HW is chosen only on a *decisive* win, never a coin-flip.
- **Cache-trust predicate:** on load, a cheap boot **spot-check** (one warmed timing of the currently-selected HW cell) must land within a tolerance of the cached value; else the cache is stale/implausible → re-run calibration or fall back to prior.
- **Force-override escape hatch:** an env/config key (`BOYKO_RAY_BACKEND=software|auto|hw`) overrides the cache for field debugging of a mis-selection.
- **Pure function:** `resolve_ray_backend(RayCalibration, DeviceCaps) -> RayBackendConfig`, single-writer (like `resolve_ddgi_grid`), ordered before any consumer by `RayResolveSet`.

**Dynamic budget [VALUES CALL #4]:** baseline = **static** (calibrate once, Principle-1-pure, no controller). A per-frame frame-time knob-controller (DDGI rays/probe, DRS-style damped loop + hysteresis) is an *optional* R6 rung — oscillation risk, gated behind a bench. **Recommend static-first.**

---

## 5. Per-workload × geometry × tier routing matrix

Encoded as `resolve_ray_backend` output (the table), **not** a hardcoded global. "Strong/Weak" = the calibrated `rt_tier`; calibration can override the prior. Every row honest about backend strength.

| Workload | Geom | RT-Strong | RT-Weak | RT-Absent | Why |
|---|---|---|---|---|---|
| Primary | Mesh | Raster (always) | Raster | Raster | Coherent; raster optimal; feeds the golden. Not routed. |
| Primary | SDF | SoftwareSdfMarch | Software | Software | ≤16-edit fold, no traversal; byte-golden. Not routed. |
| **Shadow** | Mesh | **HardwareTriBvh** | HW if calib decisively wins, else shadow-maps | Shadow-maps | Real BVH, no `rint` — canonical HW win. |
| Shadow | SDF | Software soft-shadow march | Software | Software | Low traversal at 16 edits; cone/DDA wins. |
| **AO** | Mesh | **HardwareTriBvh** | calib-decided | SSAO | Secondary G-buffer rays; strong HW case. |
| AO | SDF | Software | Software | Software | Same as SDF shadow. |
| **GI probe** | Mesh | **HardwareMixed** (feed mesh RT rays into probe update) | calib-decided | Software mesh-DF (if built) | Unifies mesh into the GI flagship. |
| GI probe | SDF | SoftwareSdfMarch | Software | Software | Best *technical* HW candidate but 16-edit fold beats any AS; swaps only at large instance count (R5). |
| **Reflection** | Mesh | **HardwareTriBvh** (hit-lighting) | calib-decided | SSR | Mirror quality needs real BVH; SSR fallback. |
| Large rigid SDF sets | SDF | **HardwareMixed** *iff beats software grid* | Software grid | Software grid | Only regime with real empty space to cull (R5 gate). |
| Physics query | Any | **CPU fold (always)** *(not a seam workload — shown for completeness)* | CPU fold | CPU fold | Zero-readback, deterministic, Principle-0 — permanent. |

The prior analysis's true finding survives intact: tiny-16-edit SDF → software march; physics → CPU fold permanent. "Comprehensive" did **not** revive AABB-AS for the 16-edit case — `HardwareMixed` is gated behind R5 + a "beats software grid" bench.

---

## 6. Raw-FFI RHI surface + eDSL composition + feature-gate/degrade + the collapsed variant matrix (P0-3)

**RHI seam:** `RhiApi::AccelerationStructure` (unbounded associated type — same deferred pattern as `Surface`/`Swapchain`, no ABI break, no `dyn`). Device create: pNext `AccelerationStructureFeatures` + `RayQueryFeatures` + `bufferDeviceAddress`; enable `VK_KHR_acceleration_structure`, `ray_query`, `deferred_host_operations`, `buffer_device_address` — all behind `supports_ray_query` → `DeviceCaps.ray_query`. Hand-loaded PFNs: `GetAccelerationStructureBuildSizes`, `CreateAccelerationStructure`, `CmdBuildAccelerationStructures`, `GetAccelerationStructureDeviceAddress`, `CmdWriteAccelerationStructuresProperties`, `CmdCopyAccelerationStructure`. No VMA, no nv-helpers — scratch from the existing suballocator with BDA usage.

**Inline `rayQuery` first; pipeline path is a calibrated escape hatch.** Consumers are already compute passes where `field_distance` lives → inline `rayQueryEXT` is the smallest raw-FFI surface (no RTPSO/SBT). Mesh visibility uses `ACCEPT_FIRST_HIT_AND_END_SEARCH | SKIP_PROCEDURAL_PRIMITIVES`. The honest AMD fork (inline ~65% slower than pipeline) is a **calibration output** (`inline_vs_pipeline_ratio`), taken only if the SDF-procedural rung (R5) ships.

**The variant matrix — collapsed at the source (P0-3 fix).** The naive count is (Shadow, AO, GiProbe, Reflection) × (sw, hw_tri, hw_mixed) = up to **12 hand-authored bodies** — an unshippable in-house tax with per-variant owner-eval (HW can't CI). Collapse rule:

- **ONE parameterized hand-HLSL traversal body per consumer**, with the backend chosen by a **compile-time preprocessor / specialization constant** (`#if RAY_BACKEND == HW_TRI …`). Backends are therefore **generated build products of one authored body**, not N maintained bodies. This keeps zero per-ray overhead (compile-time switch, no uniform branch) *and* collapses authoring to one body per consumer.
- **The SDF field leaf stays eDSL-emitted** (`boyko_shaderdsl`), byte-identity-gated — the frozen `field_probe_gate` on `field_distance`/`sdf`/`smin`/normals is untouched; the AABB intersection variant *calls* it. eDSL owns the field math; the traversal control flow is the one parameterized hand-HLSL body (traversal is not field math — `rayQueryEXT`/`OpRayQuery*` is not something the field eDSL models).

**Honest `.spv` artifact count + validator, per rung:**

| Rung | New authored bodies | Generated `.spv` variants | Validator |
|---|---|---|---|
| R1 | 0 | 0 (dormant seam) | golden byte-identity (trivial) |
| R2a | 1 (shadow traversal) | 2 (sw, hw_tri) | golden for sw; **owner-eval** for hw_tri (hard shadows, 1 body) |
| R2b | 0 (denoiser is a pass, not a traversal body) | +0 | owner-eval (in-motion capture; cross-frame-target discipline) |
| R4 | +2 (AO, reflection) | +4 (sw/hw_tri each) | golden sw; owner-eval hw |
| R5 | +1 field-leaf reuse in `rint`; +1 traversal `#if` arm | + (hw_mixed arm on existing bodies) | owner-eval + beats-software-grid bench |

Owner-eval scales because it is **per authored body (≈5 total at full scope), not per generated variant** — the collapse is what makes the system finishable in-house.

**Feature gate / degrade:** `hwrt` off by default → zero AS surface, zero SPIR-V-capability risk. Runtime `!ray_query` → the sw variant, byte-untouched. Never boot-fail.

---

## 7. Determinism / byte-identity / test disposition

- **Software backends** (SDF march + optional mesh-DF) = **golden gates, unchanged** — the frozen `field_probe_gate` stays absolute.
- **HW backends** = **one tolerance class per authored body**, owner-eval-only (per-vendor; this single-box repo cannot CI multi-vendor) — ΔE/SSIM for shadows/reflections, irradiance-RMS for DDGI. Hard spec fact (R4): Vulkan HW-RT has explicit no-ordering / no-cross-vendor determinism → HW **cannot** be byte-identical to the IEEE software march.
- **CPU physics** = byte-identical, never HW-RT → the render↔physics geometric-agreement invariant is fully preserved.
- **Matrix cost:** O(software goldens, unchanged) + O(authored-body owner-evals ≈5) — **not** O(workloads × geometries × backends). This is the collapse rule that keeps validation finite.

**Unit tests (mandatory):** `RayBackendConfig::default() == DISABLED == all-software`; `resolve_ray_backend` with `!ray_query` → all-software; calibration cache round-trip keyed on IDs; `rt_tier` per vendor-id; fault → `Faulted` → SW; **decision-margin** (SW/HW within N% → SW); **plausibility-band** (out-of-window leg → run rejected, not cached); **eligibility mask** (ineligible cell stays `INFINITY`, never selected). **Property tests:** calibration median-of-N stable across repeated runs within tolerance; `resolve_ray_backend` pure in `(RayCalibration, DeviceCaps)`. **debug_assert!:** AS build sizes non-zero; `device_address != 0` post-create; `budget[w] > 0` when HW-routed; `AsBuildSet` edge present before any trace records; loaded-cache `key` matches `DeviceCaps` IDs before trust.

---

## 8. Phased roadmap (honest gates)

Framing corrected per P0-1: the genuinely cheap value-proving rungs are **R0 + R1**; R2a is the *first HW rung* and is honestly costed, not called "cheap."

- **R0 — Instrument the software baseline (no HW code).** GPU timestamps around the DDGI probe march, SDF shadow march, mesh shadow pass → ns/ray/workload on the real scene. **Also the calibration timestamp harness.** Gate: none — do first, unconditionally.
- **R1 — Dormant unified seam (cheap, pure option value).** Land `RhiApi::AccelerationStructure` (unbounded), `DeviceCaps.ray_query`+IDs+`rt_tier()`, `RayBackendConfig` (0%-gate, all-SW Default), `RayResolveSet`, `AsBuildSet`. **No AS, no variant, no RT extensions.** Gate: `default()` resolves all-software; golden byte-identical (trivial).
- **R2a — AS builder + HARD mesh RT shadows, measured in isolation (first HW rung, honestly costed).** `feature=hwrt`. Raw-FFI static triangle BLAS from `MeshRegistry` + TLAS from the instance affine column; **one** parameterized shadow traversal body (sw + hw_tri arms); build→trace barrier under `AsBuildSet`; **no denoiser** (1 ray, hard shadows). **Precondition (P0-1):** the **M3 mesh-instance gather** must ship first — R2a cannot prove mesh value without a real multi-mesh instanced scene (`MeshRegistry` is harness-single-mesh until M3). Gate: on the real scene, `mesh_blas_build_ms` + `tlas_refit_ms` acceptable AND hard RT shadows beat/upgrade shadow-maps (owner-eval); silent degrade to shadow-maps when `!ray_query`. **If this gate fails, the AS builder is not extended — value discovered before the denoiser is written.**
- **R2b — RT-shadow denoiser (quality rung, separately gated).** Temporal accumulation + SVGF-class spatial denoise + history-clamp/motion reprojection. Gate: owner-eval with **in-motion capture** (the "wrong-only-in-motion" cross-frame-target class — wait-fence-before-per-FIF-write discipline, per the G-buffer-ring lesson). Authorized only after R2a proves the base win.
- **R3 — First-launch micro-calibration + adaptive selection.** The §4 algorithm: robust probe, decision margin, plausibility band, cache-trust spot-check, force-override, AMD forks. `resolve_ray_backend` reads the cache → `RayBackendConfig`. Gate: calibration robust (median-of-N, warm-up, driver-key, band-reject) and faults degrade to SW. **This is the dev-box-bench replacement.**
- **R4 — Mesh HW → AO + reflections.** +2 authored bodies (AO, reflection). Gate: per-workload calibration says HW wins decisively on the tier.
- **R5 — Unified mesh+SDF TLAS (SDF-AABB coexistence).** Mixed TLAS + SDF `rint` calling the eDSL leaf, pipeline-path on AMD. **Only if** large rigid SDF-instance sets exist that out-bench a software spatial grid. Gate: beats the compute-shader grid net of intersection-shader cost.
- **R6 — TLAS refit for dynamic rigid instances + optional frame-time controller** ([VALUES CALL #4]). Gated on a shipped dynamic-instance campaign + a controller bench (oscillation risk).

**Never built:** SDF meshing (forfeits analytic field/byte-identity); physics GPU-RT (permanent CPU authority, zero-readback determinism, Principle-0).

---

## 9. Final recommendation + VALUES calls

**Recommendation:** approve the lean core (R0 → R1 → R2a) as the buildable spine; treat R2b/R3/R4/R5/R6 as **measured extension rungs**, each behind its honest gate, delivering the owner's "comprehensive + universal" over time without an up-front unfinishable monolith. The seam is zero-overhead, Principle-0-clean, and degrades silently to today's byte-golden software renderer on any device.

**VALUES calls for the owner:**
1. **#1 — Reframed guarantee:** "byte-identical software-always; HW-RT bounded-tolerance, owner-eval-only opt-in" (unavoidable — Vulkan spec no-determinism). Confirm.
2. **#2 — M3 precedence:** R2a is gated behind the M3 mesh-instance gather. Confirm M3 ships (or is scheduled) before R2a — otherwise R2a stalls at "no scene to prove value on."
3. **#3 — Decision margin N%** (default ~15%): the SW-vs-HW tie-break threshold below which the byte-golden SW wins. Owner sets the number.
4. **#4 — Static vs dynamic budget:** recommend static-first (Principle-1-pure); the per-frame controller is R6, gated on an oscillation bench.

---

**Grounding (branch `ecs`, verified):** `RhiApi` unbounded-associated-type (`crates/boyko_rhi/src/api.rs` L46-67); `DeviceCaps` RECORDED-vs-fail-fast (`crates/boyko_rhi_vulkan/src/device.rs` L160-221); `resolve_ddgi` 0%-gate + `DdgiResolveSet` cross-plugin ordering (`crates/boyko_render/src/ddgi_config.rs` L196-249); `MeshRegistry` BLAS source + M3 gather status (`crates/boyko_render/src/mesh_registry.rs` L1-19); instance 3×4 affine TLAS source (same L24-27); DDGI producer + Principle-0 discipline (`crates/boyko_render/src/ddgi_update.rs` L1-80); `SdfPrimitive` capability-as-presence + boot-static gather (`crates/boyko_render/src/sdf_edit.rs` L1-27). Retained input: `docs/RENDER-HWRT-OPTIONAL-ANALYSIS.md` (SDF-tiny = one routed cell; physics-AVOID verbatim). Fetched research R1-R4 marked in-brief; DDGI texel counts + inline-procedural ergonomics are training-knowledge.

**Relevant files (absolute):**
- `D:\claude\BoykoEngine\crates\boyko_rhi\src\api.rs` — add `type AccelerationStructure`.
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\device.rs` — `DeviceCaps` additions + device-create feature/extension seam.
- `D:\claude\BoykoEngine\crates\boyko_render\src\ddgi_config.rs` — the `resolve_ddgi`/`DdgiResolveSet` pattern mirrored by `RayBackendConfig`/`RayResolveSet`/`AsBuildSet`.
- `D:\claude\BoykoEngine\crates\boyko_render\src\mesh_registry.rs` — triangle BLAS source (M3 gather is the R2a precondition).
- `D:\claude\BoykoEngine\crates\boyko_render\src\ddgi_update.rs` — the `RayBackend` producer-swap home.
- `D:\claude\BoykoEngine\crates\boyko_render\src\sdf_edit.rs` — `SdfPrimitive` SDF-AABB source.
- `D:\claude\BoykoEngine\docs\RENDER-HWRT-OPTIONAL-ANALYSIS.md` — prior SDF-only analysis (retained input).
- New: `crates/boyko_rhi_vulkan/src/accel.rs`; `crates/boyko_render/src/{ray_backend,ray_calibration,accel_build}.rs`.