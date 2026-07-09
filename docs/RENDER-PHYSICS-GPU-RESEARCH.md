# Render / Physics / GPU-ECS — Research Digest (7 web-verified streams)

> **Status: RESEARCH DIGEST (input to the unified implementation plan).** Branch `ecs`, 2026-06-16.
> Distilled by the orchestrator from 7 deep, web-verified research streams (each claim was tagged
> verified/partial/unverified by its researcher; this digest carries the load-bearing conclusions +
> key numbers + the crystallized unified direction). The seed reference is
> [docs/sdf-engine-architecture.md](sdf-engine-architecture.md) (hybrid mesh+SDF §15/§16, already thorough).
>
> **Steering principles (user, binding):**
> - **Ultimate design by FIRST-PRINCIPLES PERFORMANCE.** "No one ships it" is a RISK SIGNAL, not a
>   disqualifier. SDF is niche today — we go to the frontier deliberately. Reason perf with numbers.
> - **NO third-party crates anywhere in the graphics/core stack** — in-house RHI + RAW hand-declared FFI.
> - **Native, DX-class API** (NOT web/WebGPU). First backend = **Vulkan**; DX12/Metal later behind the RHI.
> - **Foundation/base only now** — extensible seams, not the full engine.

---

## 0. The crystallized unified direction (the headline)

Two independent re-runs (GPU-ECS-v2 + SDF-v2) **converged** on one architecture, and the integration
stream confirmed it fits boyko's existing ECS with minimal new machinery:

> **One GPU-resident, zero-readback, GPU-driven store on a single Vulkan device, where SDF bricks/edits
> and the simulated ECS component columns are BOTH device-local SSBO "columns" of GPU-resident
> archetypes — sharing one allocator, one scheduler, one command stream. The CPU is the conductor
> (owns structure, branchy/heterogeneous/low-N logic, scheduling); the GPU owns the homogeneous large-N
> data-parallel mass (sim systems, SDF eval/regen, particles) and never round-trips per-entity data back.**

This is unprecedented *as a unified ECS* (Svelto is the closest, synchronous, prototype-grade precedent),
but **every constituent mechanism is independently verified in production** (AMD Brixelizer, Unity DOTS
persistent GPU store, GPU-driven rendering, GPU particle systems). Per the steering principle, the novel
*combination* is exactly what we want; the foundation builds the **seam**, not the whole GPU world.

---

## 1. RHI + raw Vulkan (in-house graphics layer)

- **In-house RHI trait seam** modeled on **wgpu-hal's `Api` associated-types shape** (study, don't depend):
  `Instance→Adapter→Device+Queue; Buffer/Texture/Sampler/ShaderModule; PipelineLayout+BindGroup;
  Render/ComputePipeline; CommandEncoder (render/compute pass, draw, dispatch, EXPLICIT barriers);
  Surface; Fence/Semaphore`. Validation pushed to the caller (zero overhead). Static dispatch, not object-safe.
- **Resource handles = generational indices** (reuse `boyko_utils::SparseSlotMap`/`Slot`), NOT `Arc`, NOT
  raw owning pointers. ABA-safe, `Copy`, zero-heap — consistent with `EntityMaster` generations.
- **Vulkan-first** (first-principles): flat C ABI (dispatchable = opaque ptr, non-dispatchable = u64 on
  64-bit → `#[repr(transparent)]` newtype) vs DX12's COM vtable+AddRef/Release+GUID (materially harder to
  hand-bind); ONE backend covers Win+Linux. Bootstrap: `LoadLibrary`/`dlopen` + `vkGetInstanceProcAddr`
  (3 tiers: global/instance/device; use `vkGetDeviceProcAddr` for the hot path) — exactly the
  `crates/boyko_ecs/src/ecs/memory/vm.rs` raw-`extern`+`// SAFETY`+cfg-per-OS precedent.
- **Shaders = offline SPIR-V, zero runtime dep:** `glslangValidator`/`dxc` as BUILD-TIME tools (a `build.rs`
  or committed `.spv` blobs) + `include_bytes!` (⚠️ align to `u32`: wrap in `#[repr(C, align(4))]`).
  **HLSL-via-DXC recommended** (gives a free future DX12 backend; DXC = "most complete" SPIR-V). Reject
  runtime shaderc/naga. Own shader-language→SPIR-V compiler = out of scope (multi-year).
- **Vulkan 1.3 dynamic rendering** (`vkCmdBeginRendering`) — deletes the entire `VkRenderPass`/`VkFramebuffer`
  object class (biggest boilerplate cut). `renderPass = VK_NULL_HANDLE` on pipelines.
- **HIGHEST-RISK item — hand-written `VkDeviceMemory` sub-allocator** (VMA forbidden):
  `maxMemoryAllocationCount` is ≥4096 so you cannot allocate per-resource; memory-type selection from
  `VkPhysicalDeviceMemoryProperties` is non-trivial. **Reuse the `MemFreeBlockMaster`/free-list discipline
  already proven in the ECS.** Buffer Device Address (core 1.2) gives raw GPU pointers for linked
  structures (edit-list / AABB-tree nodes).
- **Effort honesty:** a minimal correct Vulkan backend is **multiple thousand lines of `unsafe`**, but
  bounded + mechanical (`vm.rs`-style declarations + `#[repr(C)]` struct-filling), not algorithmic.
- **Classic descriptor sets** for the foundation; bindless/descriptor-indexing (core 1.2, SoA-aligned) later.
- **Open decisions:** HLSL vs GLSL source; `build.rs`-invokes-toolchain vs committed `.spv`; sub-allocator
  algorithm (ring for streaming uploads vs free-list for long-lived); drop `winit`/`eframe` for raw
  Win32/XCB windowing or keep a windowing dep?; confirm the NULL-instance-queryable fn set vs the spec.

## 2. GPU-ECS (v2 overturns v1 under the new inputs)

- **v1 (precedent-grounded) said "full GPU ECS not worth it"** because it assumed wgpu/WebGPU limits and
  used "Svelto only got 1.15×, nobody ships it" as a disqualifier. **Both premises void now** (native +
  novelty-OK). Svelto's weak result was **readback-bound** (`ReadBackBuffer` round-trip), not compute.
- **Three regimes, first-principles cost:** (A) CPU-ECS: stream `N·B` at CPU-DRAM bandwidth, burns cores.
  (B) upload/compute/readback hybrid: a `2·N·B` PCIe tax/frame (~25-30 GB/s one-way, the Svelto trap).
  (C) **GPU-resident, zero-readback: stream `N·B` at VRAM bandwidth (hundreds of GB/s–>1 TB/s), ~zero CPU.**
  **Regime C dominates A by the VRAM/DRAM ratio and B by eliminating the PCIe round-trip.** Per-frame CPU
  cost collapses to recording a tiny command buffer + a ~1-2 µs doorbell.
- **GPU-driven sizing:** `vkCmdDispatchIndirect` reads group counts the DEVICE wrote → **edit/entity count
  DECOUPLES from dispatch count** (millions of entities → ~one dispatch of ~1000 groups × 1024 invocations).
- **Structural ops on GPU:** spawn = atomic bump / dead-list (one `atomicAdd`; subgroup reduction → one
  atomic per subgroup, "do fewer atomics"); despawn = stream compaction (flag → **decoupled-lookback
  prefix scan ~2n traffic, ~memcpy throughput** → scatter). **Archetype moves = the wall** (scatter
  across column sets, serializes) → keep on CPU; GPU-residency is optimal when **archetype membership is
  STABLE** and only values + intra-archetype spawn/despawn churn (the homogeneous large-N case).
- **The wall is branchy/irregular/cross-entity/low-N logic** (gameplay, AI, net, editor) — keep CPU; a
  round-trip to ask the GPU a branch (~1-2 µs) ≫ a CPU branch (~1 ns); GPU also pays SIMT divergence.
- **CPU/GPU boundary = a POPULATION partition, not a code partition:** an archetype is CPU- or
  GPU-resident; its columns live in DRAM or VRAM; its systems are CPU fns or compute dispatches.
- **Verified scale anchors:** MPM/PIC-FLIP ~1M particles ~50 fps GPU-resident (Titan X); GPU-driven
  rendering submits "a couple draws," GPU does culling+indirect; 1024 invocations/workgroup.
- **Build order:** `GpuColumn` (residency mirror of `ComponentPool`, `std430` SSBO, `base+i*stride`) →
  `GpuSystem` node in the existing conflict graph → lock zero-readback steady state → indirect dispatch +
  atomic spawn/dead-list → stream-compaction despawn → archetype-moves LAST (likely permanent CPU seam).
- **Soundness caveat:** the GPU half is **NOT Miri-checkable** (VRAM mapping, BDA pointer math, GPU↔CPU
  shared buffers, syscalls) — validation shifts to Vulkan validation layers + golden-state/golden-image tests.

## 3. SDF render + compute (v2: fully GPU-resident, GPU-driven)

- **Goes far past v1's "5-piece foundation"** (which kept a CPU edit-list + CPU regen). The ultimate is a
  **fully GPU-resident, GPU-driven SDF world**: edit-list, brick free-list, AABB tree, atlas all
  device-local; the **entire per-frame brick lifecycle (mark-dirty → allocate → voxelize/JFA → free →
  AABB-rebuild) on GPU via indirect dispatch, zero CPU round-trip per edit.**
- **Production precedent (NOT speculative): AMD FidelityFX Brixelizer** — compute-only, GPU-resident;
  **brick = 8×8×8 = 512 brixels, R8_UNORM (~512 B/brick)** in a shared `Texture3D` SDF atlas; geometry
  binned into **cascades of 64³ voxels (a clipmap)**, one brick per surface-intersecting voxel; traversed
  via a **3-level AABB tree + brick map per cascade**; **static bricks persist, dynamic bricks re-alloc
  every frame**; voxel-size is the cost knob (0.3→0.6 m ≈ 8× fewer bricks).
- **Exotic lever (Aokana, the precedent v1 avoided):** **SVDAG** static far-field + **Hi-Z 8×8-tile
  culling** + **64-bit visibility buffer**; 10-billion-voxel scenes @ ~6 ms, **2-4× faster than HashDAG**,
  ~5% of data VRAM-resident. → **Hybrid: SVDAG static far field + mutable bricks near field** (DAG sharing
  breaks on edit, so DAG is far-field-only).
- **First-principles wall = regen bandwidth + grazing-ray divergence, NOT raymarch throughput or storage.**
  Atlas is cheap (~24 K bricks ≈ 12 MiB for a typical scene). Regen: 512 B/brick write × JFA(3 passes×27
  samples) or voxelize; **~1 M brick regens/frame ≈ the 16 ms edge**. **Breakthrough = dirty-region
  scoping:** regen ONLY bricks whose AABB an edit overlaps, via a GPU dirty-list + indirect dispatch sized
  by an atomic counter → **edit count decouples from dispatch count (~6-8 indirect dispatches/frame
  regardless of millions of edits)**. Prefer **JFA-refine** (fixed, geometry-independent) over re-voxelize.
- **⚠️ CRITICAL Vulkan limit:** guaranteed `maxImageDimension3D` is only **256 (core) / 512 (1.4)** — the
  **2048³ atlas is a desktop DE-FACTO value, NOT a spec guarantee**. **Query it; design the atlas as a
  POOL of 3D textures / bindless array**, not one monolith. (This also bounds DX12: D3D12 cap is 2048.)
- **Hybrid mesh+SDF (verified, Interplay-of-Light 2017):** rasterize meshes into a deferred G-buffer, then
  an SDF raymarch pass that **writes NDC depth into the SAME Z-buffer** → correct mesh↔SDF occlusion for
  free; **two-pass HZB / Hi-Z tile culling** (Nanite/Aokana) lets the SDF raymarch skip mesh-occluded
  tiles. SDF gives soft shadows + AO ~free via cone tracing. Mesh-with-SDF-holes: bind-space cutting +
  JFA-baked interior SDF + stencil-restricted raymarch of `subtract(charSDF, cutterSDF)` (§15.2 of the seed).
- **Foundation slice (ultimate-in-miniature, NOT a CPU-mirror):** device-local SDF brick atlas + GPU
  free-list buffer/atomic-counter; GPU edit-list buffer (BDA-addressable); ONE brick-regen indirect
  dispatch (voxelize first, JFA later); ONE 64³ clipmap cascade + flat brick map; fullscreen/tiled raymarch
  writing NDC depth + a 64-bit visibility buffer composited with the mesh G-buffer.
- **SDF+ECS unify:** the brick map + edit-list are **literally ECS columns on a device buffer**; brick
  lifecycle = an ECS "system" of indirect dispatches. One device, one allocator, one scheduler.

## 4. Physics (hybrid; SDF-native in-house + swappable rigid solver)

- **Routing thesis VERIFIED in production:** SDF is used as a **particle/continuum collision** rep (Unity
  VFX Graph "Collide with SDF", Houdini); mature rigid solvers (Jolt/PhysX/Rapier/Chaos) do **not** ingest
  SDF natively. "SDF decides WHERE the cut passes; the rigid solver decides HOW debris behaves."
- **SDF collision math verified:** distance = penetration depth, gradient = contact normal. Great for the
  one-sided many-points case (particles/soft/fluid); does **nothing** for many-body rigid contact *resolution*.
- **Solver landscape (verified):** Jolt = sequential-impulse + warm-start, island-parallel, lock-free
  broadphase, **4.9×@8t / 5.7×@16t**, AAA-shipped, MIT, no SDF shape. **Rapier+parry** = Rust-native,
  **TGS-Soft** + warm-start, BVH broadphase, **HAS a `Voxels` collider** (the only documented Rust
  "SDF-ish" adapter), rayon-parallel (its own pool — coordination cost vs `boyko_threadpool`), Apache-2.0.
  Modern solvers converged on **substeps + soft constraints** (Box2D v3 "Soft Step" = the best free
  primary; Catto: "smaller time steps beat more iterations").
- **Collision-mesh-from-SDF (in-house, on `boyko_threadpool`):** Marching Cubes (per-cell parallel, manifold,
  every solver accepts it) for low-res collision; incremental regen tied to the **dirty-brick set** (same
  BVH scheme as rendering). Dual Contouring only if sharp-edge collision fidelity matters.
- **Continuum (in-house SDF-native edge):** XPBD (compliance α, timestep-independent stiffness; unifies
  rigid+soft) and MPM/PB-MPM for melt/fluid/smear; "cutting" = bulk-despawn particles/tets where
  `cutterSDF < 0` (the ECS `Commands` despawn pattern). Body-splitting = connected-components/flood-fill
  on the field (body COUNT change = an ECS structural op, the expensive part).
- **THE decision (matrix, surfaced):** custom solver (zero-dep, but loses on rigid-shatter for years) vs
  Rapier (Rust dep, voxel collider, rayon) vs Jolt FFI (best/mature, C++ FFI). **Foundation posture:**
  build the SDF-native paths in-house (real edge), put the rigid solver behind a **swappable trait seam**
  with the **manifold (points/normal/penetration) as the universal currency**; first backend = a minimal
  in-house Soft-Step solver (proves the seam, zero-dep) OR wire Rapier. Determinism needs a **stable apply
  order** (ties to the existing apply-window barrier). "Shared BVH" = share the CODE, not the instance
  (physics broadphase indexes collider AABBs; the SDF-edit BVH indexes edits — different leaves).
- **Per the ultimate/first-principles steer:** compute whether a **unified GPU-resident SDF + particle +
  XPBD/MPM continuum solver** (where SDF dominates) beats classical CPU islands+sequential-impulse, with
  rigid bodies as a special case — choose the DEFAULT by perf, keep the seam swappable regardless.

## 5. Integration into the ECS (how it all fits — minimal new machinery)

- **The Schedule ALREADY hosts GPU compute systems.** A GPU system is a `System` whose `Access`
  (`ComponentMask` read/write bitsets) feeds the **existing `ConflictGraph`** — disjoint columns →
  concurrent, same column → serialized. **The new idea: a conflict edge between GPU systems lowers to a
  `vkCmdPipelineBarrier` (RHI `transition_buffers`) instead of CPU happens-before.** The **apply-window
  barrier (`running==0`, dispatcher holds `&mut EcsMaster`) is the natural CPU↔GPU submit/sync point.**
  Reuse ONE graph algorithm for both CPU scheduling and GPU barriers.
- **Per-archetype residency:** add `ResidencyKind::{Cpu, Gpu}` as an archetype **ATTRIBUTE (not part of
  the signature** — avoid CPU/GPU archetype-count doubling against `MAX_ARCHETYPES = 1024`), classified at
  the single `create_archetype` funnel, reusing the **`ArchetypeFlags: u16` / `StorageKind` 0%-gate
  pattern** (a world with no GPU archetypes pays one bit-test).
- **`ComponentPool` storage-backend seam:** the type-erased `NonNull<u8> + Layout` column store grows a
  variant — host `VmReservation` vs device RHI buffer handle — preserving `base + i*stride` (a device SSBO
  is still `base + i*stride`). (Open: enum-in-pool vs pool-as-trait; enum keeps it `Copy`/`dyn`-free.)
- **`NonSendResource` for the RHI device** — `resource.rs` ALREADY specs this as the deferred migration
  path for "FFI handles." The RHI `Device`/`Queue`/swapchain are its canonical first client: dispatcher-only
  access in the apply window (the ALLOC1 / retired-Arena `!Send + !Sync` discipline — no new soundness model).
- **`SystemKind::{CpuConcurrent, CpuExclusive, GpuCompute}`** — extends the existing `is_exclusive` cache;
  GPU systems record commands on the dispatcher (cheap; the GPU does the work), keeping the queue
  single-threaded (do NOT fan GPU recording across the work-stealing pool — reintroduces the Phase-9.1-9.3
  aliasing problems).
- **Extract largely COLLAPSES.** Bevy's main-world/render-world split exists chiefly for CPU↔CPU pipelining
  (sim N+1 ∥ render N); its render world is "not a GPU-resident structure." With GPU-residency the data
  already lives on-device (the column IS the buffer) → no Extract copy for GPU-resident archetypes. Keep a
  `Changed<T>`-gated delta-upload system only for CPU-resident archetypes (flecs-style renderer-as-system,
  not a separate World). **Don't build a render world now; leave a double-buffer/fence seam** if display-rate
  pipelining (sim N+1 ∥ render N) is later wanted.
- **Render-graph** (a small DAG of passes, reads/writes → barriers + recording order) lives in
  `boyko_render`, ABOVE the RHI — structurally the same shape as the system conflict graph, reusing the
  edge→barrier lowering.
- **Crate topology (keep `boyko_ecs` graphics-pure):**
  ```
  boyko_utils, boyko_threadpool
      ↑
  boyko_ecs (CORE, graphics-free; gains ONLY abstract seams:
             ResidencyKind tag, NonSendResource/NonSendRes, SystemKind::GpuCompute,
             ComponentPool storage-backend variant)
      ↑                    ↑                      ↑
  boyko_rhi (trait,    boyko_physics          boyko_serialize
   NO FFI)             (manifold seam +
      ↑                 swappable solver)
  boyko_rhi_vulkan (raw hand-FFI Vulkan 1.3 backend; embedded SPIR-V)
      ↑
  boyko_render (render-graph ABOVE rhi; extract/prepare/queue/render = Phase-15 system sets)
      ↑
  boyko_app / game (replaces the boyko_demo wgpu seam)
  ```
- **Open questions for the architect:** pipelining seam now or deferred?; `ComponentPool` enum vs trait?;
  residency attribute vs signature (confirm attribute)?; GPU dispatch site (dispatcher-only vs render
  thread)?; barrier-lowering in `boyko_ecs` (knows GPU systems abstractly) vs `boyko_render` (core stays
  graphics-blind, only exposes edges — recommended)?; the single sanctioned CPU↔GPU readback contract
  (apply-window + fence, never in a hot loop).

---

## 6. Cross-cutting constraints (carry into the plan)

- **0%-gate:** every new core seam (ResidencyKind, NonSendResource, SystemKind, storage-backend) must add
  NOTHING to CPU spawn/iter/schedule when unused — the `ArchetypeFlags`/`StorageKind` bit-test pattern.
- **`boyko_ecs` dependency purity:** NO graphics type (`Buffer`/`Vulkan`/`Device`) ever appears in core —
  only abstract seams; the concrete device buffer lives behind the RHI trait.
- **Soundness:** the GPU half is not Miri-checkable → validation layers (debug) + golden-state tests;
  raw FFI follows the `vm.rs` `// SAFETY` + cfg + (where relevant) Miri-fallback discipline.
- **Effort realism:** raw Vulkan + sub-allocator + the GPU-driven SDF/ECS store is a large, multi-phase
  build. The user wants the **foundation/base** first — each foundation piece must be the real
  GPU-resident/indirect-driven version at small scale, extensible without reshaping.
- **Serialization** (Phase S0 landed, `03edd92`): orthogonal feature, continues on its own S1+ track.

## 7. Source provenance

Each stream's full source list (URLs + quotes + verification tags) lives in its agent transcript. The
load-bearing primaries: wgpu-hal docs, Vulkan spec (limits/dispatch-indirect/BDA/dynamic-rendering),
Vulkan-Loader docs, glslang/DXC + SPIR-V toolchain, AMD FidelityFX Brixelizer (GPUOpen + GDC 2023),
Aokana (arXiv 2505.02017), Nanite (GDC 2024 + writeups), Interplay-of-Light deferred-SDF (2017), NVIDIA
JCGT 2022 "Ray Tracing of SDF Grids" (Alex Evans co-author), Media Molecule *Dreams* (Alex Evans 2015),
Jolt Architecture.md, Rapier/parry docs + CHANGELOG, Box2D v3 / Erin Catto solver writeups, XPBD
(Macklin 2016), EA SEED PB-MPM, Svelto+ComputeSharp (sebaslab), Unity Entities Graphics, Bevy render-world
discussion #13494 + cheatbook, GPU Gems 3 prefix-sum, Merrill & Garland decoupled-lookback scan.
