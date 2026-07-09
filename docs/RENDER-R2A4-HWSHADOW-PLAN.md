# R2a-4 — rayQuery `#if HWRT` mesh shadow (HW-RT, first VISIBLE rung → owner-eval)

Status: **✅ SHIPPED** — R2a-4a (AS-descriptor RHI) @29c2fe7, R2a-4b (rayQuery variant + routing + soft
cone-sampled shadows) @8eb770f. **OWNER-APPROVED** on the RTX 3060. Feature `hwrt` (default OFF). Rung of
`docs/RENDER-R2A-HWRT-SHADOWS-PLAN.md`; builds on R2a-3 (@e919670, per-frame GPU-resident TLAS —
`docs/RENDER-R2A3-TLAS-PLAN.md`).

**As-built notes (vs the design below):** (1) `TMax` = a WORLD-space constant `SHADOW_RAY_TMAX=1e4` (the
critic P0-2 fix — `split_far` is view-space, dimensionally wrong). (2) The HWRT resolve triple is gated on
`scene.tlas.is_some()` (a TLAS built + barriered this frame) — a zero-mesh frame falls back to the software
triple, so no unbuilt/stale TLAS is ever traced (the code-review P0). (3) The routing gate at the boot layer
is `ctx.ray_query_enabled()` (capability-presence, matching the R2a-3 tlas gate) — `RayBackendConfig`
resolves `HardwareTri` on an RT tier and agrees by construction; a runtime config toggle (Software fallback
on RT hardware) is a follow-up. (4) **Soft shadows:** the single-ray hard version was owner-rejected ("too
sharp"); shipped = 16 Vogel-disk rays in the sun's ~2° cone (`SHADOW_CONE_RADIUS=0.035`) + per-pixel IGN
rotation → soft penumbra + contact-hardening. Owner accepted the single-frame sampling noise as
parameter-tunable; **follow-up = TAA / higher ray count to denoise it.**

## Goal

The deferred resolve routes its **mesh-shadow term to a `rayQuery` TLAS trace** (replacing the CSM
shadow-map sample for mesh geometry), gated by `feature=hwrt` + runtime `ctx.ray_query_enabled()` +
`RayBackendConfig.table[Shadow][Mesh] == HardwareTri`. Culminates in an **owner-eval** (a human visual
verdict: RT mesh shadows vs the shadow-map render). Every degrade path (hwrt off / ray_query absent /
config Software) stays byte-identical to the golden `58f6c6c3`.

## VALUES (locked by orchestrator — do not relitigate)

- RT **REPLACES** the CSM mesh-shadow term (not additive); the SDF analytic shadow stays min-combined.
- A **separate `deferred_pbr_hwrt.comp.spv`** keeps the software resolve `.spv` byte-identical.
- `instanceCustomIndex = i` (R2a-3 packs it); bias tuned at owner-eval.

## HEADLINE DISCOVERY (from the resolve-pipeline investigation)

**Binding 19 is NOT free, and the RHI has NO acceleration-structure descriptor support.**
`MAX_BIND_GROUP_BINDINGS = 19` (`crates/boyko_rhi/src/device.rs:42`); the resolve set declares exactly
19 bindings, indices **0..=18** (`crates/boyko_app/src/gpu_scene.rs:1571-1592`). A TLAS at index 19 is
the **20th** binding → over the cap. `DescriptorKind` (`crates/boyko_rhi/src/enums.rs:585-603`) has only
5 variants (no AS); there is **zero** `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` /
`VkWriteDescriptorSetAccelerationStructureKHR` in the codebase. So the descriptor plumbing must be built.

## The rayQuery contract (DXC de-risked on this box)

Confirmed: `dxc 1.9.0.5347` compiles `-T cs_6_5 -E main -spirv -fspv-target-env=vulkan1.3` on a
`RayQuery<...> q; q.TraceRayInline(tlas,0,0xFF,ray); q.Proceed(); q.CommittedStatus()==COMMITTED_TRIANGLE_HIT`
shader → valid SPIR-V with `OpCapability RayQueryKHR` + `OpExtension "SPV_KHR_ray_query"`.
`RaytracingAccelerationStructure tlas : register(t0)` binds as `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR`.

HLSL (behind `#if HWRT`): origin = surface world pos `+ N * bias`, dir = `GpuLight.dir_kind.xyz` (world
dir TO the light), `TMin ≈ 1e-3`, **`TMax = a large WORLD-space constant** covering the scene (the
directional light is at infinity, so any mesh occluder along the world-space ray casts). Do **NOT** use
`CascadeData.split_far` — it is a VIEW-space (camera-forward-projected) distance, dimensionally wrong for
a world-space ray (critic P0-2). Start with a scene-bound world constant (e.g. `SHADOW_RAY_TMAX = 1e4`,
or the CSM's world-space far coverage if a bounded trace is wanted); the MAGNITUDE is tuned at eval but
the DIMENSION is fixed world-space.
`RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES`;
`hit ⇒ occluded ⇒ shadow`.

---

# R2a-4a — AS-descriptor RHI support (byte-identical infra)

Nothing consumes it yet ⇒ byte-identical golden. Own GPU smoke. This isolates the silent-FFI risk
(raw sType/pNext — the class that bit R2a-1/R2a-2) from the visible R2a-4b.

## Steps

1. **`MAX_BIND_GROUP_BINDINGS` 19 → 20 in BOTH locations** — the agnostic `crates/boyko_rhi/src/device.rs:42`
   AND the vulkan-backend mirror `crates/boyko_rhi_vulkan/src/rhi_impl.rs:80` (there is an equality
   `assert!(MAX_BIND_GROUP_BINDINGS == boyko_rhi::MAX_BIND_GROUP_BINDINGS)` at rhi_impl.rs:86-88 that keeps
   them in sync — bump both). This resizes every `[_; MAX_BIND_GROUP_BINDINGS]` inline array in the generic
   bind-group path automatically (`rhi_impl.rs:860,874,1052,1057,1062`). **Byte-neutral to output**: the
   software resolve still writes 19 descriptors (0..=18) with identical content; the 20th array slot is
   unused. The golden is a release build (debug_assert gone), so unaffected.
2. **Fix the exact-fill guard — keep it EXACT, do NOT relax to `<=` (critic P1-2).** The
   `debug_assert_eq!(entries.len(), MAX_BIND_GROUP_BINDINGS)` at `crates/boyko_rhi_vulkan/src/present/targets.rs:655-659`
   is a resolve UNDER-FILL tripwire (catches a missing binding); relaxing to `<= MAX` would let a buggy
   18-entry resolve pass silently. Instead pin to the resolve's own expected count via a local const
   (`RESOLVE_SOFTWARE_BINDINGS = 19`, `+1` for the HWRT variant): `debug_assert_eq!(entries.len(), if hwrt_variant { 20 } else { 19 })`.
   Debug-only; no golden impact. (This is the ONLY exact-fill `==MAX` site — no other bind group fills
   exactly 19, verified.)
3. **`crates/boyko_rhi/src/enums.rs`** — add `DescriptorKind::AccelerationStructure` with the exact value
   `1000150000` (`VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR`). **Explicit value-guard** (a `const _:
   () = assert!(DescriptorKind::AccelerationStructure as i32 == 1_000_150_000);`) — the R2a-1 lesson:
   raw-FFI RT enum VALUES need value-guards, not just layout pins. Extend `as_i32()`. NOTE: `1000150000`
   numerically coincides with `ST_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR` (accel_ffi.rs:85) but
   they live in DIFFERENT Vulkan enum namespaces (`VkDescriptorType` vs `VkStructureType`) — not a bug;
   add a code comment so a future reader doesn't "fix" the apparent duplicate (critic P2-1).
4. **`crates/boyko_rhi_vulkan/src/accel_ffi.rs`** — add `VkWriteDescriptorSetAccelerationStructureKHR`
   (`s_type` `ST_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR = 1000150007`, `p_next`,
   `acceleration_structure_count: u32`, `p_acceleration_structures: *const VkAccelerationStructureKHR`).
   abi_guard the layout (size/offset/align) + a value-guard on the sType (the R2a-1 sType-collision
   lesson — one wrong sType silently device-losts).
5. **`BindGroupEntry` + the write branch** — add a `BindGroupEntry::AccelerationStructure { accel: &'a
   A::AccelerationStructure }` variant to the generic entry enum at **`crates/boyko_rhi/src/device.rs:211-236`**
   (the `RhiApi::AccelerationStructure` assoc type exists since R1). Then, in `boyko_rhi_vulkan`:
   - **`bind_group_entry_kind()` (`rhi_impl.rs:118-125`)** — exhaustive match, map the new variant →
     `DescriptorKind::AccelerationStructure`.
   - **The descriptor-pool histogram — THREE coupled constants (critic P0-1; a 5-slot fixed array, missing
     any one fails to compile or panics at runtime):** (a) `KIND_COUNT` 5 → **6** (`rhi_impl.rs:975`);
     (b) `descriptor_kind_slot` (`rhi_impl.rs:104-112`) is an EXHAUSTIVE match with NO wildcard — add a 6th
     arm mapping `AccelerationStructure → slot 5` (else E0004); (c) `DESCRIPTOR_KIND_VK: [i32; 5]` →
     **`[i32; 6]`** (`rhi_impl.rs:94-100`) with `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR` (1000150000)
     at slot 5 (else the slot-5 index overflows `hist: [0u32; 5]` at rhi_impl.rs:976 → release OOB). Add a
     `const _: () = assert!(DESCRIPTOR_KIND_VK.len() == KIND_COUNT);`. This is 4a infra (compile-time only,
     no rendered-pixel effect).
   - **The write branch in `create_bind_group` (`rhi_impl.rs:1098-1170`, the `VkWriteDescriptorSet` init at
     ~:1160)** — the AS branch sets `descriptor_type: ACCELERATION_STRUCTURE_KHR`, `descriptor_count: 1`,
     `p_image_info/p_buffer_info: null`, and `p_next: &as_write` where `as_write` is a
     `VkWriteDescriptorSetAccelerationStructureKHR { acceleration_structure_count: 1,
     p_acceleration_structures: <stable ptr to the handle>, ... }`. **TWO lifetimes must be pinned to
     survive the SINGLE batched `vkUpdateDescriptorSets` at rhi_impl.rs:1183-1185 (critic P1-1) — the
     writes are built by `core::array::from_fn`, so ANY closure-local temporary dangles:**
     (i) a `[VkWriteDescriptorSetAccelerationStructureKHR; MAX_BIND_GROUP_BINDINGS]` scratch (parallel to
     `image_infos`/`buffer_infos`, populated at slot `i`, address-stable to the update) that `p_next` points
     into; AND (ii) the `p_acceleration_structures` target — read `&accel.handle` DIRECTLY from the borrowed
     `&'a BoundAccelStruct` (its handle outlives the call), OR a parallel `[VkAccelerationStructureKHR; MAX]`
     scratch; NEVER a copied local. A `// SAFETY:` note MUST name both lifetimes. This is the silent-FFI
     UAF class the R2a-4a GPU smoke exists to catch (abi_guard/Miri cannot see it).
   - Gate the Vulkan-side AS write branch `#[cfg(feature="hwrt")]` where it names AS types; the agnostic
     `DescriptorKind` + `BindGroupEntry` variants are ungated (boyko_rhi has no hwrt feature).
6. **A public AS-handle accessor** — `BoundAccelStruct.handle` is `pub(crate)` (`accel.rs:152`), reachable
   inside `boyko_rhi_vulkan` (the bind-group write path lives there, so it can read `handle` directly). No
   app-facing accessor needed if the resolve-set builder receives a `&BoundAccelStruct`.

## R2a-4a gates
- Byte-identical golden `58f6c6c3` hwrt-OFF **and** hwrt-ON (nothing binds an AS yet).
- clippy `-D warnings` ±hwrt; abi_guard + the new value-guards pass.
- **GPU smoke** (`#[ignore]`, mirror `hwrt_blas_smoke`): build a TLAS (R2a-3 path), create a 1-binding
  bind-group layout with `DescriptorKind::AccelerationStructure`, write the TLAS handle via the pNext path,
  bind it in a trivial compute dispatch, assert no device-lost + clean validation. This is the ONLY oracle
  for the AS-descriptor pNext write (the silent-FFI class).

---

# R2a-4b — the rayQuery shadow variant + routing + owner-eval (VISIBLE)

## Shader (crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl — HAND-HLSL, eDSL untouched)

The resolve is hand-HLSL; the eDSL owns only the `sdf_soft_shadow_ranged` leaf (lines 356-382), which is
NOT touched. The mesh-shadow term is at **line 1059**: `vis = min(vis, csm_visibility(P, n, csm_view_z,
NoL));` (vis starts as the SDF analytic term from `gMaterial.r`).

7. **`#if HWRT` at line 1059** — inside `#if HWRT`, replace `csm_visibility(...)` with an inline rayQuery
   trace against `tlas` (the new binding-19 `RaytracingAccelerationStructure`), using `GpuLight.dir_kind.xyz`
   (dir) + `CascadeData.split_far` (TMax) + `N*bias` origin offset. `#else` keeps the `csm_visibility`
   CSM path verbatim. Declare `[[vk::binding(19)]] RaytracingAccelerationStructure tlas;` under `#if HWRT`
   only. The `min`-combine + the SDF term are unchanged.
8. **Compile twice from the one `.hlsl`** (offline, hermetic, `C:\VulkanSDK\1.4.350.0\Bin\dxc.exe`):
   - Software (unchanged, byte-identical): `dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3
     deferred_pbr.hlsl -Fo deferred_pbr.comp.spv` (rayQuery behind undefined `HWRT` ⇒ identical to today).
   - HWRT: `dxc -spirv -T cs_6_5 -E main -D HWRT=1 -fspv-target-env=vulkan1.3 deferred_pbr.hlsl -Fo
     deferred_pbr_hwrt.comp.spv`.
   Add a second `SpirvBlob<N>` `include_bytes!` in `crates/boyko_rhi_vulkan/src/compute.rs` (beside
   `DEFERRED_PBR_SPV` @297) + `deferred_pbr_hwrt_spirv()`. Confirm the software `.spv` is byte-for-byte the
   committed 65456-byte file (re-hash it).

## Pipeline + descriptor variant (app + RHI)

9. **`crates/boyko_app/src/gpu_scene.rs:1568-1608`** — when `cfg!(feature="hwrt") && ctx.ray_query_enabled()
   && config.table[Shadow][Mesh]==HardwareTri`, build a SECOND resolve pipeline from `deferred_pbr_hwrt_spirv()`
   with a 20-entry layout (the 19 existing + binding 19 `DescriptorKind::AccelerationStructure`). Otherwise the
   software pipeline/layout is unchanged (byte-identical). **Store as ADDITIVE `Option` fields, NOT mutations
   (critic P1-3):** `GBufferScene` gains `#[cfg(feature="hwrt")] resolve_pipeline_hwrt: Option<&'a ComputePipeline>`
   (beside the existing single `resolve_pipeline` @scene_types.rs:944) — `None` ⇒ software ⇒ byte-identical.
   The layouts DIFFER (19 vs 20 bindings), so `resolve_pipeline_hwrt` must carry its OWN pipeline+layout.
10. **`crates/boyko_rhi_vulkan/src/present/targets.rs:561-691`** — build a SECOND per-FIF resolve set for the
    HWRT variant (a `#[cfg(feature="hwrt")] resolve_set_hwrt: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]>` on
    the targets struct — `None` on the software path, byte-identical), with a 20th
    `BindGroupEntry::AccelerationStructure` at binding 19 fed the stable per-FIF `PersistentTlas.accel` handle
    (plumb a `&BoundAccelStruct` per FIF onto the resolve-set-building scene struct). Built through the SAME
    `create_bind_group` path (so the histogram/pool handles the AS slot — critic OQ1). The AS handle is stable
    across frames (built-into, not recreated), so the once-per-FIF write model holds. **The record site
    (gbuffer.rs:1531-1547) selects the `(pipeline, layout, set)` TRIPLE together** when routing is Hardware —
    the layout at gbuffer.rs:1540 must swap in lock-step with the pipeline+set (a mismatch → device-lost).
11. **Routing** — `crates/boyko_render/src/ray_backend.rs`: fill `resolve_ray_backend` Weak/Strong arms
    (`:220-221`) → `c.table[Shadow][Mesh] = HardwareTri`. **Make the dormancy `debug_assert!` TIER-CONDITIONAL,
    do NOT delete it (critic P1-4):** `resolve_ray_backend_system` (`:272-278`) must still assert all-Software
    **when `caps.tier == RtTier::Absent`** (the byte-identity majority / non-RT path invariant still holds);
    HardwareTri is allowed only for `Weak`/`Strong`. The resolve becomes the FIRST consumer of
    `RayBackendConfig` — thread it (a `Res<RayBackendConfig>`) to where the resolve pipeline/variant is
    selected. `RayCaps` is boot-filled from `rt_tier()` (`runner.rs:159-160`), so on an RT GPU under
    `--features hwrt` the HardwareTri cell flows automatically.

## Barrier + record (crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs)

12. **Build→trace barrier** — insert `crate::accel::cmd_acceleration_structure_barrier(self.fns, cmd)`
    (`accel.rs:589`; `AS_WRITE→AS_READ`, stages `AS_BUILD→COMPUTE_SHADER`) **immediately after the
    `cmd_build_acceleration_structures` at `gbuffer.rs:317`**, inside the existing `#[cfg(feature="hwrt")]`
    + `if let (Some(t), Some(fns))` gate (so the tlas-OFF path stays byte-identical). This orders the R2a-3
    TLAS build against the resolve's rayQuery read (the TLAS backing is untracked by the framegraph, so a
    raw barrier is correct — no double-transition). The resolve dispatch is at `gbuffer.rs:1505-1548`;
    select the HWRT pipeline+set there when routing is Hardware.

## Owner-eval

13. **`cargo run -p boyko-app --example showcase --features hwrt`** with `BOYKO_HOST_DUMP=D:\tmp\engine_hwrt_shadow.bmp`
    + `BOYKO_DISABLE_VALIDATION=1` (windowed host, `host_dump.rs` env-gated readback; scene =
    mesh+SDF+CSM+point+spot). The HardwareTri routing arrives automatically via `RayCaps::new(rt_tier())`.
    The ORCHESTRATOR runs it (subagents hit os-740 on fresh GPU exes). Dump BOTH the HWRT render and the
    software render (config Software) for side-by-side. **The owner judges:** do the RT mesh shadows look
    correct (attached to geometry, no peter-panning / no acne), and how do they compare to the shadow-map?
    Tune `bias`/`TMin` on the owner's feedback (VALUES: bias tuned at eval).

## R2a-4b gates
- Degrade paths byte-identical `58f6c6c3`: hwrt-OFF (compiled out); hwrt-ON + ray_query absent (software
  pipeline); hwrt-ON + config Software (software pipeline). The software `.spv` + its 19-binding layout are
  untouched.
- clippy `-D warnings` ±hwrt; the sdf_field_edsl_sync + ddgi_probe_gi_sync tests still pass (the eDSL leaf
  is untouched).
- GPU: the HWRT showcase renders without device-lost; validation clean (or BOYKO_DISABLE_VALIDATION note).
- **OWNER-EVAL** = the gate. Commit the visible change only after owner OK.

## Risks
- **RISK-A (silent FFI):** the AS descriptor pNext write + the `VkWriteDescriptorSetAccelerationStructureKHR`
  sType — value-guard + the R2a-4a GPU smoke are the only oracles (no validation on this box by default).
- **RISK-B (byte-identity):** the `MAX_BIND_GROUP_BINDINGS` bump + the exact-fill guard must not perturb the
  software resolve. Re-hash the golden ±hwrt after 4a AND 4b.
- **RISK-C (self-shadow / bias):** rayQuery mesh shadows are prone to acne (TMin too small) or peter-panning
  (bias too large). Start `TMin=1e-3`, origin `+N*1e-3`; tune at owner-eval. This is the owner's visual call.
- **RISK-D (TMax / cascade):** `split_far` is view-space; confirm the ray's world-space TMax matches the
  intended cascade far (the ray is in world space; `split_far` is a view-Z distance — a directional shadow
  ray can use a large finite TMax or the scene bound; verify at eval).
