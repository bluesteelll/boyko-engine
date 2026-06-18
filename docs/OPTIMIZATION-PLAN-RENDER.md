# Render Optimization Plan — boyko-engine

> Status: implementation-ready, sequenced. From the shipped deferred + SDF-hybrid base (rungs 1–11) to a production-scale renderer. Optimization-focused. Companion to `docs/RESEARCH-GRAPHICS-OPT.md` (priority/sequence) and `docs/PERF-DIRECTIONS.md` (RT-*/GPU-D* IDs). Destination: `docs/OPTIMIZATION-PLAN-RENDER.md`.
>
> **Changelog vs the reviewed draft** (every critic point resolved):
> - **C1** — P3 split into P3a (on-screen hand-FFI `swapchain.rs` barrier batching, the real per-frame win) + P3b (`boyko_rhi::enums` constants + `boyko_render::barrier.rs` narrowing, serving the compute-column path of P8/P11). Citations corrected: on-screen barriers are 11 hand-FFI `cmd_pipeline_barrier` sites in `swapchain.rs`, NOT `lower_barriers`.
> - **C2** — Added **P0**: instantiates the production scene substrate (resolution as a dispatch dimension, perspective camera, edit-list source) the spine optimizes. Every spine win is now re-stated as structural-until-P0-measured, with explicit target regimes.
> - **C3** — P5/P6/P9 are now **forked passes**; the golden-frozen `sdf_depth_composite` marcher + its host mirror `golden_composite_pixel` + the scalar field eval (`sdf`/`smin`/`combine`/normal) stay byte-identical, preserving the physics-reuse contract. Tolerance relaxation promoted from an Open Question to a per-phase owner-approval gate.
> - **W1** — P1 win reframed as structural; the fabricated "8 MB/frame at 1080p" removed (the fixture copies 16 KB).
> - **W2** — P2 merged into P1 as its first sub-step (descriptor vocabulary precedes the MRT wiring it requires). Critical-path list corrected.
> - **W3** — P8 scoped to render-local indirect (Hi-Z visible-tile count); the per-archetype `device_len`/GPU-D6 residency capstone is explicitly excluded.
> - **W4** — Added a windowed-present synchronization model section + the semaphore-present seam decision (lands with P6's first cross-frame state).
> - **W5** — Effort re-estimated: P1 (now incl. P2 + the shader rewrite) = XL; P3a = M.
> - **O1** — Phases split into "spine" vs "parallel-after-P1" tracks structurally.
> - **O2** — P9 brick format = R16 default (R8 only on a measured visual bar).
> - **O3** — Octahedral normal noted as a golden-baseline reset, not a free swap.

## 0. Shipped reality this plan attaches to (verified in-repo)

| Subsystem | State | Cite |
|---|---|---|
| On-screen frame | Compute composite → packed `RWStructuredBuffer<uint>` → `present_sampled` fullscreen blit | `swapchain.rs:1661` `record_present_sampled`; `sdf_depth_composite.hlsl:78` |
| SDF marcher | **Golden fixture**: hardcoded `IMG_W=IMG_H=64u` (4096 px), `MAX_SDF_EDITS=16u`, `MAX_IT=128u`, ORTHOGRAPHIC, one binding, CPU-written edits; folds full edit-list from `t=0`, bounded only by shared mesh depth | `sdf_depth_composite.hlsl:86-87,104,117,204,247-252` |
| Field math = golden source of truth | `sdf`/`smin`/`combine`/normal mirrored EXACTLY host-side (`golden_composite_pixel`); **a future CPU physics evaluator reuses the SAME field math** | `sdf_depth_composite.hlsl:8-12,47-51` |
| MRT G-buffer | Offscreen golden-tested rungs only (`gbuffer.fs`/`deferred_light.fs`, `color_formats: &[Format]` MRT). NOT the on-screen path | `rhi_impl.rs`; `gbuffer.fs.hlsl`; `deferred_light.fs.hlsl` |
| Shared depth | Real D32 prepass → `copy_image_to_buffer` into a DEPTH region (64×64 = 16 KB/frame) | `swapchain.rs:2071` `sync_depth` |
| Queue | ONE graphics+compute family, `queue_count:1`, **fence-only `submit`, NO semaphores** (`submit_windowed` is an unbuilt seam) | `device.rs:1634` `find_queue_family`, `:1750`; `queue.rs:3-5,22,31` |
| On-screen barriers | **Hand-FFI** `(self.fns.cmd_pipeline_barrier)`, one image barrier per call, **11 sites** in `record_scene`/`sync_depth`/`record_present_sampled` | `swapchain.rs:844,914,1169,1201,1337,1370,1432,1708,1831,1864,1926` |
| ECS-edge barriers (distinct path) | `lower_barriers` lowers conflict-graph edges into `PlannedBarrier` POD keyed by `(ArchetypeId, ComponentId)`, replayed by `GpuSystem` — the **compute-column** path, does NOT touch the on-screen frame | `boyko_render/barrier.rs:1-15,58-76,196-233` |
| Barrier constants | `boyko_rhi::enums` lacks `DRAW_INDIRECT`/`INDIRECT_COMMAND_READ`/`ALL_COMMANDS`/`MEMORY_*`; widen is `COMPUTE_SHADER\|TRANSFER` only; `GpuStage::Indirect` widens (barrier.rs:121-124) | `enums.rs:101`; `barrier.rs:36,121-124` |
| Bind groups | `create_bind_group`/`_layout` are **COMBINED_IMAGE_SAMPLER-only**; compute path binds exactly one storage buffer | `rhi_impl.rs:717`; `encoder.rs:38` `bind_storage_buffer` |
| Indirect | `dispatch_indirect` `#[cold]` no-op stub; `BufferUsage::INDIRECT` exists | `encoder.rs:269`; `enums.rs:37` |
| HW-RT | No `AccelerationStructure`/`RayTracingPipeline` associated type | `api.rs:46-67` |

**Inviolable gates carried into every phase:** 0%-gate (a world not using `boyko_render` pays nothing; named CPU hot loops byte-identical); in-house only (raw-FFI Vulkan 1.3, no ash/wgpu); native (not web); golden-image-equal + GPU-timestamp + validation/sync-validation clean on RTX 3060 as the GPU oracle; every `unsafe` carries `// SAFETY:`; **the deterministic scalar field eval (`sdf`/`smin`/`combine`/normal) and `golden_composite_pixel` stay byte-identical — no fast math, no reordered FMA — because physics collision reuses them (RESEARCH-FAST-MATH determinism boundary).**

---

## 1. Critical path (dependency-ordered, two tracks)

```
─── SPINE (graduate the fixture into a real renderer) ───────────────────────────
P0  Production scene substrate: resolution-as-dispatch-dim, perspective cam,      [L]  ← MUST precede every "ray-budget" win
    edit-list source from ECS; defines the target regimes the spine measures
P1  MRT G-buffer (incl. P2 descriptor vocabulary as sub-step 1) + shared-depth    [XL] ← foundation; subsumes ex-P2
    image + kill the depth→buffer copy + the marcher single→multi-binding rewrite
P3a On-screen barrier batching (hand-FFI swapchain.rs)                            [M]  ← co-req of P1's 4-pass frame
P4  Hierarchical tile-cull / coarse pre-trace (behind field_distance/tile_bound)  [M]  ← needs P0+P1
P5  Half-res trace + depth-aware upscale  (FORKED path; golden marcher frozen)    [M]  ← needs P0+P4
P6  Motion vectors + history + TAA  (FORKED path; semaphore-present seam lands)   [L]  ← needs P0+P1+P5; RT denoiser base
─── PARALLEL-AFTER-P1 (independent of the SDF spine) ─────────────────────────────
P3b boyko_rhi::enums constants + boyko_render::barrier.rs narrowing               [S]  ← serves P8/P11 compute-column path
P7  Clustered/tiled deferred light culling (froxel + bitfield + subgroup)         [L]  ← needs P1 only
P8  GPU-driven indirect dispatch (render-local count buffer; NOT device_len)      [M]  ← needs P3b; prereq for P11
─── THRESHOLD- / CAPABILITY- / PROFILE-GATED (build only when measured needed) ──
P9  SDF brick atlas (field backend swap behind P4 invariant)            [XL] threshold ← needs P4 invariant
P10 Clipmap LOD over bricks                                              [L]  threshold ← needs P9
P11 Hi-Z two-pass mesh occlusion culling                                [L]  threshold ← needs P8 + P1
P12 BVH dirty-region regen + JFA                                         [XL] threshold ← needs P9
P13 Async compute (multi-queue + cross-queue semaphores)                [XL] profile   ← needs P3a/P3b + P6's seam
P14 AS/RT seam + HW-RT SDF-as-AABB-BLAS backend                         [XL] capability← needs P9 (bricks=BLAS)
P15 RT lighting (shadows/AO/GI/reflections) + SVGF                      [XL] confirmed ← needs P6 (denoiser) + P14
```

**The spine is P0→P1→P3a→P4→P5→P6.** P0 instantiates the cost; P1 graduates the buffer-packed fixture into a real attribute-image deferred renderer (and is the only place the marcher's single-binding shader is rewritten); P3a cuts the per-frame barrier tax on the new 4-pass frame; P4→P5→P6 amortize the march. P6 is the RT-lighting denoiser foundation. P3b/P7/P8 parallelize after P1. P9–P15 are gated.

Why this order, concretely:
- **P0 absolutely first.** The shipped marcher is a 64×64/≤16-edit golden fixture (verified `IMG_W=IMG_H=64u`, `MAX_SDF_EDITS=16u`); it is not march-step-bound at any measurable scale and reads a CPU array, not the ECS world. Every "tile-cull removes empty space / half-res quarters it / TAA multiplies the budget" claim is **unmeasurable until P0 instantiates a resolution, a perspective camera, and an edit feed.** Building P4/P5/P6 before P0 designs amortization layers for a cost that does not exist (foundations-before-APIs).
- **P1 second** because the packed `u32` buffer conflates depth+attributes+output into one binding and forces a per-frame depth image→buffer copy; every downstream opt (tile far-bound, motion vectors, clustered lighting, RT) needs real attribute images. P1 also necessarily rewrites the single-binding `RWStructuredBuffer<uint>` marcher to write STORAGE images (the descriptor vocabulary, ex-P2, is its first sub-step).
- **Tile-cull (P4) before half-res (P5) before TAA (P6)**: tile-cull removes the dominant empty-space march, half-res quarters what remains, TAA amortizes across frames. TAA-first would temporally-stabilize an un-culled, full-res, full-cost march.
- **Indirect (P8) before Hi-Z (P11)**: Hi-Z writes device-side visible-tile counts the CPU must not read back.
- **Bricks (P9) before HW-RT (P14)**: the non-empty-brick AABB list *is* the BLAS source; building the AS seam before bricks gives it no consumer.

---

## 1b. Windowed-present synchronization model (resolves W4)

The spine introduces a multi-pass on-screen frame and cross-frame state, but `submit` takes **no semaphores** (`queue.rs:22,31`; `submit_windowed` is an unbuilt seam). Two distinct synchronization scopes:

- **Intra-frame (one command buffer):** the 4 passes (MRT raster → SDF compute → deferred lighting → present blit) synchronize via **pipeline barriers within a single recorded command buffer** — exactly the existing `record_scene` model. No semaphore needed. P1/P3a operate entirely here. **This is sufficient for P0–P5.**
- **Cross-frame (acquire→render→present chain + history):** P6's history buffer and P11's "last frame's visible set" introduce a dependency on the *previous frame's* output. A correct swapchain present needs the acquire-image / render-finished **semaphore chain** that `submit` cannot express today. **Decision: the `submit_windowed` semaphore-present seam lands as P6's first sub-step** (P6 is the first phase that needs cross-frame state). Until then, P0–P5 use the fence-only single-command-buffer model (correct, just serializes frames — acceptable pre-P6). P13 (async) extends this seam to cross-queue semaphores; it does not invent it.

This is **not async** — it is basic windowed-present correctness, and the seam's absence is now an explicit P6 precondition rather than a silent gap.

---

## PHASE P0 — Production scene substrate (resolve C2) — SPINE

**What.** Turn the golden fixture into a renderer that has a resolution, a camera, and a scene feed:
1. **Resolution as a dispatch dimension.** Replace `IMG_W=IMG_H=64u` static consts with dispatch-dimension/uniform-driven width/height; the marcher reads its extent, not a compile-time constant. The 64×64 golden fixture remains as a frozen test (a fixed-extent invocation), but the production path is resolution-parametric.
2. **Perspective camera.** Add a perspective ray-generation path alongside the golden-frozen orthographic one. The ortho convention is golden-frozen (rung-8..11) — perspective is a **new, additive** ray-gen mode selected by a camera-mode uniform, never a modification of the ortho path.
3. **Edit-list source from ECS.** Define how SDF edits reach the marcher: the CPU collects `SdfEdit` components from the ECS world into the edit buffer at frame setup (the current CPU-push path, generalized from a hardcoded array to an ECS query). The GPU-resident-column feed (MEM-D2) is explicitly **out of scope** here (it is the GPU-D6 residency pillar, W3).

**Why our path needs it.** Without a resolution/camera/scene, the spine's ray-budget wins are unmeasurable (the critic's C2 blocker): a 64×64/≤16-edit fixture is not march-step-bound, so no gate of the form "GPU-timestamp shows fine-march step reduction on a sparse scene" can be evaluated. P0 instantiates the cost the spine amortizes.

**Target regimes the spine measures against** (stated so every later gate is falsifiable):
- **Resolution:** 1920×1080 production target; 1280×720 as the fast-iteration profile.
- **Edit-count regime:** "scene" = 256–4096 active `SdfEdit`s (the threshold band where the analytic `O(edits)` fold starts to hurt and P9 bricks become a candidate). The ≤16-edit fixture is the "tech-demo / bricks-lose" floor.
- **Empty-screen fraction:** measured per scene; the tile-cull (P4) win scales with it. Two canonical scenes: **sparse** (geometry clustered, ~70% empty screen — the P4/P5 best case) and **dense** (geometry fills the frame — the honest worst case showing ~no P4 win).

**How.** Marcher extent + camera-mode + edit-count become push-constants/uniforms (the single-binding packed buffer already carries `count`; P1 graduates this to a proper uniform). The perspective ray-gen is a forked `[branch]` on camera-mode, leaving the ortho path's instruction sequence untouched so the rung-8..11 goldens stay bit-exact.

**Expected win.** None directly — P0 is the cost-instantiation prerequisite. It makes P1's "removes the per-frame depth copy" measurable at a real resolution and P4/P5/P6's ray-budget gates evaluable.

**Dependency/order.** First on the spine. Everything else's "expected win" lines are unsubstantiated until P0 lands.

**Gate.** The rung-8..11 goldens (ortho, 64×64) stay **bit-exact** (the parametric extent + the additive perspective branch must not perturb the frozen ortho path); a 1080p perspective dispatch produces a validation-clean frame; an ECS-fed scene of N edits renders (correctness vs a host reference at small N); GPU-timestamp baselines recorded for sparse + dense scenes at 720p/1080p (the numbers every later phase measures against).

**Effort.** L. **Conflicts.** None — additive ray-gen mode + parametric extent; the golden-frozen ortho path is byte-untouched (0%-gate + determinism honored). **Principle note:** perspective ray-gen must not introduce fast-math into the field eval — only ray *generation* changes; `sdf`/`smin`/`combine` are byte-identical.

---

## PHASE P1 — MRT G-buffer + descriptor vocabulary + shared-depth image + kill the depth→buffer copy + marcher single→multi-binding rewrite (subsumes ex-P2) — SPINE

**What.** (Sub-step 1, ex-P2) Generalize the descriptor seam from "fixed single compute storage buffer" + "COMBINED_IMAGE_SAMPLER-only" (`rhi_impl.rs:717`) to a small typed multi-resource bind-group vocabulary: `{StorageImage, SampledImage, CombinedImageSampler, StorageBuffer, UniformBuffer}` in arbitrary per-set combination, bindless-ready (descriptor-indexing reserved behind a `DeviceCaps` query) but not bindless yet. Add a compute-pipeline descriptor-set bind point (today the compute path binds exactly one storage buffer, `encoder.rs:38`).
(Sub-step 2) Replace the packed-`u32`-buffer composite with a real G-buffer of images: `depth (D32_SFLOAT)`, `normal (R8G8B8A8_UNORM` — see O3/Open-Q3 for octahedral`)`, `albedo (R8G8B8A8_UNORM)`, `material (R8G8B8A8_UNORM: roughness/metalness/flags)`. **Rewrite the marcher from `RWStructuredBuffer<uint>`-only to write STORAGE images** and read the SHARED depth image directly (not via a copied buffer region). Mesh raster writes COLOR attachments + DEPTH; the deferred-lighting pass (`deferred_light.fs`) consumes the G-buffer.

**Why our path needs it.** The packed buffer is a single-binding test-harness shortcut that conflates depth+attributes+output and forces a `copy_image_to_buffer` of the D32 attachment into a buffer region **every frame** (`sync_depth`, swapchain.rs:2071) — because the marcher physically *cannot sample an image* (it has only `RWStructuredBuffer<uint> Buf`). The MRT G-buffer is the committed OQ-B destination; treating the packed buffer as final blocks hardware depth-test, motion vectors, clustered lighting, and the entire RT-lighting foundation. The descriptor vocabulary (sub-step 1) is a **hard prerequisite** of the MRT wiring (sub-step 2 needs a set with 3 storage images + 1 sampled depth + 1 uniform), which is why ex-P2 is now P1's first sub-step (resolves W2).

**How (optimal for raw Vulkan 1.3 + our RHI).**
- *Descriptor (sub-step 1):* `BindGroupLayoutDesc` → `&[BindGroupLayoutEntry { binding, kind, stage, count }]`; map each `kind` to its `VkDescriptorType` at the cold create boundary. `BindGroupDesc` → `&[BindGroupEntry]` (image-view + optional sampler, or buffer + offset + range), written once with `vkUpdateDescriptorSets` at create (no per-frame rewrite). Add a `BindPoint`/compute sibling to `bind_descriptor_set` (today GRAPHICS-only, `encoder.rs:168`). Seam-with-default-`#[cold]`-body pattern (the `copy_buffer`/`image_barrier` template) keeps Mock + ABI stable.
- *Passes (sub-step 2):* (1) geometry MRT raster (`gbuffer.vs/fs`, `color_formats=[normal,albedo,material]`, depth) → G-buffer + depth; (2) SDF compute marcher bound to G-buffer STORAGE images + the depth as a sampled image, writing surface attributes where it wins the §15.1 depth seam — now via the depth IMAGE; (3) deferred-lighting fullscreen pass (`deferred_light.fs`) → lit image; (4) present blit (`present_sampled`).
- *Images:* G-buffer `COLOR_ATTACHMENT | STORAGE | SAMPLED`; depth `DEPTH_STENCIL_ATTACHMENT | SAMPLED`.
- *Sync:* intra-frame pipeline barriers in one command buffer (§1b); batched by P3a.

**Expected win (structural, not a fabricated number — resolves W1).** Removes the per-frame depth image→buffer copy + its transfer→compute barrier (**16 KB/frame at the current fixture, scaling with resolution** — NOT the previously-quoted 8 MB, which assumed a 1080p path that does not yet exist pre-P0). Graduates the renderer off the single-binding packed buffer so depth is an image (zero-copy occlusion) and attributes are column-granular (MEM-D2, GPU-image side). Unblocks P4/P6/P7/P14. No marcher-speed change yet (that is P4/P5).

**Dependency/order.** Foundation; needs P0 (resolution-parametric marcher to graduate). Prereq for P3a (the 4-pass frame it batches), P4, P6, P7.

**Gate.** Golden-image-equal (±2/255, rung-10 tolerance) between the new G-buffer composite and the packed-buffer composite on crater_csg/box_csg/smooth_union + mesh-occludes-SDF; the depth→buffer copy is **GONE** from the per-frame command stream (recording inspection); a compute pipeline bound to a 3-storage-image + 1-sampled-depth + 1-uniform set produces the golden (descriptor-type/layout mismatches are a validation fault — the oracle catches them); no per-frame `vkUpdateDescriptorSets` in the steady stream; validation + sync-validation clean on RTX 3060.

**Effort.** **XL** (resolves W5: subsumes ex-P2's descriptor vocabulary M + the full marcher single-binding→STORAGE-image shader rewrite, which is a rewrite not a wiring change). **Conflicts.** None — additive, on-screen path only under the `boyko_render` schedule (0%-gate). The packed buffer + the 64×64 golden marcher stay as the offscreen golden harness (the determinism reference). **Determinism:** the marcher's field eval (`sdf`/`smin`/`combine`/normal) is **copied verbatim** into the image-writing variant — byte-identical, preserving the physics-reuse contract; only the output target (buffer word → storage image) and the depth source (buffer region → sampled image) change.

---

## PHASE P3a — On-screen barrier batching (hand-FFI `swapchain.rs`) (resolves C1) — SPINE co-requisite

**What.** Batch the **hand-FFI** per-frame barriers in `swapchain.rs`. Today every transition is its own `(self.fns.cmd_pipeline_barrier)` call — **11 sites** across `record_scene` (844, 914, 1169, 1201, 1337, 1370, 1432), `record_present_sampled` (1708, 1831, 1864, 1926), one image barrier each. Where a sync point transitions N images at once (e.g. the color+depth UNDEFINED→OPTIMAL transitions in `record_scene`, or the multi-image transitions P1's 4-pass frame adds), lower them into **one `vkCmdPipelineBarrier` with N `VkImageMemoryBarrier`s** instead of N calls.

**Why our path needs it (corrected attribution — C1).** The reviewed draft mislocated this win in `boyko_render/src/barrier.rs` (`lower_barriers`). That file is a **different codepath**: it lowers the ECS conflict-graph's abstract edges into `PlannedBarrier` POD keyed by `(ArchetypeId, ComponentId)`, replayed by `GpuSystem` on the compute-column path — it never touches the on-screen frame (verified: barrier.rs:1-15,196-233). The per-frame barrier tax the spine pays is the **11 hand-FFI calls in `swapchain.rs`**, and batching is achievable **only** there. "Barriers can drain the GPU of work" — merging N transitions into one call lets the driver compute one merged dependency. P1's 4-pass frame adds new sync points; doing them with a batched primitive in hand keeps the call count flat.

**How.** Refactor the `swapchain.rs` barrier sites to accumulate a `&[VkImageMemoryBarrier]` (+ any `VkBufferMemoryBarrier`) per sync point and issue a single `cmd_pipeline_barrier`. The UNDEFINED→OPTIMAL color+depth pair in `record_scene` merges into one; P1's raster→compute→lighting transitions each become one batched call. Each merged barrier keeps the existing (sound) stage/access masks — this phase only batches, it does not narrow (narrowing is P3b, on the other codepath).

**Expected win.** Fewer `cmd_pipeline_barrier` calls/frame (driver merges one dependency vs several); measured by command-stream inspection (call count) + GPU-timestamp (neutral-or-better — batching never regresses correctness, the masks are unchanged).

**Dependency/order.** Co-requisite of P1 (batch P1's new 4-pass transitions as they land). On the spine.

**Gate.** Golden-image-equal (the batched barriers must preserve the exact dependencies — a dropped/weakened barrier is sync-validation UB); barrier-call count per frame measurably lower (command-stream inspection — this is the gate the mis-attributed draft could not meet, now meetable because it targets the correct codepath); validation + sync-validation clean on RTX 3060; GPU-timestamp neutral-or-better.

**Effort.** **M** (resolves W5: was understated as "S" — it is a hand-FFI refactor across 11 sites + P1's new transitions, with sync-validation as the FAIL oracle, not a one-function change). **Conflicts.** Batching fights soundness only if a merge drops a dependency — mitigated by keeping each merged barrier's masks identical to the pre-merge set and by sync-validation as a test-failing oracle. 0%-gate neutral (record-time only).

---

## PHASE P3b — `boyko_rhi::enums` constants + `boyko_render::barrier.rs` narrowing (resolves C1) — PARALLEL-AFTER-P1, serves P8/P11

**What.** (a) Add the missing `boyko_rhi::enums` stage/access constants the foundation lacks (verified absent, barrier.rs:36): `BarrierStage::{DRAW_INDIRECT (0x0000_0002), ALL_COMMANDS (0x0001_0000), ALL_GRAPHICS}`; `BarrierAccess::{INDIRECT_COMMAND_READ (0x0000_0001), MEMORY_READ, MEMORY_WRITE, SHADER_STORAGE_READ, SHADER_STORAGE_WRITE}` — identity-cast `u32`, each documented with its `VK_*` source. (b) Narrow the superset-widen in `boyko_render/src/barrier.rs` (`stage_of`/`access_of`/`wide_*`) only where producer/consumer (stage, access) is provably unambiguous — notably `GpuStage::Indirect` stops widening to `COMPUTE_SHADER|TRANSFER` (barrier.rs:121-124) and maps to `DRAW_INDIRECT` / `INDIRECT_COMMAND_READ`. Keep the widen helpers as the documented fallback.

**Why our path needs it (corrected scope — C1).** This is the **compute-column codepath** (the ECS-edge `lower_barriers`), distinct from P3a's on-screen frame. Its constants + narrowing serve **P8 (indirect dispatch)** and **P11 (Hi-Z device-written counts)**: today `GpuStage::Indirect` over-widens because no `DRAW_INDIRECT` constant exists, blocking a tight count-buffer barrier. The narrowing reduces false serialization between compute and transfer on the GPU-resident-ECS path (qualitative — AMD/NVIDIA guidance, measured per-pass with GPU-timestamps).

**How.** Extend the `enums.rs` bitflag families (identity-cast, `#[inline] bits()`, `VK_*`-documented). In `barrier.rs`: where the producer intent has exactly one touch on the consumer's key with a known stage/access, emit the precise mask; otherwise widen. `stage_of(Indirect)` returns `BarrierStage::DRAW_INDIRECT`. Keep `wide_stage`/`wide_access` as the fallback.

**Expected win.** Tighter masks reduce false serialization on the compute-column path; unblocks P8/P11's count-buffer barrier. Structural for P8/P11.

**Dependency/order.** Parallel-after-P1 (independent of the SDF spine and of P3a — different codepath). Prereq for P8/P11.

**Gate.** `sync_validation.rs` Test A (lowered barrier validation-clean AND correct) + Test B (a deliberately-missing barrier trips sync-validation) both green with the narrowed masks; the `Indirect→DRAW_INDIRECT` narrowing keeps both green; validation/sync-validation clean.

**Effort.** **S** (constants + the narrowing; the narrowing is the only soundness-sensitive part — sync-validation is the oracle, narrow conservatively). **Conflicts.** Narrowing fights soundness; mitigated by the widen fallback + sync-validation as a FAIL oracle. 0%-gate neutral (build-time lowering only).

---

## PHASE P4 — Hierarchical tile-cull / coarse pre-trace of the SDF march (RT-4) — SPINE

**What.** A 1/8-res coarse pass: each coarse "pixel" represents an 8×8 fine-pixel tile and cone-traces the field once → a conservative per-tile `near_t` + an `empty` flag. The fine marcher starts from `near_t` instead of `t=0` and early-outs empty tiles. The per-tile far bound is the G-buffer mesh depth (now an image, P1).

**Why our path needs it.** After P0 instantiates a real resolution and edit count, the marcher folds the FULL edit-list at every step from `t=0` for every pixel (`MAX_IT=128`), most steps crossing empty space — `O(pixels)·O(steps)·O(edits)`. This is the dominant cost before any acceleration structure, and tile-cull needs none (purely additive to the field eval). The coarse pass costs 1/64 the ray count; it removes the empty-space prefix from all 64 fine rays per tile. Claybook's 1/8-res coarse cone-trace + Lumen's two-level DF target exactly this — highest impact-vs-effort.

**How (optimal).**
- *Pass 0 (coarse):* a compute dispatch of `ceil(W/8)·ceil(H/8)` invocations; cone-trace the tile-center ray with a tile-radius cone (conservative widening), write `TileBound { near_t, far_t, flags }` to a tile SSBO (P1's vocabulary). `far_t` clamped by the tile's max mesh depth (min/max over the tile from the depth image).
- *Pass 1 (fine):* the existing marcher, modified: `t = TileBound[tile].near_t; if (flags & EMPTY) { write background/mesh; return; }`. The field eval is **byte-identical**.
- *The invariant (the load-bearing decision):* both passes call the SAME `field_distance(p)` (the `sdf()` fold) + a NEW `tile_bound(tile)`, in one shared HLSL include `sdf_field.hlsli`. When the backend swaps to bricks (P9) / clipmap (P10), only `field_distance`/`tile_bound` change — tile-cull, the shared-depth seam, half-res, TAA, lighting are byte-untouched. **`sdf_field.hlsli` is the verbatim cut of the determinism-frozen `sdf`/`smin`/`combine`/normal — it is shared with the host `golden_composite_pixel` and the future physics evaluator, so it carries the no-fast-math contract.**
- *Buffer layout:* the tile SSBO is a standalone buffer (post-P1's vocabulary), host-mirrored with a const-assert like rung-10's `COMPOSITE_*_BASE_WORDS` so a desync is a build error.

**Expected win.** Removes the empty-space march prefix; the coarse pass is ~1.5% of fine ray count. Measured by GPU-timestamp on the **P0 sparse scene** vs the **P0 dense scene** (the latter is the honest ~no-win worst case).

**Dependency/order.** Needs P0 (a resolution + scene where the prefix is measurable) + P1 (depth as an image for the far bound); benefits from P3a (batched coarse→fine barrier). Prereq for P5.

**Gate.** Golden-image-equal (±2/255) vs the P1 composite on all SDF scenes — the cull must be **conservative** (a wrongly-empty tile is a visible hole, the golden catches it); a deliberately-too-aggressive cull trips the golden (Test-B negative); GPU-timestamp shows fine-march step reduction on the P0 sparse scene; validation clean.

**Effort.** M. **Conflicts.** None; additive, gated behind `boyko_render`. The conservative-bound requirement is the soundness surface — the golden is the oracle. **Determinism:** `field_distance` is the frozen field eval; the coarse cone-trace uses it unchanged.

---

## PHASE P5 — Half-res trace + depth-aware upscale (RT-6 first half) — FORKED PATH (resolves C3) — SPINE

**What.** A **separate, forked marcher path** that runs the fine SDF march at half resolution (1/4 rays), then upscales with a depth-aware (bilateral) filter using the full-res G-buffer depth/normal to avoid edge bleeding. Jitter the half-res grid per-frame (sets up P6).

**Why our path needs it.** After P4 removes empty space, the remaining cost is the per-ray field fold at hit-adjacent depths; halving resolution quarters it. The depth-aware upscale keeps edges crisp via the full-res depth (P1).

**How — the fork is mandatory (C3).**
- **The golden-frozen `sdf_depth_composite` marcher + its host mirror `golden_composite_pixel` + the scalar field eval (`sdf`/`smin`/`combine`/normal) are byte-identical** — P5 does NOT modify them. The half-res path is a **new compute pass** that *calls* the shared `sdf_field.hlsli` field eval (unchanged) at jittered/half-res ray origins. Jittering the camera/ray-origin changes the *points p* at which the (deterministic) `sdf(p)` is sampled — it does not change `sdf` itself. The rung-8..11 goldens (fixed ortho ray per pixel) are pinned to the frozen path and survive untouched.
- The half-res marcher writes a half-res surface buffer (distance/normal/albedo); a full-res upscale compute pass reads it + the full-res G-buffer depth/normal and does a 4-tap joint-bilateral upscale (weights from depth/normal similarity) into the full-res G-buffer SDF region.
- Half-res grid jittered by a per-frame Halton offset (the seed P6 reprojects). A tile straddling a depth discontinuity (P4's per-tile "complex" flag) falls back to full-res for those pixels (`[branch]`).

**Expected win.** ~4× fewer fine rays on SDF-surface pixels; net frame win by GPU-timestamp on the P0 sparse scene.

**Dependency/order.** Needs P0 + P4 (tile structure + conservative bounds) + P1 (full-res depth for the bilateral weights). Pairs into P6.

**Gate (tolerance is an owner-approval gate — C3).** Golden within a **RELAXED tolerance** (half-res + upscale is not bit-exact) — the new SSIM/PSNR bar (e.g. ±4/255 on smooth regions + an explicit edge-pixel no-bleed check at the mesh↔SDF seam) is a **per-phase owner-approval item** (RESEARCH-FAST-MATH lists "accept a one-time determinism-baseline reset" as an explicit architect→owner question, not critic-discretion). **Mandatory additional exit criterion:** the deterministic `sdf`/`smin`/`combine`/normal field functions and `golden_composite_pixel` are **byte-identical** (verified by diff) — the physics-reuse contract is unaffected. GPU-timestamp shows the ray reduction; validation clean.

**Effort.** M. **Conflicts.** The relaxed tolerance is the first deliberate departure from bit-exact — confined to the *consumer* (ray generation/resolution/blend), never the field eval. The forked path leaves the golden harness as the reference. **Owner sign-off on the relaxed bar is a hard gate.**

---

## PHASE P6 — Motion vectors + history + TAA / temporal reprojection (RT-6 second half) — FORKED PATH (resolves C3, W4) — SPINE

**What.** A motion-vector G-buffer attachment, a history color buffer, and a TAA resolve pass (reproject prev frame via motion vectors, blend under neighborhood-clamp rejection, accumulate). Motion vectors derive from the Phase-20.1 prev/cur interpolation (the GPU already has `prev_pos`). **Lands the `submit_windowed` semaphore-present seam (W4) as its first sub-step** — P6 is the first phase with cross-frame state.

**Why our path needs it.** TAA multiplies the effective ray budget (jittered samples accumulate temporally), converging the half-res march (P5) cheaply. **It is the hard prerequisite for confirmed-future RT-lighting (P15):** few-sample RT is pure noise without temporal accumulation — SVGF (the RT denoiser) IS the TAA infrastructure (motion vectors + history + reprojection + variance-guided blend). Building TAA now means the RT denoiser is ~80% built when RT lands.

**How.**
- *Semaphore-present seam (sub-step 0, W4):* build `submit_windowed` (acquire-image / render-finished semaphores) — `submit` is fence-only today and cannot express the cross-frame present chain P6's history needs. P13 later extends this to cross-queue; it does not invent it.
- *Motion-vector attachment:* an `R16G16_SFLOAT` G-buffer target. Mesh raster writes `clip_cur − clip_prev` (prev MVP + `prev_pos`). For the **static analytic field + moving camera**, the SDF hit's motion vector is the camera reprojection of the hit world position (deterministic per frame given the camera — see Open-Q2; a GPU-mutating field, P12, needs per-hit velocity, deferred until then).
- *History buffer:* a double-buffered full-res color image (ping-pong, the MEM-D5 seam) — bounded memory, justified only for the resolved output, NOT blanket-doubling every G-buffer column.
- *TAA resolve:* a compute/fullscreen pass — sample current, reproject history via motion vector, clamp history to the current 3×3 neighborhood AABB (YCoCg variance clamp), blend `lerp(history, current, α≈0.1)`, write the new history + the present source.
- *Rejection:* disocclusion (depth mismatch), out-of-bounds reprojection, and rapidly-edited-SDF regions (a dirty flag) fall back to current-frame-only.
- *Jitter:* the P5 half-res Halton jitter becomes the TAA sub-pixel jitter; the projection is jittered per frame and un-jittered in the resolve.
- **The field eval (`sdf`/`smin`/`combine`/normal) + `golden_composite_pixel` stay byte-identical (C3)** — P6 is a forked consumer; TAA is temporally non-deterministic *by design* (that is what TAA is), but render history non-determinism is OUTSIDE the physics gate, and the SDF-field reuse coupling is already severed by P5's fork.

**Expected win.** Stable converged image at the half-res cost; effective-sample multiplier; RT denoiser foundation.

**Dependency/order.** Needs P0 + P1 (image G-buffer + depth + motion-vector attachment) + P5 (the jittered half-res input). Doubles as the P15 denoiser base. **Lands the semaphore-present seam P13 extends.**

**Gate.** Golden on a STATIC camera + static field converges to the P5 reference (temporal accumulation of identical frames = identity); a CAMERA-PAN golden shows no ghosting beyond a documented bar (clamp working); a disocclusion test (object reveal) shows no smearing (rejection working); **the field eval + `golden_composite_pixel` are byte-identical (diff-verified)**; replay-determinism note: render history non-determinism does NOT enter the physics/solver determinism gate (render is outside that gate; the field reuse is already forked); the semaphore-present chain is sync-validation clean; GPU-timestamp for the added attachment + resolve; validation clean.

**Effort.** L (+ the semaphore-present seam sub-step). **Conflicts.** TAA introduces temporal-lag/ghosting (documented; clamp + rejection bound them). The history double-buffer is bounded (MEM-D5). **Owner-approval gate for any new relaxed tolerance** (same as P5). **Determinism: render history may be non-deterministic; the physics-reused field eval may NOT — and is not touched.**

---

## PHASE P7 — Clustered / tiled deferred light culling (froxel + bitfield + subgroup) — PARALLEL-AFTER-P1

**What.** Partition the view frustum into froxels (screen-tile XY × depth-slice Z); a compute pass culls the light list against each cluster and writes a per-cluster **bitfield** light list (`u32[ceil(N_lights/32)]` per cluster, bounded, Granite-style). The deferred-lighting pass (P1's `deferred_light` generalized) reads its cluster's bitfield and iterates only set bits, with subgroup scalarization (`WaveReadLaneFirst` when a wave shares a light) for occupancy.

**Why our path needs it.** `deferred_light.fs` applies ONE hardcoded directional light today. A production scene has many; without clustering, deferred lighting is `O(pixels·lights)`. Froxel culling makes it `O(pixels·lights_per_cluster)`. The bitfield is bounded (no per-cluster Vec); subgroup scalarization cuts per-lane light fetches by wave width.

**How.**
- *Cluster build:* a compute pass writes per-cluster bounds (or computes them from the froxel grid). Light SSBO (P1's vocabulary) holds position/radius/color.
- *Cull pass:* one invocation per cluster (or per light, scatter); test light sphere vs cluster AABB; set the light's bit.
- *Lighting pass:* `deferred_light` reads the pixel's cluster, walks the bitfield (`firstbitlow`), accumulates. Scalarization: when `WaveActiveAllEqual(cluster)`, load each light once per wave into SGPRs.
- *Subgroup exposure (GPU-D2):* expose `VK_SUBGROUP_FEATURE_{BASIC,BALLOT,ARITHMETIC}` via the device-create caps query (none today); shaders query `SubgroupSize`, never hard-code; capability-gate the ballot/arithmetic path with a **scalar fallback**.

**Expected win.** Many-light scaling; lower lighting bandwidth/divergence. Measured by GPU-timestamp on a 256-light scene vs the single-light baseline.

**Dependency/order.** Needs P1 (G-buffer + depth for Z-slices + light/cluster SSBO bind group + subgroup exposure). **Independent of P0/P4/P5/P6** — parallel-after-P1.

**Gate.** Golden on a known multi-light scene (host-computed reference) within ±2/255; a light fully outside a cluster contributes zero (cull correctness); the bitfield path matches a brute-force-all-lights path bit-for-bit (differential test, CPU-D3 house template applied to GPU); the subgroup path matches the scalar fallback bit-for-bit (differential, capability-gated); validation clean; GPU-timestamp shows sub-linear scaling in light count.

**Effort.** L. **Conflicts.** None; additive. Subgroup ops live in SPIR-V (0%-gate-neutral on CPU). The capability gate + scalar fallback are mandatory (wave width is HW-variable).

---

## PHASE P8 — GPU-driven indirect dispatch (render-local count buffer; NOT `device_len`) (resolves W3) — PARALLEL-AFTER-P1

**What.** Fill the `dispatch_indirect(buffer, offset)` stub (`encoder.rs:269`) with a real `vkCmdDispatchIndirect`. A prior compute pass writes `VkDispatchIndirectCommand {x,y,z}` into a small **render-owned** device-local INDIRECT buffer (e.g. the P11 Hi-Z visible-tile count); the cull/compaction passes self-size from it.

**Why our path needs it (scoped — W3).** For GPU-driven culling (P11 Hi-Z writes a device-side visible-tile count) the CPU must not read the count back. **Explicitly excluded:** the per-archetype/per-pass `device_len` counter and the `GpuSystem` residency machinery — that is the **GPU-D6 residency capstone** (CPU-orchestrate/GPU-execute ECS, ~3M-entity break-even, opt-in, NOT default), a separate pillar with its own break-even gate. P8 imports only `vkCmdDispatchIndirect` + a render-local count buffer; it cites `gpu_system.rs` only to note the stub, not to adopt its counter model.

**How.**
- Vulkan backend: `cmd_dispatch_indirect(cmd, buffer, offset)`.
- A RAW barrier so the count buffer is visible as `INDIRECT_COMMAND_READ` (P3b's constant) at the `DRAW_INDIRECT` stage — replacing the current full-widen for `GpuStage::Indirect`.
- Count buffers created with `BufferUsage::INDIRECT` (exists, `enums.rs:37`).
- Keep the synchronous CPU-count path untouched (the stub is `#[cold] #[inline(never)]`, no foundation code calls it — 0%-gate honored).

**Expected win.** Structural decoupling (dispatch count from CPU knowledge); prereq for P11. NOT a per-dispatch speedup.

**Dependency/order.** Needs P3b (`INDIRECT_COMMAND_READ`/`DRAW_INDIRECT` constants). Prereq for P11. Parallel-after-P1; independent of the SDF-accel and lighting tracks.

**Gate.** A compute pass that writes a count → indirect dispatch consuming it produces the same golden as a CPU-count dispatch of the same size; the `INDIRECT_COMMAND_READ` barrier is validation/sync-validation clean; the zero-count case is handled (no dispatch when the count buffer is 0); validation clean.

**Effort.** M. **Conflicts.** None; the seam is `#[cold]`, no hot path touched. Costs an extra RAW barrier (P3b's narrow constant keeps it tight). **W3 boundary honored: no `device_len`/GPU-D6 residency work under this render banner.**

---

## PHASE P9 — SDF brick atlas (field backend swap behind the P4 invariant) — THRESHOLD-GATED

**What.** Replace the analytic edit-list fold with a sparse brick representation: a brick-map (dense grid of indices to 8³ bricks) + a brick atlas (a 3D texture pool, sized by `maxImageDimension3D`, NOT hardcoded) storing distances near the isosurface. `field_distance(p)` becomes a trilinear brick fetch; `tile_bound(tile)` becomes a per-brick AABB test. Empty space = "no brick", skipped free.

**Why / threshold.** The analytic fold is `O(edits)` per eval; correct to ~dozens of edits. **The threshold:** when per-pixel `O(edits)` is the wall — i.e. when edit-count × march-steps dominates the frame even after P4 + P5, at the **P0 256–4096-edit regime**. Below that (≤16-edit fixture) the analytic path is faster and bricks are pure overhead. AMD Brixelizer (2024) validates the architecture (64³ cascade → sparse 8³ bricks → per-cascade AABB tree).

**How — the invariant is the whole point.**
- P4's tile-cull + the shared-depth seam ask ONLY `field_distance(p)` + `tile_bound(tile)` (the `sdf_field.hlsli` include). P9 swaps the *implementation* of those two from analytic to brick-fetch. **The marcher, tile-cull, half-res, TAA, clustered lighting, and shared-depth composite are byte-untouched.** One interface, hot-swappable backend.
- **Determinism boundary (C3):** the brick *fetch* (trilinear sample) replaces the analytic *eval* behind `field_distance`. This is a **forked field backend** selected by a residency/scene flag; the analytic `sdf`/`smin`/`combine` path **remains byte-identical and stays the determinism reference + the physics-reuse source of truth** (physics evaluates the analytic field on the CPU, not the brick atlas). Bricks are a render-side acceleration; the physics evaluator is unaffected.
- Brick atlas: a `TextureDimension::D3` STORAGE/SAMPLED image (the enum reserves D3). Brick-map: an SSBO of brick indices. Regen (CPU authoring → upload, or GPU JFA in P12) rebuilds dirty regions; the AABB list of non-empty bricks is the natural BLAS source for P14 (do NOT pick a brick layout that can't emit AABBs — NVIDIA JCGT 2022).
- **Brick format = R16 default** (resolves O2: MEM-D3 commits R16; R8 only on a measured visual bar).

**Expected win.** `O(1)` trilinear fetch replaces `O(edits)` fold; empty-brick skip. Measured on the P0 256–4096-edit scene vs the analytic path (the ≤16-edit fixture shows bricks LOSING — the honest threshold proof).

**Dependency/order.** Needs P4's invariant (so the swap is local). Prereq for P10/P12/P14. Threshold-gated on the P0 edit regime.

**Gate.** Golden-image-equal (within a **documented brick-quantization tolerance** — R16 distance is lossy by design; an owner-approval item per C3) vs the analytic field on a shared scene; **the analytic path stays as the reference oracle AND the byte-identical physics-reuse source**; empty bricks produce no march steps (timestamp); `maxImageDimension3D`-sized atlas (no hardcode); validation clean.

**Effort.** XL. **Conflicts.** Brick quantization leaves bit-exact on the *render* side (documented, golden-tolerance-gated, owner-approved); the analytic determinism reference is preserved. Native/in-house (own brick format, own JFA). 0%-gate: the field backend is forked behind the scene flag; the analytic path remains for low-complexity scenes + physics.

---

## PHASE P10 — Geometry clipmap LOD over bricks — THRESHOLD-GATED

**What.** Nested player-centered brick cascades, each level 2× the extent, so on-screen brick size stays ~constant and far regions evaluate far less often. `field_distance(p)` selects the finest cascade covering `p` (still behind the invariant — marcher untouched).

**Why / threshold.** Needed ONLY for vast worlds where one brick resolution can't cover the draw distance (~2.5 km ≈ 200 trillion dense cells vs ~20 million with clipmaps). Below world-scale, clipmaps are overhead.

**How.** Cascade selection in `field_distance`/`tile_bound`; per-cascade brick-map + a shared atlas pool; far cascades regenerated less frequently.

**Dependency/order.** Needs P9. Threshold-gated on world size.

**Gate.** Golden across a cascade boundary (no seam artifact at the LOD transition); a far-region scene shows the bounded brick-eval count (timestamp); validation clean.

**Effort.** L. **Conflicts.** Cascade-transition seams are the artifact risk (golden across the boundary is the oracle).

---

## PHASE P11 — Hi-Z two-pass mesh occlusion culling — THRESHOLD-GATED, needs P8

**What.** Build a mip-mapped max-depth pyramid (Hi-Z) from the G-buffer depth (P1). Two-pass GPU-driven cull: pass 1 draws last frame's visible set + culls candidates against Hi-Z via indirect dispatch (P8); pass 2 re-tests anything pass-1 may have wrongly culled, so nothing is incorrectly dropped (fixes the single-pass false-cull).

**Why our path needs it.** Removes occluded mesh draws AND (for the hybrid) reduces tiles the SDF must march behind opaque meshes. Production-proven (Frostbite, Killzone 3). **Threshold:** only when the mesh side is non-trivial (many occluded draws).

**How.**
- Hi-Z build: a single-dispatch mip-pyramid (FidelityFX-SPD-style) from the depth image; handle the odd-dimension boundary correctness pitfall (each texel = max of its mip-1 children + the boundary fix).
- Two-pass cull: device-written visible-instance counts → indirect dispatch (P8). The pyramid bounds both the raster candidate list and the P4 tile far-bound.

**Dependency/order.** Needs P8 (indirect, for device-written counts) + P1 (depth). Threshold-gated on mesh complexity.

**Gate.** No false culls (the two-pass property — a known-visible-but-temporarily-occluded object must reappear; a single-pass-only variant fails this, the negative test); golden on an occlusion scene; GPU-timestamp shows draw/tile reduction; validation clean.

**Effort.** L. **Conflicts.** Single-pass false-cull is the trap (the two-pass form + the negative test guard it). Use the two-pass form, not reprojected-previous-depth (staleness).

---

## PHASE P12 — BVH dirty-region regen + JFA — THRESHOLD-GATED

**What.** A per-cascade AABB BVH over non-empty bricks, regenerated only for dirty (edited) regions; brick distances refreshed via jump-flooding (JFA) over dirty bricks. The BVH is both the software-traversal accelerator and the HW-RT BLAS source (P14).

**Why / threshold.** Needed ONLY when the field is GPU-authoritative AND large AND actively mutating (so analytic/CPU regen can't keep up). Incremental regen is "not strictly correct in all cases" (author caveat) — gate on a measured need. **A GPU-mutating field is also where SDF motion vectors need per-hit velocity (Open-Q2) — that work lands here, not in P6.**

**How.** Dirty-region tracking (an edit touches a bounded brick set); JFA over those bricks; BVH refit for modest motion, rebuild for large deformation (the P14 build-flag discipline). The cure for a high regen cost is fewer JFA passes + tighter dirty scoping (ALU/sample-bound), not more bandwidth.

**Dependency/order.** Needs P9 (bricks). Feeds P14 (BVH=BLAS). Threshold-gated on GPU-authoritative mutating large worlds.

**Gate.** Golden after a dirty-region edit matches a full-regen reference (within JFA tolerance, owner-approved); regen timestamp scales with dirty size not world size; validation clean.

**Effort.** XL. **Conflicts.** "Not strictly correct in all cases" — bound by the dirty-scope golden. Highest correctness surface after the solver. **Determinism: a GPU-mutating brick field is render-side; the CPU physics evaluator stays on the analytic field (C3 boundary preserved).**

---

## PHASE P13 — Async compute (multi-queue + cross-queue semaphores) — PROFILE-GATED

**What.** Query queue families at device-create (graphics+compute+dedicated transfer/DMA — today `find_queue_family` picks ONE family, `queue_count:1`). Expose a second compute queue + a transfer queue. **Extend the P6 `submit_windowed` semaphore seam to cross-queue wait/signal semaphores** (P6 built intra-frame/cross-frame present; P13 adds cross-queue). Overlap independent passes: SDF regen / brick JFA on async-compute while raster runs on graphics; staging on the DMA queue.

**Why / gate.** Async helps ONLY when the GPU is not already saturated (NVIDIA: only with unused warp slots; the Vulkan-sample win is ~5%, not order-of-magnitude). The classic anti-pattern (FRAGMENT→COMPUTE forcing COMPUTE→FRAGMENT back) can net-stall. **STRICTLY profile-gated:** do NOT build until a GPU profile shows under-occupancy on a real frame. Same lift unblocks RT-8 (AS-build overlap, P14).

**How.** Multi-queue device create; queue-family-ownership transfers for cross-queue resources; cross-queue semaphores extending `submit_windowed`. Perturbs the P3a/P3b barrier model (which assumes one timeline) — the barrier pass gains a queue-ownership dimension. Purely additive RHI (new queue handles behind the trait).

**Dependency/order.** Needs P3a/P3b + **P6's semaphore-present seam** (which it extends — it does not invent semaphores). Profile-gated; only if measured.

**Gate.** A profile FIRST showing under-occupancy; then golden-equal with async on/off (async must not change the image); GPU-timestamp shows real overlap (not net-stall); sync-validation clean across queues (cross-queue hazards are the hard part — sync-validation is the oracle).

**Effort.** XL. **Conflicts.** Heaviest lift; most in tension with the single-timeline foundation. Cross-queue ownership transfers are a real cost the single-queue path never paid. Never speculative — profile is the gate.

---

## PHASE P14 — AS/RT seam + HW-RT SDF-as-AABB-BLAS backend — CAPABILITY-GATED

**What.** Add the `AccelerationStructure` + `RayTracingPipeline`/`ray_query` RHI seam (absent today, `api.rs:46-67`), raw `VK_KHR_acceleration_structure` + `VK_KHR_ray_tracing_pipeline`/`VK_KHR_ray_query` FFI. Pack each non-empty SDF brick (P9/P12) as a `VK_GEOMETRY_TYPE_AABBS_KHR` BLAS; RT cores traverse the BVH and skip empty space free; the intersection shader runs a trilinear/analytic solve or short march only inside the hit brick. Capability-gate at device-init; the software sphere-tracer (P4/P5) stays the universal fallback (Lumen's HW-or-software split).

**Why / what to decide NOW (do NOT build the AS yet).** RT cores run autonomously parallel to the SMs; for sparse SDF worlds the hardware BVH gives empty-space skipping the software march pays for in ALU (NVIDIA JCGT 2022). **Foundation choices made earlier so RT lands cleanly:** (1) deferred G-buffer as shading substrate (P1) — forward would force reworking shading; (2) motion-vector + history slot (P6) — RT's SVGF denoiser IS the TAA infra; (3) the SDF brick AABB list (P9/P12) as the natural BLAS source; (4) the software sphere-tracer (P4/P5) as the universal fallback. **Defer the AS/RT seam itself** until bricks (P9) exist to feed it (a large raw-`VK_KHR_*` FFI with no consumer before then). The shared-depth composite (P1) already gives correct mesh↔SDF occlusion, so TLAS unification (RT-2) is not urgent.

**How.** New RHI associated types + create/build/destroy verbs; build-flag discipline (`PREFER_FAST_TRACE` static, `PREFER_FAST_BUILD+ALLOW_UPDATE` dynamic, `ALLOW_COMPACTION` static); region routing (HW-RT for static + secondary rays, software trace for the actively-edited near field — AS rebuild is the mutating-SDF worst case); mark geometry `OPAQUE` to avoid expensive any-hit. AS-build overlap wants async (P13).

**Dependency/order.** Needs P9 (bricks=BLAS). Capability-gated (device must advertise the extensions; fallback to software). Feeds P15.

**Gate.** Golden-equal between HW-RT primary visibility and the software tracer (the fallback is the oracle) on a capable device; capability-gate verified (a non-RT device cleanly takes the software path); validation clean; GPU-timestamp vs software march (the honest "order-of-magnitude, not a quoted 5–10×" measure).

**Effort.** XL. **Conflicts.** Must stay opt-in capability-gated with the software tracer as fallback (non-negotiable). AS rebuild is the mutating-SDF worst case (region routing mitigates). In-house: the AS is built via raw `VK_KHR_*` FFI (no ash), consistent with the no-third-party graphics rule.

---

## PHASE P15 — RT lighting (shadows / AO / GI / reflections) + SVGF — CONFIRMED FUTURE

**What.** HW-RT for secondary/incoherent lighting rays against the unified mesh+SDF TLAS: ray-traced shadows (`RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH` for opaque shadow/AO), AO, diffuse GI, reflections. Owner-confirmed. Pair with an SVGF-class spatiotemporal denoiser (= the P6 TAA infra, extended with variance guidance).

**Why.** Secondary/incoherent rays are exactly where HW-RT wins most and software marching hurts most (no screen-space coherence) — the strongest single justification for HW-RT.

**How.** Few rays/pixel → SVGF (temporal accumulation via P6 motion vectors/history + spatial à-trous variance-guided filtering). Cost moves from traversal (cheap on HW) to secondary-ray SHADING divergence — simplify secondary shaders, drop GI/rough-specular to vertex-level shading, bias mips.

**Dependency/order.** Needs P6 (denoiser base) + P14 (AS/RT). Last.

**Gate.** Golden on a known-lighting scene vs a path-traced reference (within a denoised tolerance, owner-approved); temporal stability (P6 gates carried forward); GPU-timestamp; validation clean.

**Effort.** XL. **Conflicts.** Few-sample noise REQUIRES the denoiser (couples to P6); ghosting/temporal-lag failure modes. The honest measure is golden-buffer diffs + timestamps, never a quoted speedup.

---

## 2. Cross-cutting principles applied

- **0%-gate.** Every render pass runs only under a `boyko_render` schedule; a world not using it pays nothing; the named CPU hot loops (`row_ptr`, `for_each_chunk`, query iter, `find_ready`) stay byte-identical (no phase touches them). GPU-side type-keyed routing stays behind const/flag gates per CG-D5.
- **Determinism boundary (the load-bearing constraint).** The deterministic scalar field eval (`sdf`/`smin`/`combine`/normal) + the host mirror `golden_composite_pixel` are **byte-identical across the entire plan** — they are the CPU/GPU golden source of truth a future physics SDF-collision evaluator reuses. No fast math, no reordered FMA, no rsqrt. P5/P6 fork the *consumer* (ray generation, resolution, accumulation), never the field eval; P9 forks the field *backend* (brick fetch) behind `field_distance` while the analytic path remains the reference + physics source. Render-history non-determinism (TAA) is OUTSIDE the physics determinism gate.
- **Measurement oracle.** The GPU half cannot be Miri'd: golden-buffer diffs (bit-exact where possible, **owner-approved documented tolerance where not — P5/P6/P9/P12 each carry a per-phase tolerance gate, NOT critic-discretion**) + Vulkan validation + sync-validation wired to FAIL tests, on a real RTX 3060. CPU-side lowering/build code stays criterion + Miri where unsafe.
- **In-house / native / raw-Vulkan.** No ash/wgpu; all new verbs are raw-FFI behind the RHI trait (static dispatch, monomorphized, no `dyn` in the hot record path). The seam-with-default-`#[cold]`-body pattern (already used for `copy_buffer`/`image_barrier`/`dispatch_indirect`) is the template for every new encoder verb — Mock + ABI stay stable.
- **Differential SIMD/subgroup discipline (P7).** Subgroup paths ship with a scalar reference + a differential golden (the CPU-D3 house template applied to GPU), capability-gated, never hard-coding wave width.
- **Two barrier codepaths are distinct (C1).** The on-screen frame uses hand-FFI `swapchain.rs` barriers (P3a batches them); the ECS conflict-graph uses `boyko_render::barrier.rs` `lower_barriers` (P3b narrows + adds constants for P8/P11). They are never conflated.

## 3. "Production-ready when" — spine exit criteria

The renderer is production-scale (the spine complete) when ALL hold:
1. **P0**: the marcher renders at 1080p with a perspective camera, fed by an ECS `SdfEdit` query; the rung-8..11 ortho goldens stay bit-exact; sparse/dense GPU-timestamp baselines recorded.
2. **P1**: the on-screen frame is a real MRT-image deferred renderer (depth/normal/albedo/material images, multi-resource descriptors); the per-frame depth→buffer copy is gone; golden-equal to the packed-buffer reference.
3. **P3a**: per-frame `cmd_pipeline_barrier` call count measurably reduced (command-stream inspection); sync-validation clean.
4. **P4**: tile-cull shows a measured fine-march step reduction on the P0 sparse scene; conservative (golden-equal, no holes).
5. **P5**: half-res + depth-aware upscale at an owner-approved relaxed tolerance; the field eval + `golden_composite_pixel` byte-identical (physics contract intact); ~4× ray reduction on SDF-surface pixels measured.
6. **P6**: TAA-stable on static + camera-pan + disocclusion goldens; the semaphore-present seam (`submit_windowed`) is built and sync-validation clean; the field eval byte-identical.
7. **Throughout**: validation + sync-validation clean on RTX 3060; 0%-gate honored (CPU hot loops byte-identical); every `unsafe` carries `// SAFETY:`.

P7/P8/P3b may land in parallel after P1 (not spine-blocking). P9–P15 are gated and NOT part of the production-ready spine bar — they activate only when their threshold/capability/profile gate fires.

## 4. Open questions for the owner (decisions, not critic-discretion)

1. **Per-phase relaxed-tolerance / baseline-reset approval (C3).** P5 (half-res+upscale), P6 (TAA), P9 (brick R16 quantization), P12 (JFA) each leave bit-exact on the *render* side. RESEARCH-FAST-MATH lists "accept a one-time determinism-baseline reset" as an explicit architect→owner question. **Requesting owner sign-off on each per-phase tolerance bar** (the field eval + physics-reuse source stay bit-exact regardless — only the render consumer relaxes). Default proposal: ±4/255 SSIM/PSNR bar for P5/P6, documented per-format quantization bar for P9/P12.
2. **SDF motion vectors for a GPU-mutating field (now scoped to P12, not P6).** For the static analytic field + jittered camera (P6), the SDF hit's motion vector is the deterministic camera reprojection of the hit world position — sufficient. A GPU-mutating field (P12) needs per-hit velocity; that work is folded into P12. **Confirm this split is acceptable** (P6 ships static-field motion vectors; per-hit velocity waits for the mutating-field threshold).
3. **G-buffer normal format (O3): R8G8B8A8 now, R16G16 octahedral later.** Octahedral is better precision/bandwidth but is itself a **golden-baseline reset** (changes the deferred-light golden's bit pattern — same class as Q1, not a free swap). **Proposal:** start R8 (matches the existing prototype golden), switch to octahedral once P6's TAA exposes normal-precision banding, under a one-time owner-approved baseline reset.
4. **Tile/half-res buffer placement (P4/P5).** Standalone SSBOs (clean, uses P1's vocabulary) vs new regions in the composite buffer (matches the rung-10 const-assert pattern). **Proposal:** standalone SSBOs post-P1 — the packed-buffer pattern was the single-binding workaround P1 retires.

---

## Files this plan is grounded in (absolute paths)

- `D:\claude\BoykoEngine\docs\RESEARCH-GRAPHICS-OPT.md` — priority/sequence source
- `D:\claude\BoykoEngine\docs\PERF-DIRECTIONS.md` — RT-*/GPU-D*/MEM-D* catalog + 0%-gate + honesty discipline
- `D:\claude\BoykoEngine\docs\RESEARCH-FAST-MATH.md` — the SDF-golden determinism boundary (the physics-reuse contract)
- `D:\claude\BoykoEngine\crates\boyko_rhi\src\{api,encoder,enums,queue}.rs` — RHI seam: AS absent (`api.rs:46-67`), `dispatch_indirect` stub (`encoder.rs:269`), `bind_storage_buffer` (`encoder.rs:38`), no-semaphore `submit` (`queue.rs:22,31`), `BufferUsage::INDIRECT` (`enums.rs:37`)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\{swapchain,device,rhi_impl}.rs` — on-screen hand-FFI barriers (`swapchain.rs:844,914,1169,1201,1337,1370,1432,1708,1831,1864,1926`), `record_scene:1123`/`sync_depth:2071`/`record_present_sampled:1661`, single queue (`device.rs:1634,1750`), COMBINED_IMAGE_SAMPLER-only `create_bind_group` (`rhi_impl.rs:717`)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\{sdf_depth_composite,gbuffer.fs,deferred_light.fs}.hlsl` — the 64×64/16-edit/`MAX_IT=128` fixture marcher (`sdf_depth_composite.hlsl:86-87,104,117`) + the determinism-frozen field math (`:8-12,47-51`) + the MRT/deferred prototypes
- `D:\claude\BoykoEngine\crates\boyko_render\src\barrier.rs` — the **distinct** ECS-edge `lower_barriers` pass (`:1-15,58-76,196-233`); P3b adds constants + narrows here (NOT the on-screen path)

---

All four critic blockers (C1, C2, C3 + the W-series and O-series) are resolved with code-verified grounding; the determinism boundary and the two-codepath barrier distinction are now load-bearing in the plan structure. Ready to save as `docs/OPTIMIZATION-PLAN-RENDER.md` and execute.