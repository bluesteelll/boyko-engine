# Research: Graphics optimizations for boyko-engine's deferred + SDF-hybrid render path

Status: **research report** (forward-looking; prioritized/sequenced plan keyed to the SHIPPED render path).
Companion to `docs/PERF-DIRECTIONS.md` (cross-references its RT-*/GPU-D* IDs, does not duplicate the catalog).
Our system: in-house RHI (raw-FFI Vulkan 1.3, dynamic rendering, no ash/wgpu), deferred G-buffer + mesh raster +
SDF sphere-trace shared-depth hybrid (rungs 1–11 shipped).

## The shipped reality (what optimizations attach to)
rungs 8–11 sphere-trace into a packed `RWStructuredBuffer<uint>` (header → edits → a DEPTH region copied from the
rasterized D32 attachment → packed-RGBA pixels), then present via a fullscreen-sample blit. The deferred MRT
G-buffer (OQ-B) is committed in DESIGN but the only on-screen frame today is a single-binding compute composite.
**First-tier opts target this compute spine (where the engine lives), not a not-yet-built MRT raster G-buffer.**

## TIER 1 — build first (high impact, fits the existing compute spine)
- **1.1 Hierarchical tile-cull / coarse pre-trace (RT-4).** A 1/8-res coarse pass (each pixel = an 8×8 tile)
  cone-traces once/tile → conservative per-tile `near_t` + empty flag; the fine march starts from `near_t` and
  skips empty tiles (Claybook's coarse cone-trace; Lumen's two-level DF). Removes the dominant empty-space march
  cost (`O(edits)×O(steps)`/pixel from t=0). **Highest impact-vs-effort; no acceleration structure; purely
  additive to the single-binding packed-buffer layout** (new region between DEPTH and PIXEL, host-mirrored with a
  const-assert like rung-10). Reuse the mesh depth as the per-tile far bound.
- **1.2 Half-res trace + depth-aware upscale (RT-6 first half).** Quarters the march; pairs with 1.1.
- **1.3 Barrier-lowering audit + batching (GPU-D4b).** Co-requisite of 1.1/1.2 — we lower our own barriers (no
  wgpu auto-sync); batch into single `vkCmdPipelineBarrier` calls; keep `sync_validation` Test A/B green. Not optional.

## TIER 2 — build second (high multiplier, needs new infra)
- **1.4 Temporal reprojection / TAA of the trace (RT-6 second half).** Jitter + reproject via motion vectors into a
  history buffer. Multiplies the ray budget AND is the **hard prerequisite for confirmed-future RT-lighting** (few-
  sample RT is noise without temporal accumulation → SVGF). Needs motion vectors (derivable from the Phase-20.1
  prev/cur interpolation), a history texture (→ graduate to image-backed targets), rejection heuristics.
- **1.5 Graduate composite from packed-buffer to MRT G-buffer images.** depth(D32)/normal/albedo/material as real
  attachments — the deferred G-buffer (OQ-B) "for real." Removes the per-frame depth image→buffer copy (rung-10),
  enables hardware depth-test, unblocks 1.4/1.6. SDF compute writes STORAGE images (OQ-5); mesh writes COLOR/DEPTH;
  SHARE the depth image (§15.1) instead of copying through a buffer. **The packed buffer is a test-harness shortcut,
  not the OQ-B destination — treating it as final would block the RT-lighting foundation.**
- **1.6 Clustered/tiled deferred light culling.** Froxel cull in compute → per-cluster **bitfield** light lists
  (`u32[ceil(N/32)]`/tile, bounded — Granite) + subgroup scalarization (load light data into SGPRs → occupancy).
  Scales lighting to many lights. Needs 1.5 + subgroup exposure (GPU-D2).

## TIER 3 — threshold-gated (only when scene complexity demands; see roadmap below)
1.7 brick atlas + brick-map (MEM-D3, SDF §6); 1.8 geometry clipmap LOD (SDF §7); 1.9 BVH dirty-region regen + JFA
(SDF §8); 1.10 Hi-Z two-pass occlusion culling for the mesh side (RT-5, needs GPU-driven indirect).

## TIER 4 — capability-gated optional backend (confirmed-future RT-lighting; seam NOW, build later)
1.11 HW-RT for SDF primary visibility (RT-1); 1.12 RT lighting/shadows/AO/GI/reflections (RT-3, owner-confirmed).

## SDF acceleration roadmap (the invariant that protects it)
analytic edit-list (HAVE, correct to ~dozens of edits) → tile-cull (1.1, any real scene — empty-space dominates) →
brick atlas (1.7, when per-pixel `O(edits)` is the wall; AMD Brixelizer 2024 validates: 64³ cascade voxels → sparse
8³ bricks 0-1 distance → per-cascade AABB tree → detailed-to-coarse traversal) → clipmap (1.8, vast worlds only) →
BVH dirty-region regen (1.9, only when GPU-authoritative + large + mutating). **Invariant:** the marcher always
asks "distance at `p`" + "is this tile empty"; whether the answer is analytic eval / trilinear brick fetch /
clipmap-selected brick / dirty-refreshed brick, **the shared-depth seam (§15.1) and the per-tile cull (1.1) never
change** — keep one `field_distance(p)` / `tile_bound(tile)` interface so the whole hierarchy is hot-swappable.

## The RT story (foundation choices to make NOW)
1. Keep the deferred G-buffer as the shading substrate (forward would force reworking shading when RT lands).
2. Reserve a motion-vector + history-buffer slot (1.4) — RT-lighting's SVGF denoiser IS the TAA infra.
3. Keep the SDF edit/brick AABB list as the natural BLAS source (NVIDIA "Ray Tracing of SDF Grids" JCGT 2022:
   BVH over non-empty bricks as `VK_GEOMETRY_TYPE_AABBS_KHR` + intersection shader; don't choose a brick layout
   that can't emit AABBs).
4. Keep the software sphere-tracer (1.1/1.2) as the universal fallback (Lumen's HW-or-software split).
**Defer:** the AS/RT RHI seam itself (`api.rs` has no `AccelerationStructure` type — large raw-`VK_KHR_*` FFI with
no consumer until the brick atlas exists to feed it); RT-1 before bricks; TLAS mesh+SDF unification (RT-2 — the
shared-depth composite already gives correct mesh↔SDF occlusion for free). **Caveats to encode:** AS rebuild is the
worst case for a mutating SDF (refit only for modest motion) → region routing (HW-RT for static + secondary rays,
software trace for the actively-edited near field, Lumen-style); RT cost moves to secondary-ray shading divergence;
async-compute AS-build overlap depends on the multi-queue path that doesn't exist yet.

## Raw-Vulkan-specific debt (foundations later opts fight)
- **4.1 Barrier minimization** — superset-correct lowering over-syncs; missing `INDIRECT_COMMAND_READ`/`ALL_COMMANDS`/
  `MEMORY_*` stage/access constants (GPU-D4). Batch barriers; narrow only where provably minimal; `sync_validation`
  is the oracle. Incremental per pass.
- **4.2 Async compute (GPU-D3/RT-8)** — single queue, fence-only submit, NO semaphores, no queue-family query.
  Async helps only with unused warp slots (the Vulkan sample win is ~5%, not order-of-magnitude); net-stall
  anti-patterns exist. **Defer** until a profile shows under-occupancy; it's the same lift that unblocks RT-8.
- **4.3 Descriptor/bindless** — fixed single compute descriptor set today (why everything packs into one buffer).
  MRT/clustered/brick need multi-resource bind groups (seamed in S0: `bind_descriptor_set`/`create_bind_group`).
  Bindless (`VK_EXT_descriptor_indexing`, core 1.2) when material count / RT demands it — not premature.
- **4.4 GPU-driven indirect (GPU-D1/D6)** — `dispatch_indirect` is a `#[cold]` no-op stub; needed for device-written
  dispatch counts, Hi-Z mesh culling (1.10), the GPU-driven structural-ops capstone. Mesh shaders only when the
  mesh side is non-trivial.

## Sequenced plan (the critical path)
A tile-cull (1.1, M) → B barrier audit (1.3, S) → C half-res+upscale (1.2, M) → D MRT G-buffer (1.5, L) → E image
bind-groups (4.3, M) → F motion vectors + TAA (1.4, L) → G clustered lighting (1.6, L) → H async-compute (4.2, XL,
profile-gated) → I+ brick/clipmap/BVH (threshold-gated) → J+ AS/RT seam → HW-RT → RT-lighting (capability-gated).
**A→B→C→D→F is the spine** that turns the buffer-packed golden-test composite into a real, temporally-stable,
half-res-amortized deferred renderer — and F doubles as the RT-lighting denoiser foundation. Each step gated by
golden-image-equal + GPU-timestamp.

## Conflicts to flag
None of Tiers 1–2 conflict with in-house/native/raw-Vulkan/no-wgpu (additive compute passes + the planned graphics
surface; 0%-gate honored — passes only run with a `boyko_render` schedule). The packed-buffer composite is a
TEMPORARY test shortcut → step D is required to honor OQ-B + unblock TAA/clustered/RT. Async (H) profile-gated, never
speculative. Bindless/mesh-shaders not premature for an SDF-dominated path. SDF §8 incremental regen is "not strictly
correct in all cases" (author's caveat). HW-RT must stay opt-in capability-gated with the software tracer as fallback.

## Sources
- Aaltonen, "GPU-Based Clay Simulation & Ray-Tracing in Claybook" (GDC 2018) — the hierarchical-tile SDF tracer (1/8-res coarse cone-trace). https://media.gdcvault.com/gdc2018/presentations/Aaltonen_Sebastian_GPU_Based_Clay.pdf
- Narkowicz, "Journey to Lumen" (2022) — software DF trace as the fastest method; two-level DF. https://knarkowicz.wordpress.com/2022/08/18/journey-to-lumen/
- AMD FidelityFX Brixelizer/Brixelizer GI (GDC 2024) — production sparse-brick SDF (validates SDF §6-8). https://gpuopen.com/fidelityfx-brixelizer/
- Turitzin, "Hierarchical Depth Buffers" — Hi-Z + the odd-dimension correctness pitfall. https://miketuritzin.com/post/hierarchical-depth-buffers/
- AMD FidelityFX SPD — single-dispatch mip-pyramid build. https://gpuopen.com/fidelityfx-spd/
- Arntzen, "Clustered shading evolution in Granite" — decoupled XY-Z, bitfield light lists, subgroup scalarization. https://themaister.net/blog/2020/01/10/clustered-shading-evolution-in-granite/
- NVIDIA, "Ray Tracing of SDF Grids" (JCGT 2022) — BVH over bricks (AABB BLAS) + intersection shader; the RT-1 architecture. https://jcgt.org/published/0011/03/06/paper-lowres.pdf
- Schied et al., SVGF (2017) — the RT-lighting denoiser (couples to 1.4 TAA). https://cg.ivd.kit.edu/publications/2017/svgf/svgf_preprint.pdf
- Vulkan async_compute sample (~5% win); NVIDIA "async compute and overlap"; AMD GPUOpen concurrent-execution/RDNA guides; vkguide GPU-driven; Khronos `VK_KHR_acceleration_structure` (single-BLAS-one-geometry-type → TLAS unification).
- Reed, "Deferred Texturing" (visibility-buffer bandwidth model); Ortiz, "Clustered Shading primer."
