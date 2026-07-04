# R2a — AS builder + HARD mesh RT shadows (HW-RT track, first hardware rung)

Converged build spec (architect wf + RT researcher wf, 2026-07-04). Rung R2a of
[RENDER-HYBRID-RAY-SYSTEM-DESIGN.md](RENDER-HYBRID-RAY-SYSTEM-DESIGN.md) §8. The FIRST rung that
enables real RT hardware. Preconditions M3 (@353dcf0), R0 (@1fdd99d), R1 (@a4b52c3) all shipped.
Full FFI struct detail lives in the architect design transcript; this doc is the strategic authority
+ the sub-rung split + gates + the VALUES decisions.

## The load-bearing fact
`InstanceModelCol.rows: [[f32;4];3]` (M3 per-instance 3×4 ROW-MAJOR world affine, 48 B, `Pod`) is
**byte-identical to `VkTransformMatrixKHR` `float[3][4]` row-major** → the TLAS
`VkAccelerationStructureInstanceKHR.transform` is a direct 48-byte memcpy from the M3 `ring[i].rows`;
`acceleration_structure_reference` ← `blas[mesh_ids[i]].device_address` (the M3 mesh-id lane). No
transpose, no repack. This match is the whole reason M3's mesh-id lane makes the TLAS build O(1).

## Build split — 4 sub-rungs (strictly dependency-ordered; R2a-1..3 byte-identical `58f6c6c3`)
| Sub-rung | Delivers | Gate | Pixel change |
|---|---|---|---|
| **R2a-1** | `feature=hwrt` + RT extension enable + presence → real `DeviceCaps.ray_query`; raw-FFI AS surface (`accel_ffi.rs` structs/PFNs + abi_guard); AS verbs on RhiDevice/RhiCommandEncoder (`#[cold]` erroring defaults, Mock-safe); `BoundAccelStruct` | compile ±hwrt green; abi_guard passes; hwrt-OFF ⇒ device-create bytes UNCHANGED; `ray_query==false` when ext absent | NONE (byte-identical) |
| **R2a-2** ✅ SHIPPED @e2ba0df | BLAS build (per-mesh triangle BLAS) + scratch lifecycle + gated mesh buffer-usage bits; a BLAS/TLAS **GPU smoke** (build+destroy a trivial TLAS, assert `device_address!=0`, no device-lost) | smoke PASSES on RTX 3060 (BLAS×2+TLAS distinct+non-zero addrs, scratch_align=128); non-hwrt byte-identical `58f6c6c3`; clippy-D±hwrt green | NONE (AS built, never traced) |
| **R2a-3** | per-frame TLAS **RDG pass** under `AsBuildSet`; instance buffer fed from M3 `ring`+`mesh_ids`; the `ACCELERATION_STRUCTURE_WRITE→READ` host barrier; resolve `.after_set(AsBuildSet)` (still samples shadow-maps) | TLAS rebuilt per-frame from live M3; frame clean; `tlas_refit_ms` acceptable; resolve `.spv` untouched ⇒ byte-identical | NONE (TLAS live, not read) |
| **R2a-4** | the `rayQuery` shadow variant (`deferred_pbr_hwrt.comp.spv`, `#if HWRT` one-body, +TLAS binding 19); variant+layout selected ONLY when hwrt+`ray_query`+`table[Shadow][Mesh]==HardwareTri`; the resolve routes its mesh-shadow term to the TLAS trace | **OWNER-EVAL** (RT mesh shadows vs shadow-map, RTX 3060); degrade paths byte-identical | YES (owner-eval) |

Why 4 not 2: separating the static BLAS build (R2a-2, proves the FFI *sequence* on HW) from the
per-frame TLAS data path (R2a-3, proves M3-ring→instance→build + ordering, pixels still frozen) means
R2a-4's shader arm sits on a fully-proven live TLAS — the only unproven thing left is the trace, which
IS the owner-eval surface. Each pre-shader rung is byte-identical → a regression is caught by the
golden without the owner in the loop.

## Byte-identity (the load-bearing safety) — two OFF paths textually = pre-R2a
1. **`feature=hwrt` compiled OFF (CI/golden box):** every AS FFI item, RT ext, feature-chain, `AccelFns`
   load, mesh usage bits, `DEVICE_ADDRESS` alloc flag, TLAS resources, the hwrt `.spv`+binding-19 are
   `#[cfg(feature="hwrt")]` → compile to nothing. `create_device` = the R1 extension array + `p_next`.
   `ray_query` literally `false`. Resolve = software layout + frozen `deferred_pbr.comp.spv`.
2. **hwrt ON but `ray_query` absent (non-RT GPU) / routed Software:** `supports_ray_query()==false` →
   `create_device` else-arm = R1 bytes; `resolve_ray_backend(Absent)==DISABLED` (all-Software) → software
   resolve variant. The mesh usage bits + alloc flag are ALSO runtime-gated on `ctx.device_caps().ray_query`
   → buffer bytes identical. The two-variant `.spv` split keeps the `RayQuery` capability out of the
   software module, so no software device rejects the pipeline.

## FFI surface (R2a-1) — mirror the R0 discipline (repr(C) + abi_guard size/value asserts + PFN typedefs)
New `accel_ffi.rs` (`#[cfg(feature="hwrt")]`): `VkAccelerationStructureKHR` handle;
`VkAccelerationStructureGeometryTrianglesDataKHR`/`GeometryKHR`/`BuildGeometryInfoKHR`/`BuildRangeInfoKHR`/
`BuildSizesInfoKHR`/`CreateInfoKHR`/`DeviceAddressInfoKHR`; `VkBufferDeviceAddressInfo`; and the
ABI-critical **`VkAccelerationStructureInstanceKHR` (64 B packed): `transform:[[f32;4];3]`@0 (48 B,
row-major) + `instance_custom_index_and_mask:u32`@48 (customIndex:24|mask:24..31) +
`instance_sbt_offset_and_flags:u32`@52 + `acceleration_structure_reference:u64`@56**. abi_guard: all 4
offsets + size==64 + align==8 + the cross-crate `[[f32;4];3]==48B` bridge to `InstanceModelCol.rows`.
sType numeric values + `RAY_FLAG` combo confirmed by the RT researcher (the two silent-error spots).
Verbs (RhiDevice): `get_acceleration_structure_build_sizes`/`create_acceleration_structure`/
`get_acceleration_structure_device_address`/`get_buffer_device_address`/`destroy_acceleration_structure`;
(RhiCommandEncoder): `cmd_build_acceleration_structures`/`cmd_acceleration_structure_barrier`. Agnostic
POD descriptors in `boyko_rhi/descriptor.rs` (no `Vk*` leak). `BoundAccelStruct{handle,buffer,device_address,kind}`.
Enable: `VK_KHR_acceleration_structure`+`VK_KHR_ray_query`+`VK_KHR_deferred_host_operations` +
`bufferDeviceAddress` (Vk1.2 core feature); pNext chain `AccelerationStructureFeaturesKHR`→
`RayQueryFeaturesKHR`→`Vulkan12Features`; `supports_ray_query()` presence+feature query →
`DeviceCaps.ray_query` (+ `as_scratch_align` from `AccelerationStructurePropertiesKHR`). NO ray-tracing-pipeline
(inline rayQuery needs no RTPSO/SBT).

## The shader variant (R2a-4) — protect the frozen golden
Two `.spv`: software `deferred_pbr.comp.spv` (DXC invocation UNCHANGED → `58f6c6c3`) + hwrt
`deferred_pbr_hwrt.comp.spv` (`-D HWRT=1`, SM6.5, `-fspv-target-env=vulkan1.3` → `SPV_KHR_ray_query`).
ONE parameterized `mesh_shadow()` body: `#if HWRT` → `RayQuery<ACCEPT_FIRST_HIT_AND_END_SEARCH|
SKIP_PROCEDURAL|CULL_NON_OPAQUE>` single occlusion ray toward the light, `CommittedStatus()==TRIANGLE_HIT
? 0 : 1`; `#else` → the verbatim frozen CSM PCF term. TLAS = new binding 19 (`RaytracingAccelerationStructure`),
referenced ONLY in `#if HWRT` → absent from the software `.spv`/layout. New `DescriptorKind::AccelerationStructure`
(`VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR=1000150000`) written via `VkWriteDescriptorSetAccelerationStructureKHR`
pNext. Host picks the variant+layout at resolve-pipeline build via one `if` (hwrt && ray_query &&
table[Shadow][Mesh]==HardwareTri). The mesh-shadow term is already hand-HLSL (CSM), so NO eDSL change,
no byte-identity risk to eDSL-owned paths.

## VALUES decisions (orchestrator-decided per "decide perf/architecture yourself"; owner adjusts at R2a-4 eval)
- **#A shadow combine:** the HWRT term REPLACES the CSM mesh-shadow term and stays `min`-combined with the
  analytic SDF soft-shadow (preserves mesh∪SDF union). Owner can request a `min(rt,csm)` blend at eval.
- **#B shadow-ray bias:** default `SHADOW_BIAS ≈ 1e-3·scene_scale` (normal offset) + `TMin ≈ 1e-3` (along L);
  TUNED at the first owner-eval (acne vs peter-panning), mirroring `CSM_NORMAL_BIAS`.
- **#C TLAS rebuild-vs-refit:** REBUILD every frame in R2a (simplest, correct for a changing instance count,
  sub-ms on Ampere for hundreds of instances). Refit (`ALLOW_UPDATE`) is R6.
- **#D:** software byte-identical always; HW owner-eval opt-in; both backends game-dev-selectable (RESOLVED).
- `instanceCustomIndex = i` (instance index — free, R4 hit-lighting needs it).

## Top RISKS (full list in the architect transcript)
InstanceKHR ABI/row-major (abi_guard offsets + the 48B bridge + smoke) · `SHADER_DEVICE_ADDRESS` needs
`VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT` on the backing memory (gated suballocator p_next; assert addr!=0) ·
scratch alignment (`as_scratch_align`) + build-fence order · BLAS device-address lifetime (static, never moved) ·
frozen-golden perturbation from the variant (the `#if HWRT` split + CI golden guard) · non-RT-GPU degrade
(ray_query=false → all-software) · the box's validation-crash-proneness (`BOYKO_DISABLE_VALIDATION=1`; abi_guard
+ `device_address!=0` + fail-fast caps are the primary oracle) · DXC SM6.5 + vulkan1.3 for rayQuery ·
subagents can't run fresh GPU exes (os-740) → the smoke + owner-eval run through the orchestrator/owner.

## Files per sub-rung (dependency order) — see the architect transcript §8 for the full lists
- R2a-1: Cargo `hwrt` feature · `accel_ffi.rs`(new) · `ffi.rs`(consts) · `abi_guard.rs` · `device.rs`(enable+PFN+caps) ·
  `boyko_rhi/descriptor.rs`(AS descs) · `boyko_rhi/{device,encoder}.rs`(verbs) · `accel.rs`(new, BoundAccelStruct+impls) · `handle.rs`(Mock=()).
- R2a-2: `enums.rs`+`ffi.rs`+`abi_guard.rs`(usage bits) · `memory.rs`(DEVICE_ADDRESS alloc flag) ·
  `mesh_registry.rs`(gated usage + MeshBlasTable + build_mesh_blas) · `accel_build.rs`(new) · gated `#[ignore]` GPU smoke.
- R2a-3: `accel_build.rs`(per-frame TLAS fill) · `gpu_scene.rs`(TlasResources) · host(`build_tlas_system` in AsBuildSet +
  resolve after_set + the AS barrier) · `ray_plugin.rs`(register).
- R2a-4: `deferred_pbr.hlsl`(#if HWRT + gTlas) · `deferred_pbr_hwrt.comp.spv`(new committed) ·
  `enums.rs`+`ffi.rs`+`abi_guard.rs`(AS DescriptorKind) · `rhi_impl.rs`(AS-descriptor pNext write) ·
  `ray_backend.rs`(real resolve arms, remove R1 tripwire) · `gpu_scene.rs`(variant/layout select if).

**Gate cadence:** each sub-rung — `cargo check --all-targets` ±hwrt + `clippy -D warnings` (touch first) + the
grand_showcase golden `58f6c6c3` (R2a-1..3 byte-identical) → R2a-4 owner-eval. Windowed dumps `--test-threads=1`,
`BOYKO_DISABLE_VALIDATION=1`.

## R2a-2 live-run lesson (the smoke's payoff — an R2a-1 bug HW-only could surface)
`AccelFns::load` resolved `vkGetBufferDeviceAddress**KHR**` → **NULL on the RTX 3060**: we enable the
CORE Vulkan 1.2 `bufferDeviceAddress` FEATURE, not the `VK_KHR_buffer_device_address` EXTENSION, so
the `*KHR`-suffixed command alias is never exported (only the extension exports it) — the whole AS
command table failed to load, `DeviceCaps.ray_query` never latched, and HW-RT was silently disabled
(the smoke SKIPped as "non-RT GPU" on an RT GPU). **FIX**: resolve the CORE `vkGetBufferDeviceAddress`
first, `*KHR` as fallback. **GENERAL RULE for the rest of the track**: a feature promoted to core and
enabled via the core bit exports the CORE command name — resolve that, not the `*KHR` alias. This is
a THIRD silent class (after wrong sType values, wrong enum values): wrong *command name* — and it is
invisible to `cargo check` AND to the no-validation box; the live GPU smoke is the only oracle.

## Research-confirmed constants (2 RT researcher passes, cross-validated vs Khronos + nvpro/Vulkan-Samples)
- **Enablement chain:** enable the 3 NON-core extension STRINGS `VK_KHR_ray_query` + `VK_KHR_acceleration_structure`
  + `VK_KHR_deferred_host_operations` (deferred-host-ops is a DECLARED dependency of acceleration_structure — must be
  enabled even though a GPU-build path never calls its API; omitting it fails device create). The transitive deps
  `buffer_device_address` / `descriptor_indexing` / `spirv_1_4` / `shader_float_controls` are ALL **core in Vulkan 1.2**,
  and the device targets 1.3 → available WITHOUT enabling their extension strings; enable the `bufferDeviceAddress`
  FEATURE bit (in `VkPhysicalDeviceVulkan12Features` or the standalone `BufferDeviceAddressFeatures`). pNext feature
  chain into `VkPhysicalDeviceFeatures2`: `BufferDeviceAddressFeatures{bufferDeviceAddress=TRUE}` →
  `AccelerationStructureFeaturesKHR{accelerationStructure=TRUE}` → `RayQueryFeaturesKHR{rayQuery=TRUE}`. Presence-query
  via `vkGetPhysicalDeviceFeatures2` with the same chain (mirror `supports_dynamic_rendering`); absent → `RtTier::Absent`,
  never boot-fail. NOT `VK_KHR_ray_tracing_pipeline` (inline rayQuery needs no RTPSO/SBT).
- **`VkAccelerationStructureInstanceKHR` (both researchers confirm):** 64 B, align 8; `transform`@0 (48 B, `float[3][4]`
  ROW-MAJOR, translation in column 3 = `m[r][3]`); word0@48 = `customIndex:24`(LSB) | `mask:8`(MSB); word1@52 =
  `sbtOffset:24`(LSB) | `flags:8`(MSB); `accelerationStructureReference:u64`@56 = the BLAS **device address** (NOT the
  handle). Pack the bitfields as raw u32 (Rust has no C bitfields): `word0 = (custom & 0xFFFFFF) | (mask<<24)`.
- **⚠️ ROW-MAJOR VERIFY (the #1 raw-FFI RT bug — resolve at R2a-3):** the architect read `InstanceModelCol.rows` as
  3×4 ROW-major (→ direct 48-B memcpy). One researcher warns `boyko_math` is typically COLUMN-major (→ transpose the
  upper 3×4 before the instance write). **The developer MUST confirm `InstanceModelCol`'s actual row/column convention
  from `instance_model.rs` + the instanced-VS usage before the R2a-3 TLAS fill** — a wrong convention yields shadows
  detached from geometry (caught by the R2a-2 smoke + R2a-4 owner-eval, but verify upfront). abi_guard the 48-B bridge.
- **Scratch:** `scratchData.deviceAddress` MUST be aligned to `minAccelerationStructureScratchOffsetAlignment` (=128 on
  Ampere/RTX 3060; query from `VkPhysicalDeviceAccelerationStructurePropertiesKHR` — do NOT trust buffer memreq alignment).
  Buffer triple-gate: `bufferDeviceAddress` feature + `SHADER_DEVICE_ADDRESS_BIT` usage + `VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT`
  on the allocation (miss any → address is garbage). AS backing: `ACCELERATION_STRUCTURE_STORAGE_BIT_KHR|SHADER_DEVICE_ADDRESS_BIT`;
  scratch: `STORAGE_BUFFER|SHADER_DEVICE_ADDRESS`; instance + mesh vtx/idx: `ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR|SHADER_DEVICE_ADDRESS`.
  `maxVertex = vertexCount-1`; `primitiveCount = indexCount/3`.
- **rayQuery shadow (R2a-4):** HLSL `RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH|RAY_FLAG_FORCE_OPAQUE|
  RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES> q; q.TraceRayInline(tlas,0,0xFF,ray); q.Proceed(); return
  q.CommittedStatus()==COMMITTED_TRIANGLE_HIT;` (hit ⇒ occluded). `RayDesc.TMin ≈ 1e-3` bias + origin `+N*1e-3`;
  `TMax = light distance`. DXC `-T cs_6_5 -spirv -fspv-target-env=vulkan1.3` → emits `SPV_KHR_ray_query`+`RayQueryKHR`
  cap (SM6.5 mandatory; DXC ≥1.7). AS descriptor: `VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR`, written via
  `VkWriteDescriptorSetAccelerationStructureKHR` in `VkWriteDescriptorSet.pNext` (NOT pImageInfo/pBufferInfo).
  Build→trace barrier: `PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR`/`ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR`
  → `COMPUTE_SHADER`/`ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR`.
- **eDSL note (R2a-4):** the mesh-shadow term (CSM `SampleCmpLevelZero`) is already HAND-HLSL, NOT eDSL-owned, so the
  `#if HWRT` rayQuery arm is added there without violating the eDSL-authors-shaders agreement (the eDSL owns the SDF
  field/marcher spans only). Confirm at R2a-4.
