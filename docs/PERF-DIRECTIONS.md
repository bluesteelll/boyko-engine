# Performance Directions

> **STATUS: FORWARD-LOOKING DIRECTIONS / OPTIONS TO REVISIT — NOT COMMITTED SCOPE, NOT A PHASE PLAN.**
> This document is a prioritized reference of performance directions for boyko-engine. Nothing here is scheduled, promised, or sequenced. Each item is a *lever to evaluate when its prerequisites and a measured need exist.* The owner decides if and when any of it ships.
>
> Every item is tagged:
> - **`HAVE`** — already in the foundation today (cited to the live `ecs` branch).
> - **`PARTIAL`** — the substrate/seam exists; the lever itself is unbuilt.
> - **`FUTURE`** — designed/researched at most; no implementation exists.
>
> **Honesty discipline (inviolable):** no fabricated speedup numbers. Where a figure is cited it is attributed to its source and flagged order-of-magnitude/qualitative until reproduced here with our own oracle (criterion for CPU, golden-buffer diffs + Vulkan validation layers for GPU). "No one ships X" is never an argument against X.
>
> **0%-gate (inviolable, applies to every item below):** a world using none of a feature pays at most one `test`/`jz`; the named hot loops (`row_ptr`, `for_each_chunk`, query iter, scheduler `find_ready`) stay `cargo-asm` byte-identical; existing Miri-TB suites stay green. Any direction that cannot be added behind this gate is not a direction — it is a rewrite, and is flagged as such.

---

## Decision rules (read these first — they govern everything below)

1. **Route to the cheapest representation.** Hybrid mesh+SDF is *never fused*. Each draw/query goes to whichever structure answers it cheapest (mesh raster vs SDF march vs HW-RT traversal). Pay-for-what-you-use, per query, not per world.
2. **Residency is an economic opt-in, never a default.** GPU-resident + zero-readback beats CPU-ECS only *past* a first-principles break-even of ~3M entities for one cheap system; the win grows with many-systems-per-frame, stable-residency mass workloads. Below that, or for branchy/low-N/gameplay-coupled work, CPU wins. Residency is a per-archetype flag, a deterministic function of the archetype signature, *not* in the dedup key. A CPU `Access` can never match a GPU archetype, so a CPU system touching GPU bytes (the Svelto readback trap) is impossible by construction.
3. **CPU for branchy / heterogeneous / low-N / gameplay-coupled (rigid-body constraint resolve, AI). GPU for homogeneous large-N mass** (rendering, particles/soft/fluid, SDF eval). CPU orchestrates, GPU executes.
4. **Reject the readback round-trip.** Upload→compute→readback (a 2·N·B PCIe tax/frame) is rejected by construction. The only sanctioned GPU model is resident-with-zero-readback.
5. **Minimize bytes touched, then keep them where the consumer is.** Bandwidth (VRAM for GPU-resident, cache+DRAM for CPU) is the real ceiling — not FLOPs. Hot/cold split, SoA columns, and quantize-where-measured come before any algorithmic cleverness.
6. **Measure before optimizing; measurement is the oracle.** No speculative optimization. A change is not an improvement until criterion (CPU) or a golden-buffer/validation diff (GPU) says so. PGO, LTO, and mimalloc shift the *mean* — never compare their numbers against a non-PGO/non-LTO/system-heap baseline.
7. **Measured inlining.** `#[inline]` for cross-crate/generic bodies (so LTO can see them); `#[inline(always)]` only on `cargo-asm`/profiler evidence; `#[cold]`/`#[inline(never)]` for error/rare paths. Blind `#[inline(always)]` bloats L1i and *lowers* perf — it is a review red flag.
8. **0%-gate is mechanical, not aspirational.** Type-keyed features go behind `const { }` monomorphization gates (zero instructions, not even a `test/jz`). Runtime-keyed features go behind an `ArchetypeFlags` bit-test (the one-`test/jz` floor). Conflating the two is a design error.

---

## Section RT — Hardware Ray Tracing (the headline render direction)

HW-RT is called out separately because it is the single largest forward render lever and because the engine's existing SDF-edit BVH + brick-AABB structure *is already the natural acceleration-structure source geometry*. Nothing in the current RHI exists for it: `crates/boyko_rhi/src/api.rs:46-67` declares no `AccelerationStructure` or `RayTracingPipeline` associated type — the entire surface is unbuilt and needs a new RHI seam plus raw `VK_KHR_*` FFI in `boyko_rhi_vulkan`.

### RT-1 — HW-RT for SDF primary visibility — capability-gated backend behind the RHI `[FUTURE]`

**What:** Pack each non-empty SDF brick as an axis-aligned box into a `VK_GEOMETRY_TYPE_AABBS_KHR` BLAS. RT cores traverse the BVH and skip empty space *for free*; the bound intersection shader then runs a trilinear/analytic-cubic solve OR a short sphere-march *only inside the hit brick*. Exposed via `VK_KHR_acceleration_structure` (AABB geometry + intersection shaders) + `VK_KHR_ray_tracing_pipeline` (raygen/intersection/any-hit/closest-hit/miss), or `VK_KHR_ray_query` for inline tracing in compute. Mark geometry `OPAQUE` wherever possible to avoid any-hit (NVIDIA RTX best-practices flag any-hit as expensive — it runs many times per `TraceRay`).

**Why:** RT cores run autonomously in parallel with the SMs, offloading the shader cores; for sparse SDF worlds the hardware BVH gives the empty-space skipping a software clipmap/brick march pays for in ALU. This is exactly the architecture validated by NVIDIA's *Ray Tracing of Signed Distance Function Grids* (JCGT 2022): a BVH over all non-empty bricks with an analytic cubic / repeated-linear-interpolation per-voxel intersection, reported real-time (millisecond) on RTX 3090 as the fastest method tested. **Synergy:** our per-edit BVH-over-edits + brick-AABBs are already the BLAS source — the AABB list is a byproduct of the SDF representation.

**Caveat:** Acceleration-structure rebuild/refit is the worst case for a runtime-mutating SDF world. Refit (`ALLOW_UPDATE`) only holds for geometry that moves modestly relative to its local neighborhood; a "mesh-explosion"-class change must **rebuild**, and refit quality degrades fast under large deformation. The robust pattern is **region routing**: HW-RT for mostly-static geometry and secondary rays; software sphere-trace (RT-4) for the actively-edited near field; capability-gate at device init on `VK_KHR_acceleration_structure` + `VK_KHR_ray_tracing_pipeline` with the software sphere-tracer as the **universal fallback** (mirrors Lumen's HW-or-software split). Build-flag discipline: `PREFER_FAST_TRACE` for static, `PREFER_FAST_BUILD(+ALLOW_UPDATE)` for per-frame-dynamic, `ALLOW_COMPACTION` for static. The "5–10× faster" figure floating around is a search-engine summary, **not** a verified quote — treat as order-of-magnitude only until reproduced with criterion + golden-buffer diffs.

**Cite:** `crates/boyko_rhi/src/api.rs:46-67`; JCGT 2022 SDF-grid RT; `VK_KHR_acceleration_structure`; NVIDIA RTX best-practices.

### RT-2 — Unified mesh + SDF in one TLAS `[FUTURE]`

**What:** One TLAS whose instances reference two BLAS kinds: triangle BLAS (`VK_GEOMETRY_TYPE_TRIANGLES_KHR`) for mesh draws, procedural-AABB BLAS for SDF bricks. A single `TraceRay`/`rayQuery` resolves visibility *and* cross-representation occlusion in one traversal — mesh correctly occludes SDF and vice versa — with no screen-space composite/depth-merge pass. The SBT routes triangle hits to the triangle hit group, AABB hits to the SDF intersection+closest-hit group.

**Why:** Solves the hardest part of a hybrid renderer — correct cross-representation occlusion — at the acceleration-structure level instead of in screen-space heuristics. Honors "never fused, route each query to whichever answers cheapest": geometries stay separate (a BLAS holds only one geometry type), but the TLAS is the shared spatial index. Reuses the exact traversal RT-1 already needs.

**Caveat:** Hard Vulkan constraint — a BLAS contains *only* triangles OR *only* AABBs; unification must happen at the TLAS (two BLAS per logically-mixed object). This compounds RT-1's dynamic-rebuild caveat: every actively-edited SDF region dirties its AABB BLAS and forces a TLAS update — static/dynamic region routing is what keeps it affordable. Depends on the AS/RT RHI seam *and* the still-unimplemented graphics/mesh path (`api.rs:61-66`: `GraphicsPipeline`/`BindGroup` are deferred).

**Cite:** `VK_KHR_acceleration_structure` (single-BLAS-one-geometry-type); "A Diligent Approach to Ray Tracing"; `crates/boyko_rhi/src/api.rs:61-66`.

### RT-3 — RT for LIGHTING (shadows / AO / GI / reflections) — **CONFIRMED FUTURE, highest-value RT use** `[FUTURE]`

**What:** HW-RT for secondary/incoherent lighting rays against the unified mesh+SDF TLAS: ray-traced shadows (`RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH` for opaque shadow/AO rays), AO, diffuse GI, rough/sharp reflections. **Owner has confirmed RT-lighting will be used** — this is a committed-future direction, not merely an option. Pair with an SVGF-class spatiotemporal denoiser since few rays per pixel are affordable.

**Why:** Secondary/incoherent rays are *precisely* where HW-RT wins most and software ray-marching hurts most: shadow/AO/GI rays scatter with no screen-space coherence, so a software march pays full per-ray traversal while RT cores absorb the divergence. The NVIDIA SDF-grid paper calls out a dedicated shadow-ray optimization; RTX best-practices treat secondary-ray *shading* divergence (not traversal) as the primary limiter — i.e. traversal is already cheap on hardware. This is the strongest single justification for adopting HW-RT at all.

**Caveat:** Cost moves from traversal to shading divergence on incoherent rays — use simplified secondary-ray shaders, drop diffuse-GI/rough-specular to vertex-level shading, bias mips, optionally bin/sort shades. Few-sample RT lighting is noisy and **requires** temporal+spatial denoising (SVGF), which couples this to RT-6 (TAA/temporal reprojection) and adds temporal-lag/ghosting failure modes. Gated on the same absent AS/RT RHI seam plus a denoiser. The honest measure is golden-buffer diffs + criterion, not a quoted speedup.

**Cite:** NVIDIA RTX best-practices; JCGT 2022 SDF-grid RT (shadow-ray opt); SVGF (Schied et al. 2017); NVIDIA "HW vs SW accelerated ray tracing".

### RT-4 — Tile-culled software raymarch — universal fallback + near-field tracer `[PARTIAL]`

**What:** A software sphere/cone-tracer driven by a two-level distance field (low-res global DF as coarse acceleration structure + per-instance/per-brick higher-res fields), with per-tile culling so each screen tile marches only the bricks/clipmap cells overlapping its frustum slab (the Lumen software-trace model). Serves as (a) the capability fallback when HW-RT is absent and (b) the near-field tracer for actively-edited geometry where AS rebuild is too expensive.

**Why:** Lumen documents software SDF tracing as its *fastest* tracing method and the recommended path where HW-RT is unaffordable/unsupported — the right universal baseline for pay-for-what-you-use. It needs *no* acceleration-structure rebuild, so it handles the runtime-mutating near field that defeats RT-1.

**Caveat:** Software marching pays full ALU for empty-space traversal RT cores skip for free, and incoherent secondary rays are its weak spot (why RT-3 prefers HW-RT for lighting). `PARTIAL` because the GPU-compute execution substrate exists (Phase 5 `GpuSystem` compute dispatch + zero-readback + barrier lowering) but no SDF brick/clipmap storage, no graphics/`Texture` seam, and no tile-culling pass are built.

**Cite:** "Journey to Lumen"; UE Lumen technical details; `crates/boyko_render/src/gpu_system.rs` (compute substrate HAVE); `crates/boyko_render/src/lib.rs:42-52` (barrier lowering HAVE); `crates/boyko_rhi/src/api.rs:57-60` (Texture/Sampler deferred).

### RT-5 — Hi-Z two-pass occlusion culling `[FUTURE]`

**What:** Build a mip-mapped max-depth pyramid from the depth buffer, then a two-pass GPU-driven cull: pass 1 draws last frame's visible set and culls candidates against Hi-Z; pass 2 re-tests anything pass-1 may have wrongly culled via indirect-dispatch compute, so nothing is incorrectly dropped. Each Hi-Z texel at mip *i* holds the max depth of its mip *i−1* children.

**Why:** Removes occluded draws/bricks before shading — cuts raster *and* (for SDF) the number of tiles that must march. The two-pass form fixes the classic single-pass false-cull. Production-proven (Frostbite, Killzone 3).

**Caveat:** Needs the depth/raster path that doesn't exist (graphics pipeline + render targets are the deferred seam). Reprojected-previous-depth variants trade GPU cost for CPU savings but add hole-filling artifacts + one-frame staleness. Entirely FUTURE.

**Cite:** RasterGrid Hi-Z; "Experiments in GPU-based occlusion culling"; `crates/boyko_rhi/src/api.rs:61-62`.

### RT-6 — Half-res marching + TAA temporal reprojection `[FUTURE]`

**What:** Render the expensive march/RT-lighting buffer at half resolution with a jittered per-pixel start offset, reproject the previous frame via motion vectors into a history buffer, and accumulate. TAA acts simultaneously as anti-aliasing *and* temporal denoiser, converging stochastic/low-sample effects (volumetrics, RT lighting) cheaply. Optionally keep history above display resolution (TSR-style) to avoid reprojection blur.

**Why:** Directly multiplies the effective ray budget; TAA's temporal averaging is what makes few-sample RT lighting (RT-3) viable in real time. Highest-leverage amortization knob for any march/RT pipeline.

**Caveat:** Ghosting, disocclusion holes, lag under fast motion / rapidly-edited SDF; needs motion vectors + history-rejection heuristics (neither exists). Couples tightly to RT-3's denoiser. FUTURE — no graphics output/history-buffer infrastructure.

**Cite:** "Reprojection TAA"; UE TSR; SVGF.

### RT-7 — 64-bit visibility buffer + deferred shared-depth hybrid `[FUTURE]`

**What:** Primary pass writes a compact ~64-bit-per-sample visibility buffer (depth high bits + packed primitive/brick + draw/instance id) instead of a fat 24–32 B G-buffer; deferred shading reconstructs attributes from geometry/material buffers via that id. Shared depth lets SDF march and mesh raster write the same depth so the deferred pass shades both uniformly.

**Why:** Large bandwidth/footprint win (cited 1080p/8× example: ~64 MB visibility vs ~398 MB for a 24 B G-buffer) + better shading memory-access patterns; shared depth is the clean place to merge mesh+SDF visibility for deferred lighting, complementing RT-2 for the raster-side path.

**Caveat:** Shifts cost to re-fetching triangle/vertex/material data during shading + attribute-interpolation complexity; for SDF the "primitive id" must encode brick/voxel coordinates, so the packing is bespoke. FUTURE — depends on the unbuilt graphics/render-target seam.

**Cite:** JCGT cache-friendly deferred / visibility buffer; "Deferred Texturing"; Aokana (arXiv 2505.02017); `crates/boyko_rhi/src/api.rs:57-66`.

### RT-8 — Async-compute overlap (AS build, SDF march, denoise on a separate queue) `[PARTIAL]`

**What:** Submit AS build/update, SDF compute marching, sim, and denoising on an async-compute (and/or transfer) queue so they overlap graphics-queue raster work that leaves SMs idle. NVIDIA RTX best-practices: move AS management to async compute, overlap BLAS/TLAS build+denoise with G-buffer/shadow passes, keep AS build < ~2 ms, single BLAS→TLAS barrier, avoid serializing barriers.

**Why:** For HW-RT it can hide AS-rebuild cost almost completely — directly mitigating RT-1's dynamic-rebuild caveat. Vendor guidance cites occupancy lifts from ~40–60% to ~85–95% on balanced frames.

**Caveat:** Async is scheduling, not free parallelism — co-scheduled passes contend for SMs, caches, and bandwidth; pairing two ALU-bound or two bandwidth-bound passes can slow *both*. `PARTIAL`: the project has a queue + explicit barrier-lowering (`PlannedBarrier` replaying `vkCmdPipelineBarrier`) + `copy_buffer`, but no dedicated async-compute/transfer queue and no multi-queue scheduling — multi-queue submission + cross-queue semaphore sync is new work (see GPU-D3 below — the same lift).

**Cite:** NVIDIA RTX best-practices; AMD GPUOpen concurrent execution; "Async compute all the things"; `crates/boyko_render/src/lib.rs:42-52`.

### RT-9 — Variable-rate / adaptive-epsilon marching `[FUTURE]`

**What:** Adapt sphere-march termination epsilon and step count per ray/region: tighter epsilon + more steps for foreground/center/specular rays, coarser + fewer for distant/peripheral/diffuse-GI rays. Optionally drive with a VRS-style screen-tile mask.

**Why:** March cost scales with step count and epsilon; spending precision only where the eye/lighting needs it is a direct pay-for-what-you-use lever, reducing ALU on exactly the incoherent/diffuse rays software marching handles worst. Pairs with RT-6 (half-res + TAA) and Lumen cone tracing for rough/distant lobes.

**Caveat:** Risks surface-acne / missed thin features + temporal flicker if the rate field changes frame-to-frame; needs a stable per-region rate heuristic and likely TAA to hide residual noise. FUTURE — no marcher exists; a refinement layered on RT-4.

**Cite:** NVIDIA RTX best-practices; "Journey to Lumen"; JCGT 2022 SDF-grid RT.

---

## Section GPU — GPU execution efficiency (behind the RHI, on GPU-resident columns)

Foundation state verified in-repo: `dispatch_indirect` is a `#[cold]` no-op stub (`encoder.rs:73-77`); `RhiQueue::submit` is single-queue, fence-only, no semaphores (`queue.rs`); `RhiDevice` exposes no queue-family query (`device.rs`); `GpuSystem` dispatches `ceil(len/64)` from a CPU-pushed push-constant `count`, synchronous submit+fence, one-thread-per-element (`gpu_system.rs`); barrier lowering is superset-widen with only `COMPUTE_SHADER|TRANSFER` stages, no `ALL_COMMANDS`/`MEMORY_*` (`barrier.rs`, `enums.rs:96-167`); `BufferUsage::INDIRECT` already exists (`enums.rs:37`) and `GpuStage::Indirect` already widens (`barrier.rs:122-124`) — the abstract surface anticipates indirect.

### GPU-D1 — `vkCmdDispatchIndirect`: device-written group counts `[FUTURE]`

**What:** Fill the existing `dispatch_indirect(&mut self, buffer, offset)` seam so X/Y/Z workgroup counts are read by the device from a `VkDispatchIndirectCommand` (three `u32`) in a small device-local INDIRECT buffer that a prior compute pass wrote — instead of a CPU-pushed `count`. The CPU records a constant number of dispatches/frame regardless of live row count; the device computes `ceil(live_rows/64)` itself. Pairs with maintaining the per-archetype `device_len` counter (device twin of `ComponentPool::len`) on-GPU.

**Why:** Serves "roughly constant dispatches/frame regardless of millions of elements." Today `GpuSystem` pushes a CPU-known `count`, forcing a CPU↔GPU dependency exactly where GPU-driven residency wants none. Indirect dispatch is the standard Vulkan mechanism and the prerequisite for every other GPU-driven direction below (GPU-D6 compaction-driven spawn, RT-5 culling).

**Caveat:** Fights the 0%-gate *only* if it touches the synchronous path — mitigated because the seam is a separate `#[cold] #[inline(never)]` default body no foundation code calls. Real costs: (a) an extra RAW barrier so the count buffer is visible as `INDIRECT_COMMAND_READ` — a stage the foundation enum set lacks, so `GpuStage::Indirect` currently over-widens (sound but coarse); (b) indirect dispatch can be marginally slower per-call on some drivers and loses the CPU's ability to skip a zero-count dispatch; (c) requires INDIRECT usage at buffer create time. **The win is structural (decoupling), not a per-dispatch speedup — no cited Nx.**

**Cite:** `encoder.rs:73-77`; `gpu_system.rs:115-133`; `enums.rs:37`; `barrier.rs:122-124`; Vulkan spec `vkCmdDispatchIndirect`.

### GPU-D2 — Subgroup / wave intrinsics (ballot/scan/reduce) `[FUTURE]`

**What:** Use Vulkan subgroup ops (`VK_SUBGROUP_FEATURE_BASIC/BALLOT/ARITHMETIC`; HLSL `WaveActiveBallot`/`WaveActiveCountBits`/`WavePrefixCountBits`/`WaveReadLaneFirst`) inside the column compute shaders: (a) broadphase prefix-scan/reductions without LDS round-trips; (b) per-wave atomic-bump append — one global atomic per wave, each lane's slot = `base + WavePrefixCountBits(is_active)`; (c) AABB-merge reductions. All Vulkan 1.1 devices support BASIC subgroup ops in compute.

**Why:** The dominant per-shader efficiency lever for the homogeneous large-N GPU mass. The atomic-bump pattern cuts global-counter atomic traffic by ~the wave width (32/64); the cited source states it "can bring traffic to the atomic global counter down by a lot." Subgroup reduce/scan "avoid shared memory, reducing latency and increasing bandwidth." It is the in-shader building block for GPU-D6 and decoupled-lookback scan.

**Caveat:** 0%-gate-neutral (lives entirely in SPIR-V, no CPU asm change). Real caveats: (1) wave width is hardware-variable — shaders must query `SubgroupSize`, never hard-code; (2) subgroups must be uniform/converged before the op or results are undefined (`GL_EXT_subgroup_uniform_control_flow`); (3) arithmetic/ballot bits beyond BASIC are OPTIONAL — capability query + fallback mandatory; (4) requires exposing subgroup feature flags through the RHI device-create path, which has no such query. Only quoted figure is the qualitative atomic-traffic reduction.

**Cite:** Khronos Vulkan subgroup tutorial; Vulkan subgroups guide; "Stream compaction using wave intrinsics"; `gpu_system.rs:108-120` (plain `numthreads(64,1,1)`, no subgroup ops).

### GPU-D3 — Async compute queues (SDF regen ∥ physics ∥ render) `[FUTURE]`

**What:** Add a multi-queue submission path: query queue families at device create (graphics+compute+dedicated transfer/DMA), expose a second compute queue and a transfer queue, extend `RhiQueue::submit` to take wait/signal semaphores so independent passes overlap (SDF regen on async-compute while rigid-body integrate runs on main compute and render runs on graphics; staging uploads on the DMA queue).

**Why:** The hybrid + per-archetype-residency design produces genuinely independent GPU workloads (SDF eval, particles/soft/fluid, render) the constraints call out as overlap candidates. The Vulkan `async_compute` sample measured a real win (21.8 ms vs 22.9 ms) overlapping shadow raster with compute post. Dedicated copy/DMA queues let staging uploads run without blocking compute.

**Caveat:** Heaviest lift, most in tension with the foundation: today one queue, fence-only submit, NO semaphores (`queue.rs:31-36`), no queue-family query (`device.rs`). Cross-queue sync needs semaphores + queue-family-ownership transfers (a real cost the single-queue path never pays). Async helps **only** when the GPU is not already saturated (NVIDIA: only when SM occupancy shows unused warp slots); the cited win is ~5%, not order-of-magnitude. The classic anti-pattern (a `FRAGMENT→COMPUTE` barrier forcing a `COMPUTE→FRAGMENT` barrier back) can net-stall. 0%-gate: must be purely additive (new queue handles behind the RHI), but it perturbs the GPU-D4 barrier-lowering pass, which currently assumes one timeline. (Same lift underpins RT-8.)

**Cite:** `queue.rs:31-36`; `device.rs`; AMD GPUOpen concurrent execution; Vulkan `async_compute` sample; NVIDIA advanced-API async-compute.

### GPU-D4 — SoA SSBO coalescing + narrowing the superset-correct barrier lowering `[PARTIAL]`

**What:** (a) Keep device columns std430 / 16 B-aligned with the `base + i*stride` layout `GpuColumn` already uses so consecutive lanes read consecutive 4/16 B elements (coalesced 256 B/wave per AMD); split hot/cold GPU fields into separate columns mirroring the CPU SoA so a shader touches only the column it needs. (b) Narrow barrier lowering from superset-widen to provably-minimal where producer/consumer (stage, access, sub-range) is unambiguous, and BATCH barriers ("barriers can drain the GPU of work").

**Why:** Coalescing is the single biggest bandwidth factor — AMD recommends "coalesced 256-byte blocks per wave" + SoA. The `base+i*stride` device column (Phase 5) already gives the coalesced layout; this direction preserves it as more columns/fields are added. Barrier minimization is a direct execution win; the current lowering is deliberately superset-correct ("over-synchronising is sound, a missed barrier is not") and explicitly flags it "should narrow where provable."

**Caveat:** Narrowing barriers is where this fights the 0%-gate/soundness most: the coarse lowering exists precisely because a *missed* barrier is UB. Any narrowing must keep `sync_validation.rs` green and provide an over-approximation fallback. The enum set lacks `INDIRECT_COMMAND_READ` and `ALL_COMMANDS`/`MEMORY_*` (`enums.rs:96-167`), so narrowing first needs new stage/access constants (each keeping the identity-cast-to-Vk numeric trick). Coalescing caveat: 16 B std430 can waste VRAM on small components (4 B padded to 16 B) — measure bandwidth-vs-footprint per column, never blanket-pad. Qualitative vendor guidance only.

**Cite:** `crates/boyko_render/src/barrier.rs`; `crates/boyko_render/tests/sync_validation.rs`; `crates/boyko_rhi/src/enums.rs:96-167`; AMD RDNA performance guide.

### GPU-D5 — Push-constant vs UBO/SSBO params; persistent-threads vs one-thread-per-element; occupancy `[PARTIAL]`

**What:** (a) Keep small per-dispatch params (row count, base offset, stride) in PUSH CONSTANTS (already done — `gpu_integrate` pushes `count`); move larger/rarer param blocks to a UBO/SSBO. (b) Evaluate persistent-threads (a fixed grid of long-lived workgroups pulling work from a device-side queue/counter) vs one-thread-per-element for irregular/variable-N workloads. (c) Tune workgroup size to a multiple of 64 (AMD: best across all GPU generations; smaller groups improve async overlap/occupancy).

**Why:** Push constants for per-dispatch params is the recommended pattern (AMD: keep root signature < 13 DWORDs) and already done for `count`. Persistent-threads is the standard answer when work is irregular or when GPU-D1's indirect count would launch many empty/divergent workgroups. Multiple-of-64 is free occupancy alignment for wave32 and wave64.

**Caveat:** Push-constant budget is small (Vulkan guaranteed minimum 128 B; AMD's <13-DWORD advice is tighter) — over-stuffing spills to memory; keep the struct tiny, larger blocks go to a UBO/SSBO (extra binding + a barrier if device-written). Persistent-threads needs a device-side work counter (atomics, ties to GPU-D2/D6) and a forward-progress assumption NOT guaranteed across all GPUs (same hazard as decoupled-lookback). 0%-gate-neutral on CPU, but persistent-threads changes the dispatch model and must not regress the simple one-thread-per-element path small-N archetypes use. Occupancy gains are workload-dependent — criterion-measure.

**Cite:** `gpu_system.rs:108-133`; AMD RDNA performance guide; Vulkan subgroups guide.

### GPU-D6 — GPU-driven structural spawn/despawn via stream compaction (the capstone) `[FUTURE]`

**What:** Move structural ops on GPU-resident archetypes onto the GPU: despawn marks rows dead (tombstone/enabled bit); a compute pass STREAM-COMPACTS live rows into a dense prefix via single-pass decoupled-lookback prefix scan (Merrill & Garland 2016) + the per-wave atomic-bump append (GPU-D2) for spawn, writing the new `device_len` into the indirect-dispatch buffer (GPU-D1) so the next frame self-sizes. End state: spawn/despawn/compaction never round-trip to the CPU for a GPU-resident archetype.

**Why:** The capstone of GPU-resident-zero-readback dominance (the ~3M break-even regime) — it removes the CPU from the structural-op loop entirely for opted-in archetypes, the only way the residency-dominance claim holds at scale. Decoupled-lookback scan is single-pass, work-efficient, ~2n data movement with "throughput approaching that of copy operations," and its single-pass design "enables in-place compaction and in-situ global allocation" — exactly the spawn/despawn primitive needed. Builds on GPU-D1, GPU-D2, and the existing `device_len` counter.

**Caveat:** Highest-risk direction, most in tension with the constraints. (1) Residency is opt-in with a ~3M break-even — GPU-driven structural ops are a NET LOSS below that and MUST NOT become default; the 0%-gate requires a CPU-spawn world byte-identical, so this is a wholly separate GPU path gated by a per-archetype residency flag. (2) Decoupled-lookback assumes forward-progress guarantees not all GPUs/drivers provide ("Decoupled Fallback" 2025 exists precisely to run without them) — a portable impl needs the fallback or a vendor-capability gate. (3) Compaction invalidates cached row indices/handles — ChildOf/relationship remaps and the `(archetype, component)` indirection must survive a GPU-side reshuffle. (4) A CPU system touching the compacted bytes is the forbidden Svelto readback trap. (5) Oracle is golden-buffer diffs + Vulkan validation, not intuition. "Approaching copy throughput" is measured against memcpy in the paper, not against this engine.

**Cite:** Merrill & Garland 2016 (single-pass decoupled-lookback); Decoupled Fallback 2025; "Stream compaction using wave intrinsics"; GPUPrefixSums reference impls; `crates/boyko_ecs/src/ecs/memory/device_column.rs:58`.

---

## Section CPU — CPU / ECS data-layout / SIMD / rigid solver + AI

### CPU-D1 — SoA hot/cold component split — the inviolable substrate `[HAVE]`

**What:** Components stored Struct-of-Arrays per archetype with a deliberate hot/cold split: `RigidBody` (position/velocity/rotation — the only bytes integrate streams) in its own column; `mass`/`material` (`inv_mass`, `inv_inertia`, restitution, friction, body_type) in a SEPARATE `RigidBodyMass` column so it never pollutes integrate cache lines. The solve gathers both into a flat `#[repr(C)] Copy BodyState` SoA snapshot once at the seam boundary. **Direction:** make this the non-negotiable rule for every new CPU subsystem (AI, future solver buffers) — net-new state lands hot/cold-split from day one, never as AoS a later phase has to unpick.

**Why:** The integrate loop's working set is one tight column — stays in L1d, the hardware stride-prefetcher tracks it perfectly. Splitting cold mass out makes a body's hot footprint 24 B (`RigidBody`) not ~44 B — ~1.8× more bodies per cache line on the hottest pass.

**Caveat:** The gather→solve→apply round-trip is itself a full extra streaming pass over all bodies every step — the price of the swappable-solver seam, free only relative to a real solve cost. The split forces two column touches on spawn. Structural, already paid, no gate concern.

**Cite:** `crates/boyko_physics/src/components.rs:18-71`; `crates/boyko_physics/src/resources.rs:91-131`; `crates/boyko_physics/src/systems.rs:92-105,241-269`.

### CPU-D2 — Explicit AVX2/AVX-512 over component columns (alignment already in place) `[PARTIAL]`

**What:** `ComponentPool` guarantees each column's BASE pointer is aligned to `SIMD_BUFFER_ALIGN = 32 B` (AVX2 256-bit aligned load from row 0); `ComponentMask` is `#[repr(align(32))]`. `ChunkedQueryData` already hands `&'c [T]`/`&'c mut [T]` slices per archetype via `Query::for_each_chunk` — the exact shape LLVM autovectorizes and intrinsics consume. **Direction:** write hot CPU kernels (integrate, future constraint solve, AI steering/flow-field sampling) as chunk kernels over those slices: AVX2 baseline, AVX-512 behind `cfg(target_feature="avx512f")`, nightly `portable_simd` (`Simd<f32,8>`) where it measures cleaner than `core::arch`.

**Why:** The alignment guarantee + slice exposure is the hard part and it is DONE; remaining work is kernels. A 4/8-wide integrate/contact-prep over a column is the textbook SIMD win — 8 bodies per AVX2 op. Column-start alignment kills the cross-cache-line penalty on row 0.

**Caveat:** No speedup number — measure per kernel. Per-row alignment beyond `align_of::<T>()` is NOT guaranteed for non-pow2 `T` (e.g. `[f32;3]` = 12 B), so kernels use unaligned interior loads (`_mm256_loadu_ps`); only the column head is aligned. Current physics integrate uses a per-row `par_iter_mut` closure, NOT a chunk kernel — at the mercy of the autovectorizer through a closure boundary (likely scalar). The 0%-gate forbids changing the `row_ptr`/`for_each_chunk`/query-iter asm, so SIMD kernels must be opt-in user code or a separate `_simd` entry, never a rewrite of the generic iter path. Determinism: float SIMD reduction reorders adds (non-associative) — reproducible reductions need a fixed lane-reduction order.

**Cite:** `component_pool.rs:1121-1145`; `constants.rs:26`; `component_mask.rs:7`; `chunked_data.rs:72-81`; `crates/boyko_physics/src/systems.rs:66-70`.

### CPU-D3 — One real AVX2 kernel exists — use it as the house template `[HAVE]`

**What:** The one hand-written SIMD path: `bitset_intersects_avx2` (256 bits = 4×u64/iter via `_mm256_loadu_si256`/`_mm256_and_si256`/`_mm256_testz_si256`) on the scheduler dispatch hot path, with a scalar reference, a `cfg`+`target_feature` gate, a SAFETY comment per intrinsic, and a proptest asserting `SIMD == scalar`. **Direction:** adopt this exact pattern (gated dispatch + scalar correctness reference + differential proptest) as the house template for every future SIMD kernel — SIMD never ships without a scalar oracle to diff against.

**Why:** It proves the toolchain, cfg-gating, and validation discipline all work on this target. The differential test is the only honest way to add SIMD without silent numerical/logic divergence — measurement is the oracle.

**Caveat:** It's a 64-bit integer-AND kernel, not float math; float kernels add the non-associativity/determinism dimension this one never faced. It is currently the *only* one — the breadth of SIMD claimed by the principles is, today, a single function. Runtime CPU detection (`is_x86_feature_detected!`) is unused — dispatch is compile-time `target_feature`, so a generic binary built without `-Ctarget-feature=+avx2` silently takes the scalar path; runtime multiversioning is a future option if shipping one binary across CPUs matters.

**Cite:** `crates/boyko_ecs/src/ecs/core/schedule/bitset_intersects.rs:42-141`, `:193-217`.

### CPU-D4 — Software prefetch for predictable indirect/strided access `[FUTURE]`

**What:** Zero software prefetch exists anywhere (no `_mm_prefetch`, no `core::intrinsics::prefetch_*` — only docs). The hardware stride-prefetcher already covers dense single-column iteration well. The real opportunity is INDIRECT/strided patterns: gather (column→`BodyState` by row), the broadphase pair walk touching `bodies[i]` and `bodies[j]` far apart, future AI blackboard→navmesh-cell hops. **Direction:** prefetch the next chunk/next-pair operands a few iterations ahead (`_mm_prefetch` T0/T1) ONLY on measured indirect/strided paths, never on the prefetcher-friendly dense scan.

**Why:** Software prefetch helps exactly where the hardware prefetcher is blind: predictable-but-non-unit-stride access. On the dense integrate loop it would be pure I-cache/uop waste — principle #3 warns excessive prefetch *lowers* perf.

**Caveat:** Prefetch is the single most over-applied micro-opt; distance tuning is CPU-specific and a wrong distance is a net loss. Gate on a measured cache-miss profile (perf/VTune), not speculation. The all-pairs broadphase it would most help is itself O(n²) and should be replaced by a spatial structure first — prefetching a quadratic loop is polishing the wrong thing.

**Cite:** (absence verified — no prefetch intrinsics in `src`); `crates/boyko_physics/src/systems.rs:118-142` (O(n²) broadphase).

### CPU-D5 — Cache-line alignment / false-sharing padding — disciplined where it matters `[HAVE]`

**What:** The threadpool wraps every cross-worker atomic (`idle`, `active_scopes`, `shutdown`, per-worker handles, injectors) in `CachePadded`; `ComponentMask` is `align(32)`; `Access` is deliberately NOT `align(64)` with a documented rationale (written single-threaded, read-only after). **Direction:** hold this discipline for net-new parallel state — pad ONLY truly cross-thread-written hot atoms, document the decision either way (the Phase-10 review explicitly REMOVED a `CachePadded` that wasted 60 B for zero benefit).

**Why:** False sharing is the classic silent parallel killer; padding read-mostly/single-writer state is pure cache waste. The codebase gets BOTH directions right — the rarer discipline.

**Caveat:** Every `CachePadded<AtomicU64>` burns 56 B of a line. The rule (measure before padding, document when you don't) must extend to future AI/solver per-worker scratch — a per-island solver buffer (one writer) needs NO padding; a shared contact counter would. 0%-gate: padding changes struct size/layout — never add it to a byte-identical hot-path type without re-checking the asm.

**Cite:** `crates/boyko_threadpool/src/thread_pool.rs:116-140`; `crates/boyko_ecs/src/ecs/core/system/access.rs:9-15,59-61`; `docs/PHASE-10-CHANGE-DETECTION-PLAN.md:933-935`.

### CPU-D6 — CPU rigid-body solver: seam built, solver is the headline FUTURE work `[PARTIAL]`

**What:** The seam is in place: `RigidSolver` is a non-object-safe trait (`Resource: Sized`) so `physics_solve_step<S>` monomorphizes to a direct inlinable call (zero vtable); the manifold buffer is dense/sequential in deterministic `(min,max) BodyIndex` order; scratch is SoA + preallocated (no per-step alloc); `substeps` is a reserved config field; a touched-bitmask drives selective write-back. Missing: any real solver — only `NoopSolver` ships. **Direction:** implement the modern small-steps solver inside this seam — XPBD / Catto Soft-Step substep model (many tiny substeps beat many iterations), warm-starting (cache last step's impulse per persistent contact via `feature_id`), constraint islands + sleeping (stop integrating quiescent islands), and SIMD-batched constraint solve (4/8 contacts per AVX lane over the SoA manifold buffer).

**Why:** Small-steps/substep solvers (Catto 2021, XPBD) are state-of-the-art for stability-per-cost; warm-starting slashes iteration count; islands+sleeping make idle scenes nearly free; the dense SoA manifold buffer is already the ideal layout to solve 8 contacts/AVX-lane. Each slots into the existing seam with no core-ECS change — the monomorphized `S::solve` inlines the per-contact loop rather than firewalling it behind a vtable.

**Caveat:** No numbers — solver perf is workload-dependent and unmeasured (`NoopSolver` = no data). DETERMINISM is the hard constraint: contact order is pinned to dense `BodyIndex` = archetype row order, reproducible only under deterministic spawn/despawn (the entity-id counter is a Relaxed atomic shared by parallel Commands workers). A SIMD-batched or island-parallel solve reorders float accumulation (non-associative), so reproducibility needs a fixed reduction order + a content-defined contact key independent of row/id (currently deferred). A pair `(a,b)` writes BOTH rows, so naive parallel pair-solve RACES — islands are the prerequisite for safe parallelism. 0%-gate: `is_noop()` early-out keeps a no-solver world at one branch.

**Cite:** `crates/boyko_physics/src/solver.rs:38-81`; `crates/boyko_physics/src/systems.rs:18-31`, `:211-221`; `crates/boyko_physics/src/resources.rs:30-31`, `:85-131,202-226`.

### CPU-D7 — Rapier/Jolt-FFI behind the `RigidSolver` seam — an option, NOT a default `[FUTURE]`

**What:** Because `RigidSolver` is an open trait, an external solver can implement it on its own Resource type with no edit to `boyko_physics` — a Rapier (Rust, no FFI) or Jolt (C++ via raw FFI) adapter is a drop-in alternative backend behind the SAME seam. The no-third-party rule is scoped to graphics/core; physics is explicitly a swappable-backend subsystem. **Direction:** keep this as an evaluation option / correctness oracle to diff the in-house solver against — not as the shipped path.

**Why:** A mature external solver is a free correctness/perf baseline: implement the adapter, diff against the in-house solver on golden scenes, learn whether the bespoke solver is actually competitive before betting on it. The seam makes the experiment cheap and reversible.

**Caveat:** Rapier brings its own data layout, forcing a copy in/out of `BodyState` every step (the gather/apply tax, doubled) — it won't beat a cache-resident in-house SoA solver on the engine's own layout, and it breaks the zero-third-party aesthetic for a core gameplay system. Jolt is C++ FFI: unsafe ABI surface, build-system weight, a determinism profile you don't control. Neither touches GPU-resident bytes. Strictly oracle/option — the in-house solver is the ONLY shipped solver (OWNER DECISION 2026-06-17: physics is fully in-house). An FFI backend is at most a private offline correctness oracle, never distributed.

**Cite:** `crates/boyko_physics/src/solver.rs:14-19`; `CLAUDE.md` (no-third-party scoped to graphics/core).

### CPU-D8 — AI as a data-oriented CPU subsystem — net-new, design SoA from line one `[FUTURE]`

**What:** No AI/navigation crate exists (no `boyko_ai`/`boyko_nav`; graphify finds only doc references). AI lives on the CPU by first principles — the branchy, heterogeneous, low-N decision logic the GPU is wrong for. **Direction (net-new):** build it cache-resident next to gameplay, NOT a pointer-chasing OOP tree: (1) SoA behavior-tree / blackboard — node state in parallel arrays indexed by agent, blackboard as a typed column not a HashMap; (2) flow-field / hierarchical pathfinding for crowds — one shared flow field amortizes thousands of agents instead of per-agent A*; (3) batched navmesh queries — gather all this-frame queries, run as one column pass, scatter results; (4) avoid the per-agent virtual-dispatch tick that defines mainstream BT engines.

**Why:** Mainstream BT/utility-AI engines pointer-chase a node graph per agent per tick — cache-hostile and unvectorizable, the exact anti-pattern principle #2 forbids. A SoA blackboard + batched navmesh queries + shared flow fields turn AI into column passes that ride the same archetype-iteration + threadpool machinery the engine already has, slotting naturally as ordinary Phase-9 scheduler systems.

**Caveat:** From-scratch subsystem, the largest single item on this axis, unmeasured. Flow-field pathfinding trades per-agent optimality for crowd throughput — wrong for a handful of unique navigators, right for hundreds of homogeneous ones (a routing decision, like mesh-vs-SDF). It must obey the CPU-orchestrate/GPU-execute boundary: an AI system must NOT read GPU-resident bytes (Svelto trap); any GPU-side flow-field eval would be a GPU system feeding CPU AI through an explicit opt-in copy, never an inline readback. Determinism for replays needs fixed agent-iteration order.

**Cite:** (absence verified — no AI/nav crate; graphify "behavior tree blackboard AI pathfinding" returns only doc nodes); GOAL note (CPU owns branchy/heterogeneous/low-N incl. AI).

### CPU-D9 — Scheduler granularity: work-stealing + component-level conflicts HAVE; tune cutoffs, close false conflicts `[HAVE]`

**What:** Phase 9 ships a Chase-Lev work-stealing pool, intra-system `par_iter`/`par_iter_mut` with a `BatchingStrategy` (`entity_count/(workers*batches_per_thread)`, clamped), a `MIN_ARCHETYPE_FOR_PARALLEL=1024` inline cutoff (sub-1024 archetypes run on the calling thread to dodge the ~120 ns spawn cost), and a conflict graph built from COMPONENT-LEVEL read/write bitmasks (512-bit `ComponentMask`) scanned via the AVX2 `bitset_intersects` on every dispatch. **Direction:** (a) replace the hardcoded 1024 cutoff and `batches_per_thread=1` default with measured, possibly per-kernel values (a cheap integrate wants bigger batches than an expensive solve); (b) shrink false conflicts.

**Why:** Component-level (not archetype-level) conflict masks already avoid the coarsest false-conflict class — two systems writing different components of the same archetype run concurrently. The inline cutoff correctly refuses to fork tiny archetypes. The SIMD conflict scan keeps find-ready off the critical path. A solid, measured baseline (4.27 µs @50 systems, ~5× headroom).

**Caveat:** The cutoffs are admittedly pragmatic guesses ("benches will refine if needed") — the right values are workload-specific and unmeasured per-kernel. Remaining false-conflict sources: (1) two systems writing the SAME component but provably DISJOINT archetypes/row-ranges still serialize (conflict is per-component-id, not per-column-range) — Bevy has the same limit; finer access needs archetype/range-level masks at real complexity cost. (2) An ordering hint (`.before`/`.after`) injects a conflict bit even when accesses don't overlap, by design. 0%-gate: any change to `find_ready` must keep that loop byte-identical; tuning lives in `BatchingStrategy`/constants, not the dispatch asm. Determinism: `par_iter` over disjoint rows is order-independent, but a cross-worker reduction (future AI/solver) must use a deterministic combine.

**Cite:** `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:65-126`; `crates/boyko_ecs/src/ecs/core/system/access.rs:45-61`; `crates/boyko_ecs/src/ecs/core/schedule/conflict_graph.rs:97-150`.

---

## Section MEM — Memory / bandwidth / residency economics (the real ceiling)

For GPU-resident the ceiling is VRAM bandwidth (stream N·B/frame, zero readback); for CPU it is cache + DRAM bandwidth. Unifying rule: minimize bytes touched, then keep them where the consumer is. Residency is an economic choice, not a default.

### MEM-D1 — Residency break-even decision rule (~3M entities, opt-in per archetype, never global) `[PARTIAL]`

**What:** Make CPU-vs-GPU-resident an explicit, signature-deterministic economic rule. Three regimes: **A** = CPU-ECS (stream N·B at DRAM bandwidth, burns cores); **B** = upload/compute/readback (a 2·N·B PCIe tax/frame — the Svelto trap, rejected by construction); **C** = GPU-resident + zero-readback (stream N·B at VRAM bandwidth, ~zero CPU). Regime C beats A NOT by a flat VRAM/DRAM ratio but past a break-even of ~3M entities for a single cheap system; the win grows for many-systems-per-frame, stable-residency workloads (particles, SDF eval, boids, mass transforms) where per-frame CPU cost amortizes to a command-buffer record + a ~1–2 µs doorbell. Below break-even, or for few-systems/branchy/low-N work, CPU wins — so residency is per-archetype opt-in (`ArchetypeFlags` bit `1<<11`), classified as a deterministic function of the archetype signature, stamped at BOTH mint sites, one-residency-per-signature-for-life, NOT in the dedup key. A CPU `Access` never matches a GPU archetype, so a CPU system touching GPU bytes is impossible by construction.

**Why:** VRAM bandwidth + zero readback make Regime C the only model that scales to millions of entities without a PCIe round-trip/frame; encoding the break-even as a hard signature-level opt-in prevents the two failure modes that sink naive GPU-ECS engines — paying the residency tax below break-even, and the readback trap. Directly serves the CPU-orchestrate/GPU-execute split.

**Caveat:** The ~3M break-even is a FIRST-PRINCIPLES MODEL, not a measurement — explicitly a critique-fix estimate, sanity-check on a real workload before quoting. It is per-cheap-system; an expensive per-entity system lowers break-even, a system run once per N frames raises it. The seam is built (`PoolBacking` enum, `DeviceColumnHandle`, residency-bit design) but NO production path mints a device pool yet — designed and stubbed, not exercised end-to-end.

**Cite:** `docs/RENDER-PHYSICS-GPU-PLAN.md` §1.1, §2, §5.1–§5.2; `crates/boyko_ecs/src/ecs/memory/device_column.rs:16-29`; `crates/boyko_ecs/src/ecs/memory/component_pool.rs:57-71`.

### MEM-D2 — VRAM bandwidth dominates GPU-resident: minimize bytes touched (hot/cold split on GPU columns, quantize where safe) `[PARTIAL]`

**What:** Once residency removes the PCIe readback, the only remaining cost of a GPU-resident system is streaming N·B/dispatch through VRAM. The lever is bytes-touched, not FLOPs: (a) carry the SoA hot/cold split onto GPU columns too — a position pass touches ONLY the position column, never a fat AoS struct; (b) quantize where a measured visual/precision bar permits (the GPU instance record is already a packed 24 B struct: 2×f32 pos + f32 scale + u32 RGBA8 color + 2×f32 prev_pos, const-asserted no-padding); (c) the device pool is a per-`(archetype,component)` SSBO column resolved each frame through its key, mirroring the CPU SoA columns — column-granular access for free. Dispatch sizing via `vkCmdDispatchIndirect` (GPU-D1) decouples entity count from dispatch count, so bandwidth — not dispatch overhead — is the floor.

**Why:** VRAM bandwidth is a hard ceiling: 32 B/entity at 1M entities = 32 MB/dispatch; halving the bytes via hot/cold split or quantization halves the dominant cost with no algorithmic change. SoA column residency means a pass that needs one field pays for one field — the same cache/bandwidth-locality argument as the CPU side. The 24 B packed instance record is concrete evidence the project already designs for minimal bytes-on-the-wire.

**Caveat:** GPU-side hot/cold split + quantization are DESIGN INTENT, not a general mechanism — the only landed packed-GPU-record is the demo's 24 B `GpuInstance` (Phase 5 columns store raw component bytes 1:1 with the CPU layout). Change detection has NO per-row ticks on device, so a GPU column is data-only — quantization choices can't be validated by the CPU Miri suites and must go through golden-buffer diffs + Vulkan validation. Quantization is a per-field measured tradeoff (R16 vs R8 for brixels deferred to a visual bar), never a blanket policy.

**Cite:** `docs/RENDER-PHYSICS-GPU-PLAN.md` §1, §1.3, §5.4, "Dispatch sizing"; `crates/boyko_demo/src/render/instance.rs:43-70`; `crates/boyko_render/src/gpu_column.rs:896-951`.

### MEM-D3 — SDF memory hierarchy: 1-byte distance + 8³ brick map/atlas + geometry clipmaps `[FUTURE]`

**What:** A three-tier sparse hierarchy turning an impossible dense grid into something resident: (1) **Compact storage** — 1 byte/distance value, clamped to the minimum useful range = half the diagonal of a grid cell (only the surface neighborhood matters), vs 4 B floats. (2) **Sparsity** — keep only cells the isosurface passes through (corners straddle a sign change); store as a brick map (dense grid of pointers to 8³ bricks) + a 3D-texture brick atlas (each texel = one cached distance), chosen over an octree because it maps cleanly to GPU texture fetches; the 8³ pointer grid is trivially small vs the atlas. (3) **LOD** — geometry clipmaps: nested player-centered grids, each level 2× the size per dimension, so brick on-screen size stays ~constant and far regions are evaluated far less often. The engine's own combined figure: a ~2.5 km draw distance would need ~200 trillion brick-map cells dense vs ~20 million with clipmaps — roughly a 10-million-fold reduction. Reconstruction at any point is a single trilinear texture fetch.

**Why:** Dense 1-byte distance at 1024³ already costs ~1 GB and does not scale to an open world; without this hierarchy a mutable-SDF world is not memory-feasible at all. The hierarchy is the entire reason the SDF path can be GPU-resident: the brick atlas is a bounded 3D-texture pool (sized by querying `maxImageDimension3D`, NOT a hardcoded 2048³), and the clipmap caps how many bricks must be re-evaluated per frame — making regen cost ALU-bound, not world-size-bound.

**Caveat:** Entirely DESIGN/RESEARCH — no SDF code on the `ecs` branch; brick pool, atlas, JFA/voxelize regen, clipmap cascade, and BVH are Phase 6+ *after* the zero-readback Vulkan foundation is retired. The ~10M-fold figure is the source engine's own number (Mike Turitzin / Dreams lineage), transcribed honestly, not measured here. The plan corrects a tempting misconception: SDF regen is ALU/JFA-sample-bound (~16 ms estimate, 3 passes × 27 samples over dirty bricks), NOT write-bandwidth-bound — so the cure is fewer JFA passes + tighter dirty-region scoping, not more memory bandwidth. `BRIXEL_FORMAT` defaults to R16, not R8 (R8 only on a measured visual bar).

**Cite:** `docs/sdf-engine-architecture.md` §5, §6, §7, §12; `docs/RENDER-PHYSICS-GPU-PLAN.md` §1.3, §7 Phase 6+, D9.

### MEM-D4 — VM-backed demand-commit pools + non-temporal stores for write-once data `[PARTIAL]`

**What:** Each `ComponentPool` owns ONE contiguous virtual-address reservation laid out `[data | added_ticks | changed_ticks]`, reserved up front (1 GiB target/pool on the 64-bit syscall arms) but committed lazily at the frontier in granule-aligned slabs (`COMMIT_GRANULE = 64 KiB`). The three base pointers are write-once, so growth commits fresh pages WITHOUT copying or moving anything — O(1) in live rows, every handed-out pointer (archetype columns, query tick bases, the `EntityMaster` InlandStore) stays valid for life. Backing: `VirtualAlloc(MEM_RESERVE→MEM_COMMIT)` on Windows / `mmap(PROT_NONE→mprotect)` on Unix / `alloc_zeroed` eager fallback under Miri/wasm. Freshly committed pages read zero by contract (demand-zero), which the engine relies on (NULL `EntityInland` = all-zero, tick storage starts zero). **Forward complement:** non-temporal/streaming stores for write-once data (mass spawn-fill, SoA→GPU staging) that would otherwise pollute L1/L2 with bytes never re-read on the CPU.

**Why:** Address-stable, reserve-once/commit-lazily pools are the load-bearing CPU memory model: zero-copy O(1) growth (no realloc storm under spawn churn), pointer stability (caches and the GPU-column key both depend on bases never moving), near-zero resident cost until rows are used. Non-temporal stores are the natural next lever for write-once paths — a streaming store bypasses the cache hierarchy, so a 1M-entity spawn-fill or staging write does not evict the working set the next system needs.

**Caveat:** The VM reservation + demand-commit + zero-fill + address-stable growth is HAVE and battle-tested (Phase X.G/X.H/X.I, Miri-validated via the eager fallback arm). The reserve target is **1 GiB per pool, NOT 4 GiB** — the 4 GiB figure was the retired shared-Arena reserve (Phase X.F), deleted when Phase X.J gave every pool its own reservation; quoting 4 GiB today is stale (`constants.rs POOL_TARGET_DATA_BYTES = 1 GiB`). Non-temporal/streaming stores and software prefetch are FUTURE — a source grep finds ZERO `_mm_stream`/`movnt`/prefetch intrinsics in `boyko_ecs` or `boyko_render`; mentioned only as principles. Streaming stores need a measured win (they hurt if data IS re-read soon) and a fence discipline.

**Cite:** `crates/boyko_ecs/src/ecs/memory/vm.rs:80-180`; `crates/boyko_ecs/src/ecs/memory/component_pool.rs:122-146`; `crates/boyko_ecs/src/ecs/constants.rs:7,47,90,251`; grep (no non-temporal/prefetch intrinsics in `src`).

### MEM-D5 — Double-buffering prev/cur for interpolation: memory cost vs the zero-readback win `[PARTIAL]`

**What:** To get display-rate smoothness from a fixed 64 Hz sim WITHOUT a per-frame CPU lerp, store both previous and current so the GPU computes `mix(prev, cur, alpha)` in the vertex/compute shader. The landed reference (Phase 20.1, demo) embeds `prev_pos` directly in the GPU record: `GpuInstance` is 24 B = pos(8) + scale(4) + color(4) + prev_pos(8) — `prev_pos` APPENDED so every pre-existing attribute offset stays byte-identical. The shader reads `FixedTime::overstep_fraction()` ∈ [0,1) from an 80 B camera uniform and lerps on-GPU; the CPU only shuffles `prev_pos` once per substep (one writer site, field-granular). The memory-vs-readback trade in miniature: spend +8 B/entity to eliminate any CPU-side interpolation work and any per-frame readback.

**Why:** Interpolation is the canonical case where a small bounded memory cost buys a large bandwidth/CPU win: +8 B of interpolated field removes an entire CPU lerp pass and keeps data GPU-resident — smoothing happens for free during the draw that was going to read the position anyway. The "append, don't reshuffle" discipline (one writer, field-granular) keeps the doubling cost from corrupting the hot per-substep path.

**Caveat:** The landed mechanism is EMBEDDED prev (a wider per-instance record), NOT a separate double-buffered column nor a positional ping-pong — and it is DEMO-side (`boyko_demo`), not a generalized engine facility. A true double-buffer for arbitrary GPU-resident columns (sim N+1 ∥ render N frame-overlap) is explicitly a DEFERRED seam — reserved so it is addable without reshaping, not implemented. The memory cost is only justified for fields actually interpolated; applying it blanket-doubles every column. Load-bearing footgun: a full-struct write inside a per-substep system silently resets `prev=cur` (snap-to-pos) — the single-writer discipline is a documented invariant, not a convention.

**Cite:** `crates/boyko_demo/src/render/instance.rs:1-58`; `docs/FEATURE_MAP.md` (Phase 20.1 GPU mirror); `docs/RENDER-PHYSICS-GPU-PLAN.md` §1.11 + D12.

### MEM-D6 — OPEN QUESTION (BL-1): physical-memory placement / TLB reach `[FUTURE]`

**What:** A standing open perf question, researched and parked. Our pools reserve a large contiguous VIRTUAL range, but the OS hands out PHYSICAL frames lazily and other processes allocate concurrently — do scattered physical frames break cache locality, and can we influence placement? Verified findings (Microsoft Learn, man7, kernel.org, LWN): cache-line locality (L1/L2/L3) is NOT affected by physical fragmentation — a 64 B line always lies within one 4 KB page, and within a page virtual and physical addresses are contiguous, so sequential SoA hot loops are immune by construction. The ONLY real effect is TLB reach (random access over a working set far larger than ~6 MB TLB coverage on 4 KB pages) — a gradual percentage tax, not a cliff. "Push other processes' memory away from ours" is impossible and would not help. Candidate levers, all on OUR OWN memory, opt-in, by measurement only: 2 MB huge pages (≈512× fewer TLB entries — the only mainstream way to guarantee a contiguous locked physical block) and page-locking (`VirtualLock`/`mlock`, residency-only, no contiguity).

**Why:** The honest edge of the memory model: for random-access hot paths (entity lookup, scattered query gather) TLB reach is the one place physical placement could matter, and huge pages are the documented cure. Naming it as an open question keeps the analysis honest and gives a concrete, measurement-gated lever (dtlb_load_misses → opt-in huge-page flag) ready if a profiler ever shows the bottleneck.

**Caveat:** Decision is explicitly DO NOTHING until a profiler shows a real TLB bottleneck (measure before optimizing) — referenced as an open question, NOT a recommendation to implement. Huge pages on Windows need `SeLockMemoryPrivilege` and a single reserve+commit (incompatible with the current lazy commit-into-reserved model — a separate allocation path), and must be allocated at startup before RAM fragments; on Linux `MADV_HUGEPAGE` is unprivileged but transparent. Page-locking guarantees residency only, never contiguity/placement. Do not re-litigate: BL-1 settled that cache-line locality is immune and cross-process placement control is impossible.

**Cite:** `docs/BACKLOG.md` §BL-1.

---

## Section CODEGEN — Codegen / PGO / LTO / inlining / measurement discipline

### CG-D1 — PGO (`-Cprofile-use`) from a deterministic gameplay replay `[FUTURE]`

**What:** Two-build PGO: an instrumented build (`-Cprofile-generate`) runs a FIXED, deterministic gameplay replay (seeded RNG, fixed-timestep tick stream, scripted spawn/despawn/query/schedule mix) to capture an execution profile, which a second release build consumes (`-Cprofile-use=merged.profdata` via `llvm-profdata`). The profile teaches LLVM the real hot/cold split so it lays out `find_ready`, query iter, `row_ptr`, and `for_each_chunk` hot and sinks rare branches (migration, growth-crossing slab commit, error/validation) cold. The deterministic replay already half-exists: fixed timestep (Time/FixedTime + `fixed_advance`) plus the seeded bench worlds in `bench_bevy_vs_boyko`.

**Why:** Principle 3 explicitly endorses PGO. It is the one global codegen lever that improves BOTH halves of principle 3 at once: I-cache layout (hot compacted, cold sunk) AND letting the compiler make inlining/outlining decisions from measured frequencies instead of doctrine — directly serving principle 7. It is the natural capstone of measure-before-optimize because the profile IS the measurement.

**Caveat:** PGO shifts the MEAN, not the variance — never share a baseline with non-PGO numbers; capture a SEPARATE profiled baseline or every A/B delta is contaminated (`docs/BENCHMARKING.md` lists PGO as deferred for exactly this reason). The profile is only as honest as the replay is representative: a replay that under-exercises migration/growth gets those paths mis-sunk and a real workload hitting them pays MORE. The profile is build-host/CPU/source-specific and goes stale on any hot-path edit — it needs a re-capture step, not a checked-in blob. Keep FUTURE until a stable representative replay exists; a premature profile is worse than none.

**Cite:** `docs/BENCHMARKING.md` (Deferred: PGO); `CLAUDE.md` principle 3; Phase 20 fixed timestep.

### CG-D2 — LTO: codegen-units=1 for benches (HAVE) vs fat/thin-LTO for release (FUTURE) `[PARTIAL]`

**What:** Three distinct knobs kept separate. (1) `[profile.bench] codegen-units=1` is SET in the root `Cargo.toml` — deterministic codegen so two builds of the same source emit identical machine code, which makes the byte-identical-`cargo-asm` 0%-gate mechanically checkable; explicitly NOT `lto`. (2) Release LTO is NOT set anywhere (no `[profile.release]` in the workspace). (3) The candidate FUTURE direction: thin-LTO or fat-LTO on `[profile.release]` ONLY for the shippable engine binary, where cross-crate inlining across the `boyko_ecs`/`boyko_threadpool`/`boyko_render`/`boyko_rhi_vulkan` boundary matters (RHI trait calls + generic query monomorphizations inlining through crate walls — principle 7 notes `#[inline]` is needed precisely so LTO can SEE the body).

**Why:** codegen-units=1 in `[profile.bench]` is the load-bearing prerequisite for the entire byte-identical-asm 0%-gate discipline — without deterministic codegen you cannot assert a hot loop stayed identical. For release, the engine is a workspace of many small crates with heavy generics and a trait-object RHI seam — exactly the shape where LTO recovers cross-crate inlining that crate boundaries block.

**Caveat:** Fat-LTO over the Bevy dependency in `bench_bevy_vs_boyko` explodes compile time and would make the A/B loop unusable — `docs/BENCHMARKING.md` and the `Cargo.toml` comment both call this out, which is why `lto` is deliberately absent from `[profile.bench]`. So release LTO must be scoped to `[profile.release]`, kept OFF for `[profile.bench]`, and like PGO it shifts the mean — never compare an LTO build's numbers against a thin build's. Thin-LTO is the safe first step (most of the win, a fraction of the compile cost); reach for fat-LTO only if a measured hot path shows a missed cross-crate inline.

**Cite:** `Cargo.toml:12-20` (`[profile.bench] codegen-units=1`, "This is NOT lto"); `docs/BENCHMARKING.md`; `CLAUDE.md` principle 7.

### CG-D3 — Measured inlining: `#[inline]` cross-crate/generic, `#[inline(always)]` only on asm evidence, `#[cold]` for error paths `[HAVE]`

**What:** A standing per-call-site policy, not a global switch. `#[inline]` (soft hint) on trivial cross-crate/generic methods so their bodies are visible to LTO/the inliner across crate walls. `#[inline(always)]` (hard demand) ONLY where a profiler or `cargo-asm` shows the compiler is NOT inlining on its own AND it measurably matters. `#[cold]`/`#[inline(never)]` on error/validation/rare branches (migration, growth-crossing, panic/anyhow surfaces) to keep them out of the hot I-cache. The protected hot loops — `row_ptr`, `for_each_chunk`, query iter, `find_ready` — are the named 0%-gate byte-identical targets.

**Why:** Principle 7 verbatim + a recorded working agreement. The anti-pattern is concrete and damaging: blind `#[inline(always)]` bloats the hot path, evicts the L1i working set, and LOWERS performance — principle 3's I-cache half is violated by over-inlining, not served by it. Inlining is a measurement decision, enforced as a red flag in review.

**Caveat:** There is no automated detector for over-inlining — caught only by `cargo-asm` inspection of the named hot loops + benches, so it relies on the discipline holding. `#[inline(always)]` without a cited asm/profiler justification is a review red flag by standing agreement; it READS as an optimization while silently regressing I-cache. Conversely, REMOVING `#[inline]` from a small cross-crate/generic fn can hide the body from LTO and regress just as silently — both directions need the byte-identical-asm gate to verify, not intuition.

**Cite:** `CLAUDE.md` principles 7 and 3; MEMORY feedback-inlining-nuanced + feedback-cache-optimization; `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs`.

### CG-D4 — The measurement-oracle stack: criterion + bench.ps1 median-of-N + mimalloc; golden-buffer + Vulkan validation-as-test for the un-Miri-able GPU half `[HAVE]`

**What:** A layered oracle so nothing optimizes on faith. **CPU:** criterion benches with `bench.ps1` running median-of-N (cargo pinned High-priority across all logical cores), opt-in mimalloc (`--features bench-alloc`) to strip Windows system-heap variance for A/B signal while default `cargo bench` keeps the system heap for honest absolutes; `critcmp` for before/after deltas. **GPU (Miri cannot reach it):** bit-exact golden-buffer round-trips (`round_trip.rs:44` asserts device-column readback == expected bytes) and the Vulkan validation layer wired to FAIL tests — `debug.rs` runs `VK_LAYER_KHRONOS_validation` + a `VK_EXT_debug_utils` messenger that COUNTS every WARNING/ERROR, and `assert_validation_clean(ctx)` turns a non-zero count into a test failure (`sync_validation.rs` Test A: lowered barrier is validation-clean AND correct; Test B: missing-barrier case trips sync-validation OR a wrong result).

**Why:** Measurement is the project's stated oracle and this stack covers both unsafe-but-Miri-able CPU code and the GPU half Miri can't touch. The serialization SerPod lesson is the canonical proof: a silent fast-path demotion (array-POD components quietly routed to per-row serialize) READ as correct in every functional test — only the perf measurement caught it. The discipline is no speculative optimization: a change isn't an improvement until criterion (CPU) or a golden-buffer/validation diff (GPU) says so. mimalloc-for-variance vs system-heap-for-absolutes is the same honesty discipline as PGO's separate-baseline rule.

**Caveat:** The GPU oracle is conditional on the host: sync-validation may be unavailable (no `VK_VALIDATION_FEATURE_ENABLE` / older loader), so `sync_validation.rs` deliberately does NOT assume it and falls back to a wrong-result assertion — a green run on a box without sync-validation is weaker evidence, so CI must surface which layers were actually active. On the bench side, a single before/after pair on this noisy Windows box is untrustworthy (documented ±20–30% system-heap swing); the median-of-N protocol + turbo-lock are MANDATORY, and concurrent bench/Miri jobs invalidate each other (hard project rule). mimalloc numbers are signal-extraction only — never reported as production absolutes.

**Cite:** `docs/BENCHMARKING.md`; `Cargo.toml:19-20`; `crates/boyko_render/tests/round_trip.rs:44,56`; `crates/boyko_render/tests/sync_validation.rs:136-238`; `crates/boyko_rhi_vulkan/src/debug.rs`; MEMORY project-serialization-perf-serpod.

### CG-D5 — Const-eval / monomorphization branch elision (`const { }` gates) `[PARTIAL]`

**What:** Per-monomorphization compile-time branch removal using associated consts in `const { }` position. The query hot path already does this: `if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION }` and `if !const { F::IS_ARCHETYPAL }` const-fold the entire change-detection / non-archetypal-filter branch OUT of a Query whose type parameters don't need it (`iter.rs:298,529`; `par_iter.rs:672,697,731`); the `QueryData`/`QueryFilter` traits carry the gating consts (`IS_READ_ONLY`, `NEEDS_CHANGE_DETECTION`, `HAS_DATA_COMPONENT`, `REQUIRES_POST_FILTER_TRIM`). **Forward direction:** extend the same pattern to new pay-for-what-you-use features — GPU-residency-per-archetype, lifecycle-hook presence (`HAS_HOOKS` already exists on Component), SDF-vs-mesh routing — so a world/query/archetype that doesn't use a feature has its branch compiled away to nothing rather than tested at runtime.

**Why:** This is the mechanical enforcement of the INVIOLABLE 0%-gate: `const { }` in `if` position folds the branch at monomorphization, so a query/world not using a feature pays literally zero instructions — not even one `test/jz` — strictly better than a runtime flag for type-keyed features. It is how the hot loops stay byte-identical across feature additions: a feature behind a const gate cannot perturb the asm of a monomorphization that doesn't instantiate it. `ArchetypeFlags` bit-tests already cover the runtime-keyed cases (hooks/observers); const gates cover the type-keyed ones.

**Caveat:** `const { }` elision only works when the predicate is a compile-time const for that monomorphization — runtime-keyed state (per-archetype GPU residency decided at spawn, observers registered at runtime) CANNOT be const-folded and must stay a runtime bit-test (the 0%-gate's "at most one test/jz" floor); conflating the two is a design error. Each new const gate multiplies monomorphizations (code-size / compile-time / I-cache cost) — a poorly-chosen gate can bloat the binary and HURT I-cache, so it pays only where the branch is genuinely hot and the type-keying is real. Verification is mandatory: only `cargo-asm` on the gated hot loop proves the branch actually vanished — a const that fails to fold (a non-const operand sneaks in) silently degrades to a runtime branch that still reads as correct.

**Cite:** `crates/boyko_ecs/src/ecs/core/iters/query/iter.rs:214,298,460,529`; `par_iter.rs:672,697,731`; `data.rs:118,153`; `crates/boyko_ecs/src/ecs/core/component/component.rs:51,89`; `CLAUDE.md` principle 3 + the 0%-gate.

---

## References

**HW ray tracing / SDF rendering**
- NVIDIA, *Ray Tracing of Signed Distance Function Grids* (JCGT, 2022) — https://jcgt.org/published/0011/03/06/paper-lowres.pdf
- Khronos, `VK_KHR_acceleration_structure` — https://registry.khronos.org/vulkan/specs/latest/man/html/VK_KHR_acceleration_structure.html
- NVIDIA, *RTX Best Practices* — https://developer.nvidia.com/blog/rtx-best-practices/
- NVIDIA, *HW vs SW-accelerated ray tracing* — https://blogs.nvidia.com/blog/whats-the-difference-between-hardware-and-software-accelerated-ray-tracing
- *A Diligent Approach to Ray Tracing* — https://www.gamedeveloper.com/programming/a-diligent-approach-to-ray-tracing
- Schied et al., *Spatiotemporal Variance-Guided Filtering (SVGF)* (2017) — https://cg.ivd.kit.edu/publications/2017/svgf/svgf_preprint.pdf

**Software tracing / culling / temporal**
- *Journey to Lumen* — https://knarkowicz.wordpress.com/2022/08/18/journey-to-lumen/
- UE Lumen technical details — https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- RasterGrid, *Hierarchical-Z occlusion culling* — https://www.rastergrid.com/blog/2010/10/hierarchical-z-map-based-occlusion-culling/
- *Experiments in GPU-based occlusion culling* — https://interplayoflight.wordpress.com/2017/11/15/experiments-in-gpu-based-occlusion-culling/
- *Reprojection TAA* — https://brashandplucky.com/2023/05/06/reprojection-temporal-antialiasing.html
- UE *Temporal Super Resolution* — https://dev.epicgames.com/documentation/unreal-engine/temporal-super-resolution-in-unreal-engine
- *Cache-friendly deferred / visibility buffer* (JCGT) — https://jcgt.org/published/0002/02/04/paper.pdf
- *Deferred Texturing* — https://www.reedbeta.com/blog/deferred-texturing/
- *Aokana* (visibility-buffer packing) — https://arxiv.org/html/2505.02017v1

**GPU compute / async / compaction**
- Vulkan spec, `vkCmdDispatchIndirect` — https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/vkCmdDispatchIndirect.html
- Khronos, *Vulkan subgroup tutorial* — https://www.khronos.org/blog/vulkan-subgroup-tutorial ; subgroups guide — https://docs.vulkan.org/guide/latest/subgroups.html
- *Stream compaction using wave intrinsics* — https://interplayoflight.wordpress.com/2022/12/25/stream-compaction-using-wave-intrinsics/
- AMD GPUOpen, *Concurrent execution / asynchronous queues* — https://gpuopen.com/learn/concurrent-execution-asynchronous-queues/ ; *RDNA performance guide* — https://gpuopen.com/learn/rdna-performance-guide/
- Vulkan *async_compute* sample — https://docs.vulkan.org/samples/latest/samples/performance/async_compute/README.html
- NVIDIA, *Advanced API performance: async compute and overlap* — https://developer.nvidia.com/blog/advanced-api-performance-async-compute-and-overlap
- *Async compute all the things* — https://interplayoflight.wordpress.com/2025/05/27/async-compute-all-the-things/
- Merrill & Garland, *Single-pass parallel prefix scan with decoupled look-back* (2016) — http://www.mgarland.org/papers/2016/scan/
- *Decoupled Fallback* (portable single-pass scan, 2025) — https://dl.acm.org/doi/10.1145/3694906.3743326
- GPUPrefixSums (reference impls) — https://github.com/b0nes164/GPUPrefixSums

**Physics**
- Erin Catto, *Soft Step / small-steps solver* (Box2D v3, 2021) — Catto's GDC talks / Box2D-v3 release notes
- *XPBD: Position-Based Simulation of Compliant Constrained Dynamics* (Macklin et al.)

**Memory / hardware**
- LWN 944115 (huge pages); Microsoft Learn (Large-Page-Support / `AllocateUserPhysicalPages` / `VirtualLock`); man7 `mlock`; kernel.org transhuge — full sources catalogued in `docs/BACKLOG.md` §BL-1

**Repo design docs**
- `docs/RENDER-PHYSICS-GPU-PLAN.md` — residency economics, regimes A/B/C, dispatch sizing, deferred seams
- `docs/sdf-engine-architecture.md` — SDF memory hierarchy, brick map/atlas, geometry clipmaps
- `docs/BACKLOG.md` §BL-1 — physical-memory placement / TLB findings
- `docs/BENCHMARKING.md` — median-of-N, mimalloc, PGO/LTO deferral rationale
- `CLAUDE.md` — principles (esp. 3 and 7) and the 0%-gate
