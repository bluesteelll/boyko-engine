# R2a-3 — GPU-resident, zero-CPU-pack per-frame TLAS build (HW-RT)

Status: **APPROVED (architect → critic → revision)**, coding-ready. Feature `hwrt` (default OFF).
Rung of `docs/RENDER-R2A-HWRT-SHADOWS-PLAN.md` (R2a-1 ✅ @7119a6f, R2a-2 ✅ @e2ba0df).

## Goal

Each frame, on the GPU, build a Vulkan TLAS from live M3 mesh-instance data. The
`VkAccelerationStructureInstanceKHR[]` array is **written by a compute shader** (zero
CPU-pack, zero readback — the engine's CPU-orchestrate / GPU-execute model, mirroring
`interp_instances.comp`), recorded into the single frame command buffer, one submit.
Gated `#[cfg(feature = "hwrt")]` + runtime `ctx.ray_query_enabled()`. Nothing traces the
TLAS yet (R2a-4). **Gate:** byte-identical `grand_showcase` golden
`58f6c6c3d986f7a393ea53b01c5021e7360cf6f1b32bf9db05d4d8bb98999dd5` with hwrt OFF, and
ALSO with hwrt ON (TLAS built + barriered but never sampled; resolve `.spv`/bindings
untouched).

**Targets:** pack = 1 dispatch, `ceil(N/64)` groups, 64 B/thread streaming write; TLAS
rebuild sub-ms (Ampere, ≤ 1024 instances); zero per-frame allocation; BLAS built once at
mesh registration.

## Principle 0

- **BLAS is per-mesh durable data → it lives ON `MeshGpu`** (the existing per-mesh
  ECS-owned record in the `NonSendResource` `MeshRegistry`), NOT a parallel `Vec<BuiltBlas>`.
- All device buffers (`blas_addr`, `mesh_id` ring, the `VkAccelerationStructureInstanceKHR[]`
  output, per-FIF TLAS backing/scratch) are GPU-contiguity buffers (the sanctioned FFI/GPU
  exception), owned by ONE named sub-struct `TlasResources` (mirrors `InterpGpuProd` /
  `CsmResources`) — a named owner is not a side store.

## Framegraph integration (the crux)

The renderer records EVERY barrier via `record_graph_pass` (framegraph-derived); a hand
`cmd_pipeline_barrier` next to an RDG-tracked resource double-transitions
(`present/passes/gbuffer.rs:800-801`). So the pack + build are **first-class framegraph
passes**, exactly like the interp / DDGI compute pre-passes.

**No sync-engine change is required.** `FrameGraph::buffer_access(res, stage: u32,
access: u32)` (`framegraph/graph.rs:203`) records raw Vk stage/access; `transition`
(`framegraph/sync.rs:241`) classifies WRITE vs READ purely by `access & WRITE_ACCESS_MASK`
and derives the barrier. Therefore:

- `VK_ACCESS_SHADER_READ_BIT` is a READ (`& WRITE_ACCESS_MASK == 0`).
- `VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR` (`ffi.rs:817`) is a valid
  `dst_stage` u32.
- The build's read of the instance array declared as `buffer_access(tlas_instances,
  AS_BUILD, SHADER_READ)` → the graph derives exactly the spec-confirmed
  `COMPUTE_SHADER/SHADER_WRITE → ACCELERATION_STRUCTURE_BUILD/SHADER_READ` barrier.
  **No `sync.rs` edit, no `WRITE_ACCESS_MASK` edit, no new barrier verb.**

**TLAS backing + scratch stay OUT of graph tracking** → the build's AS write is invisible
to the graph → `WRITE_ACCESS_MASK` is untouched (P0-2 avoided this rung; it becomes an
R2a-4 item only if the TLAS backing is ever graph-tracked — the R2a-4 build→trace barrier
is a raw barrier on the untracked backing, or track-it-then).

### New tracked resource + passes (`present/graph_bridge.rs`)

1. **`tlas_instances` buffer ResId** — the compute-written instance array; the ONLY new
   tracked resource. Declared under `#[cfg(feature = "hwrt")]`. Per-FIF frame-private
   (undefined seed — the pack fully overwrites `[0..count)` each frame).
   - Because the whole declaration is `#[cfg(feature = "hwrt")]`-gated, a `not(hwrt)` build
     has **unchanged ResIds → every existing framegraph test passes byte-identical**. Pick a
     self-consistent hwrt-build ResId layout (see RISK-2); the sink slot for the tlas
     instance array must resolve correctly for that layout.
   - On a tlas-off frame (count == 0, or ray_query absent) no pass declares
     `buffer_access` on it → the graph routes zero barriers naming it → OFF-path barrier
     arena byte-unchanged (the `optional_passes_are_additive` invariant,
     `framegraph_gbuffer_equiv.rs:439`).

2. **`tlas_pack` PassId** (after the interp pass, before the raster `begin_rendering`):
   - `buffer_access(interp_model_out, COMPUTE, SHADER_READ)` — **only when
     `scene.interp.is_some()`** (mirror the raster pass's conditional declaration,
     `graph_bridge.rs:441`). Derives interp(COMPUTE-WRITE)→pack(COMPUTE-READ) on the shared
     ring. When interp is OFF the ring is host-CPU-scattered into host-coherent memory and
     the submit's host-write→device domain dependency orders it (exactly as the raster VS
     reads the host-scattered ring today) → the pack declares ONLY the write below.
   - `buffer_access(tlas_instances, COMPUTE, SHADER_WRITE)` — the pack write.

3. **`tlas_build` PassId** (right after `tlas_pack`):
   - `buffer_access(tlas_instances, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR,
     VK_ACCESS_SHADER_READ_BIT)` — derives pack(COMPUTE/SHADER_WRITE)→build(AS_BUILD/SHADER_READ).
   - Does NOT touch TLAS backing/scratch (untracked).

`GbufferPassPlan` (`graph_bridge.rs:27`) gains `#[cfg(feature = "hwrt")] tlas_pack:
Option<PassId>, tlas_build: Option<PassId>` — `Some` iff `scene.tlas.is_some()` (the
"member is Some iff its body is recorded" invariant).

The sink `buffers` array grows by one; the tlas slot resolves to
`scene.tlas.map_or(NULL, |t| t.instance_array.buffer)` (NULL when off → never named by a
derived barrier → inert, byte-identical).

### Pass bodies (`present/passes/gbuffer.rs`, after interp ~:224, before raster ~:230)

```
#[cfg(feature = "hwrt")]
if let Some(t) = &scene.tlas {                       // armed only under hwrt + ray_query + count > 0
    self.record_graph_pass(plan.tlas_pack.expect(..), cmd, targets, scene, fi);  // interp→pack + pack-write barriers
    // bind pack pipeline + t.bind_group, push {count}, cmd_dispatch(ceil(count/64),1,1)   [raw dispatch]
    self.record_graph_pass(plan.tlas_build.expect(..), cmd, targets, scene, fi); // pack→build (SHADER_WRITE→SHADER_READ@AS_BUILD)
    // crate::accel::cmd_build_acceleration_structures(fns, cmd,
    //   &[AsBuildEntry{Tlas, geometry{vertex_data: t.instance_array_addr, primitive_count: t.count, ..}},
    //     scratch_address: t.scratch_addr}], &[t.dest])                          [raw build into UNTRACKED backing/scratch]
}
```

The **barriers** are graph-emitted; only the **GPU work** (dispatch, build) is raw — the
DDGI-update discipline (`gbuffer.rs:802`). No hand `cmd_pipeline_barrier` next to a tracked
resource.

## Data structures

```rust
// boyko_render/src/mesh_registry.rs
pub struct MeshGpu {
    pub vertex_buffer: BoundBuffer, pub index_buffer: BoundBuffer,
    pub index_count: u32, pub index_type: IndexType, pub vertex_count: u32,
    /// HW-RT R2a-3: per-mesh BLAS (Principle 0: durable per-mesh data ON the record).
    /// Built EAGERLY in `register_mesh` under `ctx.ray_query_enabled()`; freed FIRST in `destroy`.
    #[cfg(feature = "hwrt")]
    pub blas: Option<boyko_rhi_vulkan::accel_build::BuiltBlas>,
}

// boyko_app/src/gpu_scene.rs  (#[cfg(feature="hwrt")]) — named owner, mirrors InterpGpuProd
struct TlasResources {
    mesh_id_rings:   [BoundBuffer; FRAMES_IN_FLIGHT],   // CAP×4B, host-visible upload target; STORAGE
    instance_arrays: [BoundBuffer; FRAMES_IN_FLIGHT],   // CAP×64B, DEVICE-LOCAL; STORAGE|ACCEL_BUILD_INPUT|SHADER_DEVICE_ADDRESS
    tlas_backing:    [BoundBuffer; FRAMES_IN_FLIGHT],   // sized once for MAX; ACCEL_STRUCTURE_STORAGE|SHADER_DEVICE_ADDRESS  (UNTRACKED)
    tlas:            [BoundAccelStruct; FRAMES_IN_FLIGHT], // persistent, built-into each frame
    scratch:         [BoundBuffer; FRAMES_IN_FLIGHT],   // build_scratch(MAX)+align slack; STORAGE|SHADER_DEVICE_ADDRESS (UNTRACKED)
    blas_addr:       BoundBuffer,                        // MESH_ADDR_CAP×8B, HOST-VISIBLE u64 table (frame-invariant → single, no ring)  [RISK-3]
    pipeline:        ComputePipeline,
    layout:          VulkanBindGroupLayout,
    bind_groups:     [VulkanBindGroup; FRAMES_IN_FLIGHT], // { instances@0, mesh_ids@1, blas_addr@2, out@3 }
    instance_array_addr: [u64; FRAMES_IN_FLIGHT],       // cached once at create
    scratch_addr:        [u64; FRAMES_IN_FLIGHT],       // round_up(base, as_scratch_align) once at create
    capacity:        u32,                               // INSTANCE_CAPACITY (the sizing MAX)
    blas_addr_gen:   u64,                               // last MeshRegistry::blas_generation the table reflects
}

// boyko_rhi_vulkan/src/present/scene_types.rs  (#[cfg(feature="hwrt")])
#[derive(Clone, Copy)]
pub struct TlasBuildActivation<'a> {
    pub pipeline: &'a ComputePipeline, pub bind_group: &'a VulkanBindGroup,
    pub dest: &'a BoundAccelStruct,       // this slot's persistent TLAS (build target; UNTRACKED)
    pub instance_array: &'a BoundBuffer,  // the compute-written array (the sink's tlas slot)
    pub instance_array_addr: u64, pub scratch_addr: u64,
    pub count: u32,                       // host-known instance count ≤ capacity
}
// GBufferScene gains (LAST field): #[cfg(feature="hwrt")] pub tlas: Option<TlasBuildActivation<'a>>,
```

### `build_tlas_instances.comp.hlsl`

```hlsl
struct InstanceModelCol { float4 row0; float4 row1; float4 row2; };  // 48B, mirrors interp InterpModel
StructuredBuffer<InstanceModelCol> Instances : register(t0);         // instance_rings[fi]   (STORAGE)
StructuredBuffer<uint>             MeshIds    : register(t1);         // mesh_id_rings[fi]    (STORAGE SSBO, NOT Buffer<uint>)  [RISK P2-1]
StructuredBuffer<uint2>            BlasAddr   : register(t2);         // u64 as (lo,hi) by mesh_id (STORAGE)
RWByteAddressBuffer                OutInst    : register(u3);         // device-local 64B/instance
struct Push { uint count; };  [[vk::push_constant]] Push pc;         // 4 B
[numthreads(64,1,1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint i = tid.x; if (i >= pc.count) return;
    uint b = i * 64u;
    InstanceModelCol m = Instances[i];
    OutInst.Store4(b +  0u, m.row0); OutInst.Store4(b + 16u, m.row1); OutInst.Store4(b + 32u, m.row2); // transform 48B row-major verbatim
    OutInst.Store (b + 48u, (0xFFu << 24) | (i & 0x00FFFFFFu));       // mask=0xFF | customIndex=i
    OutInst.Store (b + 52u, 0u);                                      // sbtOffset=0 | flags=0
    uint2 a = BlasAddr[MeshIds[i]];
    OutInst.Store (b + 56u, a.x); OutInst.Store (b + 60u, a.y);       // accelStructRef = BLAS addr, LE lo@56/hi@60  [RISK P2-2: no uint64_t]
}
```

DXC `-T cs_6_0 -spirv -fspv-target-env=vulkan1.3` (SM6.0 — no `uint64_t`, no `shaderInt64`).
Pure hand-HLSL, NOT eDSL (a byte copy + bitfield pack, no transform math — sanctioned like
`interp_instances.comp`'s non-`interp_trs` body). Commit the `.spv` beside the `.hlsl`.

**`instanceCustomIndex = i` identity:** `i` = the pack dispatch thread index = the ring slot
index in `instance_rings[fi]` = the `mesh_ids[i]` index = the M3 gather draw-order slot. So
`customIndex` uniquely identifies the instance; R2a-4 hit-lighting resolves `(ring[i] affine,
mesh_ids[i] mesh)` from `customIndex` directly.

## Public API

```rust
// mesh_registry.rs
#[cfg(feature="hwrt")] pub fn blas_generation(&self) -> u64;         // bumped each register_mesh that built a BLAS
#[cfg(feature="hwrt")] pub fn blas_address(&self, h: MeshHandle) -> u64; // 0 if none
// register_mesh: builds the BLAS EAGERLY under ctx.ray_query_enabled() (no new public sig)
// destroy: frees mesh.blas first (unchanged sig)

// upload.rs
#[cfg(feature="hwrt")] pub unsafe fn upload_mesh_ids(token: &FrameWriteToken, slot: &BoundBuffer, scratch: &MeshRenderScratch);

// compute.rs
#[cfg(feature="hwrt")] pub fn build_tlas_instances_spirv() -> &'static [u32];
#[cfg(feature="hwrt")] pub const BUILD_TLAS_INSTANCES_PUSH_BYTES: u32 = 4;

// gpu_scene.rs — scene() gains one arg:  ..., tlas_enabled: bool, ...
```

## Cross-frame correctness

- Single-thread RHI; `BoundAccelStruct` `!Send`; no atomics.
- **Per-FIF duplication** of (mesh_id_ring, instance_array, scratch, tlas_backing).
  `drive_frame` waits slot `fi`'s `in_flight` fence (`present/frame_driver.rs:360`) BEFORE
  recording → slot `fi`'s previous-use GPU reads (pack read of its instance_array, build read
  of its scratch, future trace of its TLAS) all completed → the host writes/rebuilds slot
  `fi` race-free while the sibling frame uses the other slot. Same discipline `instance_rings`
  uses. `FRAMES_IN_FLIGHT = 2` (`present/mod.rs:69`).
- **`blas_addr`** is frame-invariant (BLAS never moves — spec) → written only when
  `blas_generation` advances (mesh registration), read-only during frames. Host-visible +
  host-coherent + submit domain dependency covers COMPUTE-read visibility (like the marcher's
  edit-list SSBO). `MESH_ADDR_CAP`-sized (boot cap, `debug_assert` on overflow, consistent
  with `INSTANCE_CAPACITY`).

## Sizing (size-with-MAX / build-with-actual)

`build_sizes(Tlas, geometry{ primitive_count: capacity = INSTANCE_CAPACITY })` ONCE at
`TlasResources::create`; size `tlas_backing[fi] = as_size`, `scratch[fi] = build_scratch +
as_scratch_align`. Per-frame `AsBuildEntry.geometry.primitive_count = t.count` (actual ≤
capacity), `debug_assert!(count <= capacity)`. (R2a-2 sized with actual; R2a-3 sizes with
MAX — VUID: the build's `primitiveCount` ≤ the count used for sizing.)
`scratch_addr[fi] = round_up(buffer_device_address(scratch[fi]), as_scratch_align)` cached
once. **Zero-instance:** `count == 0 ⇒ tlas_enabled == false ⇒ scene.tlas == None ⇒` neither
pass declared nor recorded (skip). R2a-4: "no TLAS this frame ⇒ trace treats the scene as
all-unshadowed."

## Implementation plan (ordered, file:line)

1. **`boyko_rhi_vulkan/shaders/build_tlas_instances.comp.hlsl` + committed `.spv`** — the
   ByteAddress packer above. DXC `-T cs_6_0 -spirv -fspv-target-env=vulkan1.3`, hermetic.
   Header: hand-HLSL, no eDSL.
2. **`compute.rs`** (near the `interp_instances_spirv` entry): `#[cfg(feature="hwrt")]
   BUILD_TLAS_INSTANCES_SPV: SpirvBlob<N>` (`include_bytes!` const-asserted length) +
   `build_tlas_instances_spirv()` + `BUILD_TLAS_INSTANCES_PUSH_BYTES = 4`.
3. **`accel.rs` / `accel_build.rs` — RISK-1 (index width):** extend `AsGeometryDesc` with an
   `index_type: AsIndexType` field (UINT16 | UINT32); `fill_geometry` (`accel.rs:203`, which
   currently hardcodes `VK_INDEX_TYPE_UINT32`) maps it to `VK_INDEX_TYPE_UINT16 =
   0`/`UINT32 = 1`; `BlasBuildInput` gains `index_type`; `build_blas` passes it through. The
   BLAS then reads the mesh's EXISTING index buffer at its real width — NO duplicate u32
   buffer (Principle 0, no parallel data; less VRAM). The mesh index buffer already carries
   `ACCEL_BUILD_INPUT|SHADER_DEVICE_ADDRESS` usage under hwrt for both widths (R2a-2). Update
   the R2a-2 `hwrt_blas_smoke` call sites to pass `index_type`.
4. **`mesh_registry.rs:77`** `MeshGpu`: add `#[cfg(feature="hwrt")] blas: Option<BuiltBlas>`.
   **`register_mesh` (:151):** after creating vtx/idx, under the SAME `#[cfg(feature="hwrt")]
   { if ctx.ray_query_enabled() {...} }` guard the `as_bits` uses (:172-185), call
   `build_blas(ctx, &ctx.rhi_queue(), &BlasBuildInput{ vertex_buffer, index_buffer,
   vertex_count, index_count, vertex_stride: VERTEX_STRIDE, index_type: mesh_index_type })` →
   store `Some(blas)`; else `None`. Bump `blas_generation`. **`destroy` (:363):**
   `#[cfg(feature="hwrt")] if let Some(b) = mesh.blas.take() { destroy_blas(ctx, b) }` BEFORE
   the vtx/idx `destroy_buffer` (P0-3, AS before backing, device-idle contract). Add
   `blas_generation` / `blas_address`.
5. **`upload.rs`** (mirror `upload_instance_models` :149): `#[cfg(feature="hwrt")]
   upload_mesh_ids` — `bytemuck::cast_slice(scratch.mesh_ids)` + bounds-assert +
   `copy_nonoverlapping`, same `FrameWriteToken` fence-proof; empty gather writes nothing.
6. **`scene_types.rs`** (near `InterpActivation`): `#[cfg(feature="hwrt")]
   TlasBuildActivation<'a>` + `#[cfg(feature="hwrt")] pub tlas: Option<...>` as the LAST
   `GBufferScene` field.
7. **`graph_bridge.rs`:** (a) `GbufferPassPlan` (:27) += the two `Option<PassId>`. (b)
   `declare_gbuffer_graph` (:279): under `#[cfg(feature="hwrt")]` add the `tlas_instances`
   buffer ResId + the `tlas_pack`/`tlas_build` passes with the accesses above, gated
   `scene.tlas.is_some()` — **RISK-2:** pick a self-consistent hwrt ResId layout; a `not(hwrt)`
   build must leave all existing ResIds unchanged (guaranteed by the cfg-gate). If a hwrt
   framegraph test exercises interp, adjust its ResId expectations; the `not(hwrt)` tests must
   be byte-unchanged. (c) grow the sink `buffers` array + resolve the tlas slot to
   `scene.tlas.map_or(NULL, |t| t.instance_array.buffer)`.
8. **`gbuffer.rs`** (after interp ~:224, before raster ~:230): the `#[cfg(feature="hwrt")]
   if let Some(t) = &scene.tlas` block — `record_graph_pass(tlas_pack)` → dispatch →
   `record_graph_pass(tlas_build)` → `crate::accel::cmd_build_acceleration_structures`.
9. **`gpu_scene.rs`:** `#[cfg(feature="hwrt")] TlasResources` (create / destroy / activation);
   built at the `GpuSceneBundles::boot` region via `TlasResources::create(device,
   INSTANCE_CAPACITY as u32)`; field in the `Self{}` literal; torn down in `destroy` (reverse
   order, before the instance rings). `scene()` += `tlas_enabled: bool`; arm `let tlas =
   tlas_enabled.then(|| self.tlas.activation(slot, count))`. On a `blas_generation` change,
   rewrite the host-visible `blas_addr` table (plain memcpy — RISK-3).
10. **`runner.rs`:** at the upload region (~:442) `#[cfg(feature="hwrt")] if
    ctx.ray_query_enabled() { upload_mesh_ids(&token, host.gpu.mesh_id_slot(s), scratch) }`;
    compute `tlas_enabled = cfg!(feature="hwrt") && ctx.ray_query_enabled() &&
    scratch.instance_count() > 0`; pass to `scene()`.
11. **const asserts:** `size_of::<VkAccelerationStructureInstanceKHR>() == 64` (R2a-1);
    `INSTANCE_MODEL_COL_BYTES == 48`; the `BUILD_TLAS_INSTANCES_SPV` length.

## Validation

**Framegraph unit tests (pure CPU, no GPU) — the byte-identity core:**
- `tlas_pack_build_derives_two_buffer_barriers` (mirror `interp_prepass_...`): a maximal
  frame + tlas asserts EXACTLY (1) `interp_model_out` COMPUTE(WRITE)→COMPUTE(READ) at pack
  [interp on], (2) `tlas_instances` COMPUTE(SHADER_WRITE)→AS_BUILD(SHADER_READ) at build; and
  no core-resource barrier perturbation.
- `tlas_off_path_zero_new_barriers` (mirror `optional_passes_are_additive`): tlas off ⇒
  `buf_barriers` unchanged.

**Host unit:** `upload_mesh_ids` bounds + empty no-op; `MeshGpu` / `TlasBuildActivation` size
asserts ±hwrt.

**GPU smoke (`#[ignore]`, mirror `hwrt_blas_tlas_smoke`):** 2 triangle BLAS; fill
`mesh_id`/`instance`/`blas_addr`; dispatch pack; barrier; build TLAS from the GPU-written
array; assert `device_address != 0` + `assert_validation_clean` (the pack-written 64-B
records are the only reflection-unverified surface → the smoke is the oracle).

**Byte-identity gates:**
- hwrt-OFF `cargo check --all-targets` → all R2a-3 items compiled out; golden `58f6c6c3`.
- hwrt-ON, ray_query present → golden `58f6c6c3` (TLAS built + barriered, never sampled).
- hwrt-ON, ray_query absent → mesh_id upload skipped, `tlas_enabled=false`, all `None` →
  identical.
- `clippy --all-targets -D warnings` ±hwrt (touch sources first — this box false-greens stale
  fingerprints).

## Residual risks & resolutions

- **RISK-1 (BLAS index width) — RESOLVED:** extend `AsGeometryDesc.index_type` (UINT16|UINT32);
  BLAS reads the mesh's existing index buffer at its real width; NO duplicate u32 buffer.
- **RISK-2 (ResId layout) — RESOLVED:** everything `#[cfg(feature="hwrt")]` → `not(hwrt)`
  ResIds unchanged, all existing tests byte-identical; hwrt layout is self-consistent + covered
  by the new hwrt tests.
- **RISK-3 (`blas_addr` residency) — RESOLVED:** host-visible (tiny frame-invariant table; a
  plain memcpy on generation change; host-coherent + submit domain dependency covers COMPUTE
  visibility). No staging, no barrier.
