# VG R3 — two-phase occlusion culling: how the field actually does it

Research gathered for piece 2's design round ([VG-R3-P2-CAPABILITY-SPLIT-PLAN.md](VG-R3-P2-CAPABILITY-SPLIT-PLAN.md)).
Primary sources only; claims that could not be verified from a fetched source are marked in place.

## ⚠️ Two hard LOCAL constraints, neither of which the piece-2 design currently names

1. **`vkCmdDrawIndexedIndirectCount` is NOT in the device fn table.** `boyko_rhi_vulkan/src/device.rs`
   declares and loads only `cmd_draw_indexed_indirect`. Every niagara-shaped "reuse one buffer, reset
   the count" design below assumes the `…Count` entry point (Vulkan 1.2 core /
   `VK_KHR_draw_indirect_count`). Adopting that pattern means adding it to the table first.
2. **The engine already sits in the configuration vkguide and pcwalton warn about** — a fixed-length
   `VkDrawIndexedIndirectCommand` array where a culled entry gets `instanceCount = 0`
   (`present/scene_types.rs`, "A culled batch gets `0` written over it"), drawn with
   `vkCmdDrawIndexedIndirect`. That is per-entry empty draws. It is also a PREFIX, not a mask.

## The headline

**The five reference implementations do not implement the same algorithm.** They differ on the
*early-pass predicate*, the *late-pass input*, the *late dispatch*, and *buffer sizing*. Picking one
is picking four coupled decisions, not one.

| aspect | niagara | Bevy (std mesh) | Bevy (meshlet, current) | Nanite | Ubisoft 2015 |
|---|---|---|---|---|---|
| early predicate | frustum + **stored visibility bit**; **no HZB test at all** | HZB vs **previous** pyramid with **previous** transforms | HZB vs previous pyramid | HZB vs previous (reprojected) HZB | HZB vs last frame's pyramid |
| cross-frame state | `drawVisibility[]` u32/draw + `meshletVisibility` 1 bit/meshlet (atomic) | **pyramid texture only** + `previous_input` | pyramid + raster counts | **pyramid only** — explicitly no inter-frame list | pyramid (HTILE min/max) |
| late input | **re-scan all N**, different predicate | compacted `late_preprocess_work_items` | queue consumed from the far end | compacted `OccludedInstances` | phase-1 losers |
| late dispatch | fixed, over all draws | `dispatch_workgroups_indirect`, `dispatch_x` grown per 64 survivors | indirect, atomically grown | indirect via `OccludedInstancesArgs` | not stated |
| draw indirect buffer | **same buffer + same count buffer, count refilled to 0** | shared params, `reset_indirect_batch_sets` per phase | shared raster args, atomically grown | **separate per pass** | not stated |
| survivor sizing | **N, reused; overflow DROPS draws** | **two buffers** (early + late) | **one buffer, both ends** (`rightmost_slot`) | per-pass caps, 12 B / 16 B | not stated |
| off case | **not skipped** (author TODO) | **ECS/graph-level skip** (`With<OcclusionCulling>`) | n/a | `CULLING_PASS` permutation | not stated |
| HZB tap | 1× `textureLod` through `REDUCTION_MODE_MIN` + `FILTER_LINEAR` | **4× `textureLoad` + shader-side `min`, no sampler** | SPD pyramid, 4 px per AABB | min/max pyramid | `Gather4` |

## The disarmed case — the question piece 2 actually has to answer

**niagara does NOT skip its late pass, and its author knows it.** `drawcull.comp.glsl` carries the
verbatim comment `// TODO: when occlusion culling is off, can we make sure everything is processed
with LATE=false?`. So the shipped state of a performance-obsessed codebase is *a fully recorded late
pass whose count converges toward zero* — while simultaneously flagged as unfinished.

**Bevy skips at the ECS/graph level**: `late_gpu_preprocess` carries
`Without<NoIndirectDrawing>, With<OcclusionCulling>, With<DepthPrepass>`, and the late-prepass and
downsample nodes exist only under the same components.

⚠️ **No source quantifies the trade.** The closest published claims are about a DIFFERENT cost —
per-*entry* empty draws inside an MDI array (vkguide: "empty draw commands still have overhead";
pcwalton, Bevy #17211: culled entries "remain present in the list … causing overhead"). Nobody
measured recording cost versus an indirect read of zero. If it matters here it must be measured
against this campaign's own zero-control discipline.

## Vulkan: an indirect draw with count 0

From `Vulkan-Docs/chapters/drawing.adoc`, fetched verbatim:

- `vkCmdDrawIndirect`: "`drawCount` is the number of draws to execute, and **can: be zero**."
- `vkCmdDraw*IndirectCount`: "The actual number of executed draw calls is the **minimum** of the
  count specified in `countBuffer` and `maxDrawCount`."

So a stored 0 gives `min(0, maxDrawCount) = 0`. No valid-usage statement forbids zero in either
family. Note the asymmetry: the explicit "can: be zero" attaches to the non-`Count` parameter; for
the `Count` variants the zero case is *implied by the min rule* rather than stated.

**Measured IHV numbers: none found.** A WebGPU/Dawn figure exists (~3 ms of a ~6 ms pass consumed by
indirect-draw *validation*, dropping to ~10 µs after combining buffers) but that is Dawn's own
validation shim, an artifact of the WebGPU security model, and does not transfer to native Vulkan.

One lead worth verifying: `VK_EXT_conditional_rendering` is specified as discarding rendering
commands when a 32-bit value is zero, and implementations are said to use the same mechanism for
indirect-count — which would put count == 0 and a false predicate on one hardware path. **Not read
verbatim in a primary doc; treat as a lead.**

## ⚠️ A premise I had carried, corrected

The standard HZB tap is **not** "`textureLod` with a manually computed mip and point sampling".
There are two standard practices and neither is that:

- **niagara**: ONE `textureLod` through a sampler created with `VK_SAMPLER_REDUCTION_MODE_MIN`,
  `VK_FILTER_LINEAR`, `VK_SAMPLER_MIPMAP_MODE_NEAREST`. The filter *is* LINEAR; the reduction mode
  replaces the weighted average with a **min**, so one tap returns min-of-2×2.
- **Bevy** (`occlusion_culling.wgsl`): four explicit `textureLoad`s and an explicit `min` in the
  shader, **no sampler at all**, with a comment that it deliberately does not use
  `textureSampleLevel`.

**Why linear filtering of a reduced pyramid is wrong**, stated once: a reduced pyramid stores a
*bound over a footprint*, not a band-limited signal. A bilinear blend of four reduced texels is a
convex combination, so it lies strictly between their min and max — neither an upper nor a lower
bound of the underlying footprint. Under reverse-Z with a `min` reduce the stored value must be ≤
every depth in the footprint; a blend can be *greater*, and therefore reject something visible.
False negatives are missing geometry, the one failure mode that is not recoverable. A MIN-reduction
sampler with FILTER_LINEAR is not a counterexample — the reduction mode replaces the average
entirely.

**Bevy's practice B matches this engine's existing `.Load`-only discipline** in
`hzb_build.comp.hlsl` and avoids a `VK_EXT_sampler_filter_minmax`-class dependency.

## Pitfalls the field has actually hit

- **Non-conservative mip selection.** zeux, Bevy #14042: the cluster-occlusion mip computation
  scaled the screen-space bbox before `log2`, when it was already in pyramid space. Concrete case: a
  ~29.36 × 30.06 px bbox, the erroneous scaling reduced the max to 15 and selected mip 4 instead of a
  coarser level, so the 2×2 footprint no longer covered the sphere → visible clusters wrongly
  rejected. **Symptom: geometry disappears at certain distances only.**
- **Non-power-of-two framebuffers.** Bevy ships two-phase OC as *experimental* explicitly because of
  "precision issues with non-power-of-two framebuffer sizes, occasionally misclassifying small meshes
  as occluded" (#17413, #14062). Any pyramid whose level 0 is `prev_pow2` of the source makes
  screen-UV → pyramid-texel non-identity, and **that mapping is where the bug lives** — which is
  exactly the map piece 1's G3 gate pins at 7×3, 511×1023 and 1920×1080.
- **Forgetting to re-init the late indirect parameters every frame.** Bevy #17736: #17684 broke OC by
  neglecting to set the late offsets *if the work-item buffers were already set*. The fix was an
  unconditional per-phase, per-frame init. **It only manifests on the second and later frames of a
  stable scene — exactly the shape a golden pin can miss.**
- **Reusing the draw buffer without the right barrier.** niagara's reuse is legal only because of
  `stageBarrier(DRAW_INDIRECT → TRANSFER)` before `vkCmdFillBuffer`. Drop it and you overwrite
  records the previous draw is still fetching.
- **Overflow policy: everyone drops, nobody grows.** niagara drops past `TASK_WGLIMIT` ("this limits
  us to ~4M visible draws"); Nanite drops past `MaxCandidateClusters` and surfaces it as blinking
  geometry.
- **`min()` and NaN** — already recorded in `hzb_build.comp.hlsl` and in piece 1 §10.

## The only concrete numbers in print

**Ubisoft, Xbox One, 1080p** (SIGGRAPH 2015 timing table): object cull + LOD 0.28 / 0.26 ms
(phase 1 / phase 2); cluster cull 0.09 / 0.04 ms; **draw 1.60 ms / < 0.01 ms**; pyramid 0.06 ms;
total 2.3 ms.

⚠️ **The phase-2 draw at < 0.01 ms is the single most decision-relevant number here.** On a scene
where two-phase works, the second draw costs essentially nothing because almost nothing survives it
— which is precisely the regime where "recorded with a near-zero count" and "not recorded" are hard
to tell apart. Piece 2's inert late scope sits in exactly that regime *by construction*.

**Bevy meshlet**, 3092 bunnies, 2240×1260, RTX 3080: first cull 0.49 ms, downsample 0.03 ms, second
cull 0.11 ms, pipeline ~2.78 ± 0.33 ms.

**Bevy std mesh, Bistro** (#17413): rendered meshes 1591 → 585, but pcwalton records frame-time gains
"remain limited" because of per-mesh **CPU** overhead for occluded objects. **Culling wins can be
eaten entirely upstream** — a caution this campaign has already met from the other direction.

## Applicability here

**Directly transferable.** niagara's single-`render`-lambda structure (identical draw in both phases,
differing only by `loadOp` and pipeline) maps onto a pass recorder, and its
`DRAW_INDIRECT → TRANSFER → COMPUTE` barrier triple is the reusable safety argument. Bevy's
`atomicAdd(work_item_count); if (i % 64 == 0) atomicAdd(dispatch_x)` is the cheapest known way to
size an indirect dispatch from an append list without a second reduce.

**Needs adaptation.** niagara's per-draw `drawVisibility[]` is a per-object side SSBO — under
Principle 0 that is a component column or a dense component, not a raw buffer conjured beside the
ECS.

**Does not fit.** Nanite's "no inter-frame visibility list" is justified by *continuous LOD +
streaming* making cluster IDs unstable across frames. If cluster IDs here are stable, that argument
does not apply and niagara's cheaper stored-bit early pass becomes available.

⚠️ **The granularity caveat that outranks all of the above.** Every reference implementation culls at
*object or cluster* granularity. This campaign's recorded finding — the AABB of a batch is the union
over all its instances, so a per-BATCH cull rejects zero on the corpus — means a late pass bolted
onto per-batch granularity inherits the same nullity. **None of these sources solve a granularity
problem; they all assume the cull unit is already the right size.** Rung R2d moved this engine to
per-INSTANCE granularity, so the caveat is discharged — but it is the first thing to re-check if a
later measurement shows the late pass rejecting nothing.

## Open questions this leaves for the architect

1. Early predicate: stored visibility bit (one buffer, no reprojection, but cannot reject in the
   early pass something visible last frame and occluded now) or HZB re-test with previous-frame
   transforms (no persistent bit, needs `previous_world_from_local` per object, inherits reprojection
   error)?
2. Given only `vkCmdDrawIndexedIndirect`, niagara's "reset the count" trick is unavailable. Is adding
   the `…Count` entry point in scope, or does the late pass get its own record array?
3. Recorded-but-zero (niagara's shipped state, plus its author's TODO) or unrecorded (Bevy's
   component filter)? **No source quantifies it.**
4. Survivor storage: N-reused, two buffers, or one buffer from both ends? Bevy's meshlet
   `rightmost_slot` scheme is undocumented as to *why*; adopting it means adopting an uncommented
   invariant.

## Sources

zeux/niagara (`drawcull.comp.glsl`, `niagara.cpp`, `math.h`, `meshlet.task.glsl`, `resources.cpp`) ·
Bevy PRs #17413, #17211, #17736, #14042 and `mesh_preprocess.wgsl`, `occlusion_culling.wgsl`,
`gpu_preprocess.rs`, `cull_clusters.wgsl`, `meshlet_bindings.wgsl` · jms55, "Virtual Geometry in Bevy
0.14" · Haar & Aaltonen, "GPU-Driven Rendering Pipelines", SIGGRAPH 2015 (+ slide transcription for
the timing table) · Aaltonen's later restatement thread · Karis/Stubbe/Wihlidal, "A Deep Dive into
Nanite Virtualized Geometry", SIGGRAPH 2021 · UIUC CS418 Nanite notes · Tricky Bits, "Nanite Deep
Dive Part 1" · The Code Corsair, "A Macro View of Nanite" · UWA, "Analysis of UE5 Rendering
Technology: Nanite" · Epic, "Nanite Technical Details" · Khronos `Vulkan-Docs/chapters/drawing.adoc`
· vkguide, "Draw Indirect" and "Compute based Culling" · gpuweb #5175 · devsh, "Don't even dream of
reprojecting last frame depth" (note: the same author endorses the two-pass scheme; it is an
argument against reprojecting the depth *buffer*) · zeux.io, "Approximate projected bounds".
