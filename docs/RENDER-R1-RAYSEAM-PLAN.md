# R1 — Dormant unified ray / acceleration-structure seam (HW-RT track, rung R1)

Converged build spec (architect wf, 2026-07-04). Rung R1 of
[RENDER-HYBRID-RAY-SYSTEM-DESIGN.md](RENDER-HYBRID-RAY-SYSTEM-DESIGN.md) §8 (with §2/§3/§4 shapes).
**Zero-cost, dormant scaffolding: no AS, no RT extensions, no shader variant, no rendered-pixel
change. The grand_showcase golden stays byte-identical `58f6c6c3`.** Precedents mirrored one-for-one:
R0 `QueryPool` assoc-type + `DeviceCaps.timestamp_*`; DDGI `resolve_ddgi` 0%-gate + `DdgiResolveSet`.

## Locked decisions
- **D1 — `ray_query` HARD-WIRED `false` in R1.** R1 requests NO `VK_KHR_ray_query`/`acceleration_structure`
  extension, adds nothing to `VkDeviceCreateInfo`, loads no RT PFN. The field's contract is "ENABLED",
  so with nothing enabled the only honest value is `false` (a presence-query reporting `true` while
  disabled would make a consumer arm a null trace path → UB). The real presence+enable query is R2a
  (`feature=hwrt`). `vendor_id`/`device_id`/`driver_version` ARE populated with real values (free; R3's
  calibration cache key) — but `rt_tier()` gates on `ray_query`, so they arm nothing. ⇒ `rt_tier()==Absent`
  for every device in R1.
- **D2 — `AccelerationStructure` is a BARE unbounded assoc type, NO verbs.** `BoundAccelStruct` + the
  create/build/refit verbs + `VkAccelerationStructureKHR` FFI are all R2a. R1 declares only the type so
  R2a adds verbs without an `RhiApi` ABI break — exactly how `Surface`/`Swapchain`/`Texture` landed
  phases before their verbs.
- **D3 — `RayBackendConfig` is a cold `#[derive(Resource)]` POD**, resolved by a pure single-writer;
  `Default == DISABLED == every cell Software`. No consumer reads it in R1. Mirrors `ResolvedDdgi` +
  `resolve_ddgi_grid_gated` + `DdgiResolveSet`.

## 1. `boyko_rhi`
- **`api.rs`** (deferred-seam block, after `type BindGroupLayout;`): add `type AccelerationStructure;`
  (unbounded, no bound — NOT the FOUNDATION-NOW block which carries operational bounds).
- **`handle.rs`** `impl RhiApi for MockApi`: `type AccelerationStructure = ();`.
- **`lib.rs`**: no change (`RhiApi` already re-exported; assoc type travels with it). No new vocab.

## 2. `boyko_rhi_vulkan`
- **`rhi_impl.rs`** `impl RhiApi for Vulkan`: `type AccelerationStructure = ();` (cheapest placeholder;
  R2a rebinds to `BoundAccelStruct`).
- **`device.rs`**:
  - `RtTier` enum (`#[repr(u8)]` `Absent=0|Weak=1|Strong=2`), defined here (owns `DeviceCaps`), re-exported.
  - `DeviceCaps` += `ray_query: bool`, `ray_reorder: bool`, `vendor_id: u32`, `device_id: u32`,
    `driver_version: u32` (all `Copy`, no derive change). Doc each per D1.
  - `impl DeviceCaps { pub const fn rt_tier(&self) -> RtTier { if !self.ray_query { Absent } else if
    self.ray_reorder { Strong } else { Weak } } }`.
  - `query_device_caps` return literal: `ray_query: false, ray_reorder: false, vendor_id: 0, device_id: 0,
    driver_version: 0` (placeholders; IDs overwritten at the boot site).
  - Boot site (right after the R0 `timestamp_*` population): `device_caps.vendor_id = device_props.vendor_id;
    device_caps.device_id = device_props.device_id; device_caps.driver_version = device_props.driver_version;`
    — **plain field copies** (`vendor_id`/`device_id`/`driver_version` are typed `u32` at the TOP of
    `VkPhysicalDeviceProperties` in `ffi.rs`, NOT in the opaque limits blob → no offset math, no new FFI).
    `ray_query`/`ray_reorder` stay `false` (the dormancy anchor).
- **`lib.rs`**: re-export `RtTier` (+ confirm `DeviceCaps` export).

## 3. `boyko_render` (new `ray_backend.rs` + `ray_plugin.rs`)
- **Vocab** (`#[repr(u8/usize)]`): `RayBackend { Software=0, HardwareTri=1, HardwareMixed=2 }` (+`COUNT=3`);
  `RayWorkload { Shadow, Ao, GiProbe, Reflection }` (+`COUNT=4`); `RayGeom { Mesh, Sdf }` (+`COUNT=2`).
  R1 only ever SELECTS `Software`; the HW arms are declared so R2a's ABI is stable.
- **`RayBackendConfig`** `#[derive(Resource)] #[repr(C)]`: `table: [[RayBackend; RayGeom::COUNT];
  RayWorkload::COUNT]`, `budget: [u16; RayWorkload::COUNT]`, `_pad: [u8; 8]`. `DISABLED` const = every
  cell `Software`, budget `[1;..]`. `Default = DISABLED`. Add a `size_of` layout-pin const-assert
  (mirror `ResolvedDdgi`).
- **`resolve_ray_backend(tier: RtTier) -> RayBackendConfig`** (pure): every arm (`Absent|Weak|Strong`) →
  `DISABLED` in R1 (the `Weak`/`Strong` arms are written now so R2a fills them without reshaping the fn).
  Narrow `RtTier` arg (per-tier testable); R3 widens to `(RayCalibration, DeviceCaps)`.
- **`resolve_ray_backend_system(caps: Res<RayCaps>, mut out: ResMut<RayBackendConfig>)`**: single-writer,
  `*out = resolve_ray_backend(caps.tier)`. `#[allow(clippy::needless_pass_by_value)]` (as `resolve_ddgi_grid`).
  Add `debug_assert!(all cells == Software, "R1 must resolve all-software")` (an R1 tripwire R2a removes).
- **`RayCaps { pub tier: RtTier }`** `#[derive(Resource)]`, `Default = Absent` (dormant if never filled).
  **Developer: FIRST check for an existing world-resident `DeviceCaps` resource** (how `DdgiCaps` gets the
  boot query in `ddgi_plugin.rs` / the host) — if one exists, read THAT instead of a ray-specific mirror.
- **`RayResolveSet` + `AsBuildSet`** `#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]` — mirror
  `DdgiResolveSet`. `RayResolveSet` orders the resolve before consumers (none in R1). `AsBuildSet` is the
  empty by-name anchor R2a hangs the AS-build + `.after_set(AsBuildSet)` consumers on.
- **`RayPlugin`** (mirror `DdgiPlugin`): `insert_resource(RayBackendConfig::default())` +
  `insert_resource(RayCaps::default())` + `add_system(resolve_ray_backend_system).in_set(RayResolveSet)`;
  register `AsBuildSet` (explicit `configure_set` ONLY if the scheduler needs a member-less set configured
  to be an `.after_set` anchor — developer confirms against how `DdgiResolveSet` becomes orderable).
- **`lib.rs`**: `pub mod ray_backend; pub mod ray_plugin;` + re-export the vocab/config/resolve/sets/plugin
  (+ `RtTier` if the crate surfaces caps).
- **Host wiring**: add `RayPlugin` to the render plugin group + the `RayCaps` boot-fill at the SAME site
  `DdgiCaps` is filled from `context.device_caps()`.

## 4. Byte-identity argument (the proof — each link independently sufficient)
1. No RT extension requested → `VkDeviceCreateInfo` bytes unchanged → same device/queue/PFNs.
2. `AccelerationStructure = ()` both backends, no verb → no AS, no build cmd, no barrier.
3. `rt_tier()==Absent` (gated on `ray_query==false`) for every device.
4. `resolve_ray_backend(Absent)==DISABLED==default()` → every cell `Software`.
5. No consumer reads `RayBackendConfig`; the sets have no command-recording members → no SPIR-V/variant/trace.
6. The new IDs are recorded-only; nothing branches on them.
∴ command stream + every pixel unchanged.

**Gates (must stay green, unchanged):** grand_showcase golden `58f6c6c3` (`goldens.rs`) · `framegraph_gbuffer_equiv`
· the full existing golden suite · workspace `clippy -D warnings` (touch sources first — false-fresh box) ·
`software_ray_baseline_cost` still runs. No new `unsafe` → no Miri needed.

## 5. Deferred to R2a/R3 (scope discipline)
Real extension enumerate+enable+pNext+PFN load + `ray_query=true` (R2a); `BoundAccelStruct`/`AsKind`/AS verbs
+ the `ACCELERATION_STRUCTURE_WRITE→READ` barrier pair (R2a); the shadow-traversal shader variant + `.spv` +
`trace_visibility` intrinsic (R2a); `RayCalibration`/cache/decision-margin/micro-bench (R3); `BOYKO_RAY_BACKEND`
env override (R2a); routing that fills the `Weak`/`Strong` arms (R2a/R4). **If a change needs `feature=hwrt`,
a new FFI type, or a `.spv`, it is OUT of R1.**

## 6. RISKS
- **R-1 unbounded assoc type breaks a generic bound** (low — 9 existing `()` deferred types prove the pattern;
  `cargo check --all-targets` after the 3 assoc-type edits is the canary; reject any helper bounding
  `A::AccelerationStructure`).
- **R-2 `ray_query` accidentally `true`** (low — defence-in-depth: `resolve_ray_backend`'s Weak/Strong arms
  still return DISABLED in R1; add the `resolve==DISABLED` unit test + the boot canary `caps.ray_query==false`).
- **R-3 IDs at wrong offset** (RETIRED — typed `u32` at the top of `VkPhysicalDeviceProperties`; plain copy).
- **R-4 over-building** (medium temptation — the §5 table is the contract).
- **R-5 `RtTier` wrong crate / dep-cycle** (define once in `boyko_rhi_vulkan::device`, re-export; verify no
  `boyko_rhi_vulkan→boyko_render` dep).
- **R-6 `AsBuildSet` empty-set registration** (mirror `DdgiResolveSet`; add `configure_set` only if required).
- **R-7 `RayCaps` host-fill unwired** (benign now — default `Absent` keeps R1 dormant; wire it at the
  `DdgiCaps` site so R2a inherits a tested seam).

## 7. Files (dependency order)
1 `boyko_rhi/src/api.rs` · 2 `boyko_rhi/src/handle.rs` · 3 `boyko_rhi_vulkan/src/rhi_impl.rs` ·
4 `boyko_rhi_vulkan/src/device.rs` (RtTier + 5 caps + rt_tier + query literal + boot ID copy) ·
5 `boyko_rhi_vulkan/src/lib.rs` (re-export RtTier) · 6 `boyko_render/src/ray_backend.rs` (new) ·
7 `boyko_render/src/ray_plugin.rs` (new) · 8 `boyko_render/src/lib.rs` · 9 host wiring (RayPlugin + RayCaps fill).

**Gates in order:** `cargo check --all-targets` (after step 3 = RISK-1 canary, then per render step) →
`cargo build --release` → `clippy --all-targets -D warnings` (touch first) → `cargo test --all-targets`
(unit + golden suite) → orchestrator on RTX: grand_showcase `58f6c6c3` + framegraph equiv + the boot canary
(`ray_query==false`, `rt_tier()==Absent`, IDs nonzero). Windowed dumps need `--test-threads=1`.

## 8. Mandatory unit tests (pure, CI-able)
- `RayBackendConfig::default()==DISABLED`, every cell `Software`.
- `resolve_ray_backend(Absent|Weak|Strong)==DISABLED` (the R1 all-software invariant — RISK-2 lock).
- `rt_tier`: `ray_query=false ⇒ Absent` (any `ray_reorder`); `true,false ⇒ Weak`; `true,true ⇒ Strong`.
- layout pin (`size_of::<RayBackendConfig>()`), `COUNT`s (3/4/2), resolve idempotence.
- **boot canary (GPU):** `caps.ray_query==false`, `rt_tier()==Absent`, IDs nonzero on the RTX box.

## Open questions (developer resolves against the real code)
`RtTier` home (`boyko_rhi_vulkan::device`, re-exported — flag if a non-Vulkan backend later needs it) ·
narrow `RtTier` resolve arg (recommend) · `AsBuildSet` empty-set registration semantics ·
`RayCaps` vs an existing world-resident `DeviceCaps` resource (check first).
