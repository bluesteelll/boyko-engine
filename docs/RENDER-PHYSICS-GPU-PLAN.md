# Render / Physics / GPU-ECS — Unified Foundation Implementation Plan

> **Status: PLAN (research + design complete; not yet implemented).** Branch `ecs`, 2026-06-16.
> Synthesized from the 7 web-verified research streams ([docs/RENDER-PHYSICS-GPU-RESEARCH.md](RENDER-PHYSICS-GPU-RESEARCH.md))
> via an architect → 3-critic panel (perf / soundness+purity / scope+sequencing) → finalize workflow.
> All Critical/Important critic findings are resolved or explicitly deferred (§9). Live-code facts were
> verified against the `ecs` tip before finalizing (§ "Verified code facts").
>
> **Binding constraints:** (1) ULTIMATE design by first-principles perf — precedent absence is a risk
> signal, not a disqualifier; (2) NO third-party crates in the graphics/core stack — in-house RHI + raw
> hand-declared FFI; (3) native **Vulkan-first** (DX12/Metal later behind the RHI); (4) **foundation/base
> only now** — extensible seams, not the full engine; (5) `boyko_ecs` stays graphics-pure; (6) **0%-gate**
> (no CPU hot-path cost when unused); (7) English-only artifacts.

---

## 0. Foundation thesis (re-scoped per critique C2/W3)

The research crystallized a *destination*: **one GPU-resident, zero-readback, GPU-driven store where SDF
bricks/edits and ECS component columns are both device-local SSBO columns of GPU-resident archetypes.**
That destination is sound and is what we build *toward* — but it is **NOT the foundation acceptance
criterion** (that would be over-building a frontier, partly-unprecedented system before the load-bearing
risk is proven).

**The foundation thesis is the single load-bearing risk, in miniature:**

> **A hand-rolled Vulkan backend that allocates device memory, writes a buffer from a compute shader, and
> lets a SECOND GPU pass consume it through an explicit barrier — with ZERO per-frame CPU readback —
> validated headlessly against a golden state, touching `boyko_ecs` zero times.**

Everything else (the SDF brick atlas / clipmap / JFA / raymarch / visibility buffer, the GPU-resident ECS
columns, physics solving) is layered on top **once that risk is retired**. The research §3 "SDF foundation
slice" is therefore moved to **Phase 6+**, not the first deliverable.

---

## Verified code facts (basis for the integration design)

- **2 real archetype mint sites** (not "one funnel"): `create_archetype` (`archetype_master.rs:142`) and
  `add_existing_archetype` (`:481`, self-documented "third funnel"); `get_or_create_archetype` (`:667`)
  delegates to `create_archetype` on miss (`:681`). → residency must be stamped at **both** real sites.
- **Archetype dedup key = `filtered_signature_mask` only** (`:151-156`, `:498`, `:674`) — residency is
  **not** in the key. → residency must be a deterministic function of the signature (§5.1), never a second
  axis that forks identity.
- **`Access::conflicts_with` is symmetric / undirected / coarse**, no stage or access-type masks
  (`access.rs:163-171`). → the conflict graph gives **ordering only**; GPU barriers need new mask info (§5.5).
- **`ComponentPool`** = one `VmReservation` laid out `[data | added_ticks | changed_ticks]`, three
  write-once base pointers, O(1) no-copy grow (`component_pool.rs:31-94`). → the device-backing variant must
  reconcile with these invariants (§5.4).
- **`NonSendResource` is a spec comment only** today (`resource.rs:18-21`). → it must be implemented (§5.3).
- **`ArchetypeFlags` is `u16`, bits 0..=10 used, ~5 free** (`archetype_flags.rs:25-69`). → residency as bit
  `1<<11` is viable and is the 0%-gate precedent (§5.2).
- **Apply-window `running == 0`** at `schedule.rs:555`. → the sound CPU↔GPU recording/submit point (§5.3).

---

## 1. First-principles performance model (corrected)

- **Regime taxonomy** (from the digest): A = CPU-ECS (stream `N·B` at DRAM bandwidth, burns cores);
  B = upload/compute/readback (a `2·N·B` PCIe tax/frame — the Svelto trap); C = GPU-resident + zero-readback
  (stream `N·B` at VRAM bandwidth, ~zero CPU).
- **§1.1 Regime-C dominance is conditional, with a break-even (critique C1 fix).** C beats A not by a flat
  "VRAM/DRAM bandwidth ratio" but past a break-even of **~3M entities for a single cheap system**; the win is
  real and large for **many-systems-per-frame, stable-residency** workloads (particles, SDF eval, boids,
  mass transforms) where the per-frame CPU cost amortizes to a command-buffer record + a ~1-2 µs doorbell.
  Below the break-even, or for few-systems/branchy workloads, **CPU wins** — this is why residency is
  per-archetype and opt-in, not global.
- **§1.3 The SDF regen wall is ALU/JFA-sample-bound (~16 ms), not write-bandwidth-bound (critique C3 fix).**
  The naive "1M bricks × 512 B = 0.5 ms write" floor is misleading; the real cost is the JFA/voxelize passes
  (3 passes × 27 samples) over dirty bricks. The cure is therefore **fewer JFA passes + tighter dirty-region
  scoping**, not more bandwidth. `BRIXEL_FORMAT` defaults to **R16** (W2) — R8 only if a measured visual bar
  permits.
- **§1.7 Despawn is tombstone-amortized:** mark-dead + periodic decoupled-lookback stream-compaction, not a
  compaction every frame.
- **§1.8 Spawn uses a two-level atomic** (subgroup-reduce → one global `atomicAdd` per subgroup; shard the
  counter if contention is measured).
- **§1.11 Frame-overlap (sim N+1 ∥ render N) is mandatory-but-deferred:** a double-buffer/fence seam is
  reserved now (so it is addable without reshaping) but not implemented in the foundation.
- **Dispatch sizing:** `vkCmdDispatchIndirect` (device-written group counts) → entity/edit count decouples
  from dispatch count (~6-8 indirect dispatches/frame regardless of millions of edits).

## 2. CPU/GPU population partition (decision-vs-bytes split; critique C4 fix)

- The boundary is a **population partition**, refined: it is a split of **where the DECISION runs** vs
  **where the BYTES live**, which are not the same axis.
- **GPU-resident archetype:** its component bytes live in device SSBOs; its data-parallel systems are compute
  dispatches; the GPU owns spawn/despawn/intra-archetype churn. Optimal when **archetype membership is
  stable** (only values + intra-archetype spawn/despawn churn).
- **CPU MUST own:** all branchy/heterogeneous/low-N/decision logic and **all archetype-structure decisions**.
- **Forbidden: a CPU system touching GPU-resident bytes** — that is Regime B (the readback trap) and is
  rejected by construction (a CPU `Access` never matches a GPU archetype; §5.4).
- **Archetype MOVES across residency / across GPU archetypes** = a **deferred GPU scatter pass** (not a
  per-frame CPU operation). The earlier "moves are a permanent CPU seam" claim is **withdrawn**; moves of
  GPU-resident entities are a GPU compute job, just deferred past the foundation.

## 3. Crate topology + worktree map

```
boyko_utils, boyko_threadpool                 (existing; SparseSlotMap/Slot = RHI handles, pool = SDF/physics jobs)
    ▲
boyko_ecs (CORE — stays graphics-PURE; gains only abstract seams §5)
    ▲                         ▲                         ▲
boyko_rhi (trait, NO FFI) boyko_physics            boyko_serialize (S0 landed 03edd92; S1+ separate track)
    ▲                     (manifold seam +
boyko_rhi_vulkan          swappable RigidSolver;
 (raw hand-FFI Vulkan 1.3) solver = Phase 10)
    ▲
boyko_render (render-graph ABOVE the RHI; extract/prepare/queue/render = Phase-15 system sets;
              owns the conflict-edge → vkCmdPipelineBarrier lowering)
    ▲
boyko_app / game (replaces the boyko_demo wgpu seam)
```

- **One graphics-spine worktree** carries `boyko_rhi` + `boyko_rhi_vulkan` + `boyko_render` (serial, the
  critical path). `boyko_physics` is a **stub crate** (manifold + no-op solver) until Phase 10.
- **The ONLY core touch** is a single late step (§5, Phase 4/5) adding the four abstract seams to
  `boyko_ecs` — its own small worktree, merged before the GPU-ECS phases. No two phases race the same core file.

## 4. In-house RHI + Vulkan backend

- **RHI trait** = wgpu-hal `Api`-shaped (associated types `Device/Queue/CommandEncoder/Buffer/Texture/
  Sampler/ShaderModule/Pipeline*/BindGroup/Surface`); **static dispatch, not object-safe**; validation
  pushed to the caller; **explicit caller-side barriers**. NO FFI in `boyko_rhi`.
- **Handles = generational indices** over `boyko_utils::SparseSlotMap`/`Slot` (ABA-safe, `Copy`, zero-heap) —
  this index *is* the opaque `DeviceColumnHandle(u64)` core sees (§5.4).
- **Raw Vulkan** (`boyko_rhi_vulkan`): bootstrap `LoadLibrary`/`dlopen` + 3-tier `vkGetInstanceProcAddr`
  (use `vkGetDeviceProcAddr` for the hot path) on the **`vm.rs` raw-`extern` + `// SAFETY` + cfg-per-OS**
  discipline; **Vulkan 1.3 dynamic rendering** (no `VkRenderPass`/`VkFramebuffer`); `#[repr(transparent)]`
  handle newtypes; `#[repr(C)]` structs.
- **Sub-allocator (highest risk — proven in isolation FIRST, §7 Phase 0b):** two pools — a **ring** for
  streaming/per-frame uploads + a **free-list-with-coalescing** for long-lived resources — on the
  `MemFreeBlockMaster` lineage. Memory-type selection from `VkPhysicalDeviceMemoryProperties`. Bricks use a
  **uniform-block O(1) zero-fragmentation pool** (O3). (VMA is forbidden; `maxMemoryAllocationCount` ≥4096
  forbids per-resource allocation.)
- **Shaders:** offline **HLSL → DXC → SPIR-V** as a build step; **committed `.spv` blobs** + `include_bytes!`
  wrapped `#[repr(C, align(4))]` (zero runtime dep; headless-CI-friendly; DXC gives a free future DX12
  backend). (`build.rs`-invoked DXC is the alternative; committed blobs win for hermetic/headless builds.)
- **Descriptors:** classic descriptor sets for the foundation; bindless/descriptor-indexing (core 1.2) later.
- **Buffer Device Address:** used for cold linked structures (BVH nodes) only, not the hot path (O2).
- **Windowing (D8):** the foundation is **headless** (no window needed to prove the thesis), so the
  winit/eframe-vs-raw-Win32/XCB decision is **deferred to the first on-screen phase** — it does not gate
  Slice 0. (Leaning raw Win32/XCB to honor zero-third-party, decided when first needed.)

## 5. The four abstract core seams (each resolved against live code)

- **§5.1 `ResidencyKind::{Cpu, Gpu}` = a deterministic function of the archetype signature, stamped at
  BOTH mint sites** (`create_archetype:142` + `add_existing_archetype:481`), **one-signature-one-residency-
  for-life**, **NOT in the dedup key**; a spawn whose components imply a conflicting residency is **rejected**
  (loud). Classification via a cold `RESIDENCY_CLASS: [AtomicU8; MAX_COMPONENTS]` table (mirrors
  `STORAGE_KIND`). This resolves both the funnel-coverage bug (C1) and the dedup contradiction (C2).
- **§5.2 Residency stored as `ArchetypeFlags` bit `1<<11`** (the existing u16, ~5 free bits) — the proven
  0%-gate (`StorageKind`/hooks precedent): a world with no GPU archetypes pays one `test`/`jz`.
- **§5.3 `NonSendResource` / `NonSendRes<R>` / `NonSendResMut<R>` implemented** (today only a spec comment):
  the RHI `Device`/`Queue`/swapchain live here; **a system touching a `NonSend` resource runs only on the
  dispatcher in the apply-window (`running == 0`)** — genuinely disjoint from concurrent workers, so the
  retired-Arena/ALLOC1 `!Send+!Sync` discipline applies with **no new soundness model**. GPU command
  recording + submit happen here; the single sanctioned readback is `apply-window + fence`, gated behind
  `NonSendResMut<RhiDevice>`.
- **§5.4 `ComponentPool` gains a `PoolBacking` ENUM** (`Host(VmReservation)` | `Device(DeviceColumnHandle(u64))`),
  not a trait (keeps the pool `dyn`-free). `row_ptr = buffer.add(i*stride)` is **byte-identical** for the host
  variant. **Device invariant reconciliation:** no per-row ticks on device (change detection is CPU-archetype
  only); grow = realloc + copy + fence + re-fetch base (not the host O(1) no-copy grow); a CPU `Access` never
  matches a GPU archetype so `row_ptr` **never sees a `Device` pool** (the 0%-gate is airtight — no per-access
  backing check). **Core stays Miri-clean** via a `cfg(miri)` host-only gate + the opaque `u64` handle (no
  graphics type in core).
- **§5.5 `SystemKind::{CpuConcurrent, CpuExclusive, GpuCompute}`** replaces `SystemBox.is_exclusive` (same
  1-byte hot load). **Barrier model rebuilt (C3):** the `ConflictGraph` provides **ordering only**; the
  Vulkan stage/access masks come from a **new `GpuAccessIntent`** a GPU system declares; the
  **edge → `vkCmdPipelineBarrier` lowering lives in `boyko_render`** (core stays graphics-blind, exposing
  only abstract conflict edges). Obligation: the lowering is **superset-correct** (over-synchronize rather
  than miss a barrier) + a **sync-validation golden test**.

## 6. Validation (oracle-before-code)

- **The GPU half is NOT Miri-checkable** (VRAM mapping, raw FFI, GPU↔CPU buffers). Validation shifts to:
  **(a)** Vulkan **validation layers wired to FAIL the test** (debug builds, headless); **(b)** deterministic
  **golden-state / golden-buffer** assertions (a compute pass writes a known pattern → readback in the TEST
  ONLY → diff vs golden); **(c)** multi-driver CI where available for the residual (cross-vendor intermittent
  races are the one ACCEPTED residual — no silver bullet).
- **`boyko_ecs` core stays Miri-clean** — the device pool variant is `cfg(miri)`-gated to host-only; the
  abstract seams carry no graphics types. The existing Miri-TB suites must stay green.
- **The validation oracle is online from line one** (Phase 0a) — before any GPU code depends on it.

## 7. Phased implementation order (one linearized critical path)

Rooted at "a device that allocates + dispatches"; each phase compiles + demonstrates standalone.

- **Phase 0 — RHI/Vulkan spine (serial, one worktree, `boyko_ecs` untouched):**
  - **0a** Loader + instance + device + **validation-layers-fail-the-test**, headless (prints device name). Oracle online.
  - **0b** **Sub-allocator in isolation** (the highest risk): one `VkDeviceMemory` block → sub-allocate → map → write/read → assert.
  - **0c** One compute dispatch writes a known pattern → fence → readback → assert (submit/sync/SPIR-V-from-committed-`.spv` proven).
  - **0d** A SECOND compute pass transforms the buffer chained via `vkCmdPipelineBarrier` → diff vs golden (the §5.5 lowering in miniature). **← this is "Slice 0", §11.**
- **Phase 1-3 — RHI surface completion:** the wgpu-hal-shaped trait + Vulkan impl of buffers/textures/3D
  images/pipelines/descriptor-sets/swapchain (swapchain only when on-screen is needed).
- **Phase 4 — core seams (the single core worktree):** §5.1-§5.5 into `boyko_ecs` (residency, NonSend,
  PoolBacking, SystemKind, GpuAccessIntent), all 0%-gated, existing Miri suites green.
- **Phase 5 — `GpuColumn` + one `GpuSystem`:** a device-SSBO column mirror; one compute system ordered by
  the conflict graph, barrier-lowered in `boyko_render`; **zero-readback** steady state proven on a real
  ECS column.
- **Phase 6+ — SDF store (the research §3 slice):** device brick-pool (uniform-block O(1)) + GPU free-list +
  GPU edit-list (BDA-cold) + ONE regen indirect dispatch (voxelize → JFA) + ONE 64³ clipmap cascade
  (cascade-per-slice Morton locality) + flattened-array BVH + tiled raymarch writing NDC depth + 64-bit
  visibility buffer; deferred-G-buffer hybrid + Hi-Z tile cull. Then indirect dispatch + atomic spawn/dead-
  list + stream-compaction despawn for GPU-ECS structural ops.
- **Phase 10 — physics solver:** the in-house Soft-Step rigid solver behind the seam (Phase 0-9 ship a
  no-op/stub solver + the manifold seam only).
- **Deferred seams:** frame-overlap (double-buffer/fence) pipelining; GPU↔GPU archetype-move scatter pass;
  bindless descriptors; DX12/Metal backends.

## 8. Physics (foundation = seam + manifold + no-op)

- **Manifold (`points`, `normal`, `penetration`) = the universal currency**; `trait RigidSolver` is the
  swappable backend; `RigidBody`/`Collider`/`Contact` are ordinary CPU-archetype components.
- **In-house SDF-native paths** (real edge): SDF queries; **collision-mesh-from-SDF via Marching Cubes on
  `boyko_threadpool`** tied to the dirty-brick set; body-splitting via connected-components; XPBD/MPM
  continuum ("cut" = bulk-despawn particles/tets where `cutterSDF < 0`).
- **Default solver = in-house Soft-Step** (Catto Box2D-v3 substep+soft-constraint lineage; zero-dep, proves
  the seam). **The no-third-party constraint is scoped to graphics/core only** → `boyko_physics` MAY later
  take a Rust dep (Rapier/parry, whose `Voxels` collider is the SDF adapter) **behind the seam** if perf
  demands; Jolt-FFI is the other swap. **Determinism needs a stable apply order** (ties to the apply-window
  barrier). "Shared BVH" = share the CODE, not the instance.

## 9. Resolved / deferred / accepted decisions (summary)

| # | Decision | Resolution |
|---|---|---|
| D1 | Foundation thesis | RE-SCOPED to one zero-readback chained-GPU-work slice; unified store = destination (§0) |
| D2 | Residency tag | Fn-of-signature, stamped at BOTH mint sites, one-per-signature-for-life, not in dedup key (§5.1) |
| D3 | Residency storage | `ArchetypeFlags` bit `1<<11`, 0%-gate (§5.2) |
| D4 | RHI device home | Implement `NonSendResource`, dispatcher-only/apply-window (§5.3) |
| D5 | `ComponentPool` backing | `PoolBacking` ENUM (host VM \| device u64 handle), Miri-clean via cfg-gate (§5.4) |
| D6 | Barrier lowering | Conflict graph = ordering only; `GpuAccessIntent` masks; lowering in `boyko_render`, superset-correct (§5.5) |
| D7 | Shader pipeline | HLSL→DXC→committed `.spv` + `include_bytes!` align(4) (§4) |
| D8 | Windowing | DEFERRED to first on-screen phase (foundation is headless); leaning raw Win32/XCB |
| D9 | Atlas sizing | POOL of 3D textures, query `maxImageDimension3D` (NOT 2048³) (§4/§7) |
| D10 | Physics solver default | In-house Soft-Step; no-third-party scoped to graphics/core; `boyko_physics` dep-permitted behind the seam (§8) |
| D11 | Move-wall | Decision-CPU vs bytes-CPU split; CPU-touching-GPU-bytes forbidden; GPU↔GPU move = deferred scatter (§2) |
| D12 | Pipelining | Frame-overlap seam mandatory-but-DEFERRED (§1.11) |
| — | ACCEPTED residual | Cross-vendor intermittent GPU races — no Miri/golden silver bullet; multi-driver CI where available (§6) |

(Smaller resolutions folded: `BRIXEL_FORMAT`=R16 default; cascade-per-slice Morton; two-level atomic +
shard-if-hot; flattened-array BVH, BDA cold-only; uniform-block O(1) brick pool; committed `.spv` headless.)

## 10. Metrics & validation targets

- **0%-gate:** CPU `spawn`/`iter`/`schedule` benches byte-identical vs pre-feature when no GPU archetype
  exists (residency bit-test only). Grep-proof: no graphics type in `boyko_ecs`.
- **Slice-0 golden:** the chained two-pass compute result diffs bit-exact vs a CPU-computed golden buffer.
- **Validation-layers-clean** on every GPU test (debug); zero validation errors = a test gate.
- **Miri:** existing CPU suites stay green (device pool cfg-gated out under Miri).
- **Perf (when the store lands):** GPU-resident column update bandwidth vs the Regime-C model; SDF regen
  cost vs the ALU/JFA-bound estimate; the ~3M-entity break-even sanity-checked on a real workload.

## 11. RECOMMENDED FIRST VERTICAL SLICE — "Slice 0"

**Zero-readback chained GPU work, headless, `boyko_ecs` untouched.** Lives entirely in `boyko_rhi` +
`boyko_rhi_vulkan` (one serial worktree); needs no SDF / physics / windowing / indirect-dispatch / core-seam.
Four steps, each compiles + demonstrates:

1. **Loader + instance + device + validation layers wired to FAIL the test** (prints the device name) — the
   oracle online from line one.
2. **Sub-allocator in isolation** — one `VkDeviceMemory` block → sub-allocate → map → write/read bytes →
   assert (the highest-risk item proven before anything depends on it).
3. **One compute dispatch** writes a known pattern → fence → one readback → assert (submit/sync + SPIR-V from
   a committed `.spv` proven; this readback is the TEST ORACLE, not a per-frame path).
4. **A second compute pass** transforms the buffer chained via `vkCmdPipelineBarrier` → result diffed against
   a CPU golden (proves the §5.5 edge→barrier lowering in miniature).

Proves the single load-bearing risk (zero-readback chained GPU work on a hand-rolled Vulkan backend) with
zero core changes. The `GpuColumn` ↔ `ComponentPool` seam is cut into core only afterward (Phase 4-5), once
real device-buffer constraints are known.

**Prerequisite check before Slice 0:** a Vulkan loader + a Vulkan-capable GPU + the DXC/glslang toolchain
(or committed `.spv`) on the build machine; the GNU Rust toolchain (msvc is broken here).
