# VB-P2 — Material classification / material-grouped shading (implementation plan)

**Status:** DESIGN (architect ↔ critic hardened — 4 P1 resolved). Sub-plan of
[VB-PERFORMANCE-TRACK.md](VB-PERFORMANCE-TRACK.md) §4 (VB-P2). The clean pre-textures foundation the
owner chose to land before textured materials ([RENDER-PARITY-PLAN.md](RENDER-PARITY-PLAN.md) TV0).

## Goal
Restructure the VisibilityBuffer shade stage from one fused über-dispatch (`vb_resolve.comp.hlsl`)
into a **material-classified pipeline**: classify mesh pixels by material id, then shade
material-coherent groups so each shading workgroup runs against a **wave-uniform `mat_id`**. Payoff
is deferred to textures (each material samples only its bindless maps via a UNIFORM index — no
`NonUniformResourceIndex`, no intra-wave sampler divergence). For today's flat materials it is a
**byte-identical refactor** (per-pixel shading is independent → regrouping cannot change any pixel's
bytes → `vb_mesh f4719cbf` / `vb_both f4719cbf` / `vb_sdf_only a1256bde` hold exactly, no re-pin).

## The pipeline (replaces the single `vb_resolve` dispatch when selected)
1. **fill** — `vkCmdFillBuffer` zeros `counts[M]` **AND fills `group_to_mat` with SENTINEL
   (`0xFFFFFFFF`)** (critic P1-1 — the over-dispatch tail must read SENTINEL from a known-init state,
   not stale last-frame data).
2. **count** (grid `w*h`) — each pixel loads `vb_id`, skips `VB_ID_SENTINEL`,
   `mat = instance_materials[iid].id`, `InterlockedAdd(counts[mat], 1)`.
3. **scan** (1 workgroup) — exclusive prefix sum `counts→offsets`; `cursors=offsets`;
   `gc=ceil(counts/64)`; prefix→`gbase`; `total_groups=Σgc`; write `group_to_mat[0..total_groups]=mat`
   (only OVERWRITES the range; the SENTINEL tail from `fill` stands).
4. **scatter** (grid `w*h`) — each non-sentinel pixel: `slot=InterlockedAdd(cursors[mat],1)`;
   `pixel_list[slot]=py*w+px`.
5. **vb_shade** — a REGULAR `vkCmdDispatch(G + present_material_count, 1, 1)` (NOT indirect —
   `vkCmdDispatchIndirect` is not in the FFI). Each group `g`: `mat=group_to_mat[g]`; SENTINEL→return;
   `slot=(g-gbase[mat])*64 + (tid&63)`; `slot>=counts[mat]`→return; `idx=pixel_list[offsets[mat]+slot]`;
   `px=idx%w; py=idx/w`; then **token-for-token** the `vb_resolve.comp.hlsl:220-356` shading tail
   (`vb_geom_fetch` + PBR + `shadow_apply`), `gLit[px,py]=…`.

## Key decisions (architect, critic-verified)
- **D1 — full-screen per-material bins (count→scan→scatter), NOT tiled.** Full-screen bins give
  wave-uniform `mat_id` with zero in-kernel discard (the coherence textures need); tiled needs a
  per-tile `if(mat!=mine) return` discard. 1D dispatch matches the codebase idiom.
- **D2 — regular over-dispatch, NOT `vkCmdDispatchIndirect`.** The FFI lacks indirect dispatch;
  `vkCmdDispatch(G + present_material_count)` provably covers `total_groups ≤ ceil(w*h/64) +
  present_material_count` (each present material adds ≤1 partial group); surplus groups early-out on
  the SENTINEL read. **`present_material_count` = the frame's distinct material ids from
  `scene.mesh_draw`, NOT `MaterialTable::capacity_rows()`** (critic P2-2 — avoids launching
  `capacity_rows` wasted groups).
- **D3 — byte-identical by construction.** `vb_shade` = the `vb_resolve` body + an ~8-line prologue;
  `idx=py*w+px` recovers `px=idx%w; py=idx/w` (same instructions as `vb_resolve.comp.hlsl:214-215`);
  the shading tail is character-identical and still reads `Materials[instance_materials[iid].id]`
  (unchanged). Image goldens authoritative (Decision 3); **no re-pin** — a moved hash is a bug.
- **D4 — one packed `gClassify` buffer at `vb_layout0 b7`** (`[counts|offsets|cursors|gbase|
  group_to_mat|pixel_list]`) keeps VB-P2 shade at **3 sets** (Set0 core+classify / Set1 shadow / Set2
  geometry). TV0 later adds Set3 bindless-tex → 4 sets (Vulkan floor). SV0's mesh-shadow binding then
  becomes `b8` on Set 0 (no conflict). Anchors: TV0 Set-3 = RENDER-PARITY-PLAN.md:350; 4-set floor =
  :115/:147.

## Critic P1 resolutions (baked in)
- **P1-1 (SENTINEL tail):** `fill` also `vkCmdFillBuffer(group_to_mat_subregion, 0xFFFFFFFF)`; scan only
  overwrites `[0, total_groups)`. Decouples correctness from the scan loop bound.
- **P1-2 (runtime growth of `material_count`):** **pre-size the M-arrays
  (`counts/offsets/cursors/gbase`) to `MAX_MATERIAL_ROWS`** (~1 MB total, negligible vs the 8 MB
  `pixel_list`) so the `gClassify` sub-region layout is FIXED and never invalidated by
  `MaterialTable` growth (the F7 rebind class). The scan/dispatch iterate over the frame's
  `present_material_count`, not `MAX` (so the pre-size costs no runtime iteration). `pixel_list` is
  `w*h` (material-independent). `group_to_mat` cap = `G + present_material_count`.
- **P1-3 (RW→RW barrier):** VERIFY during P2b that the framegraph emits a `SHADER_WRITE→SHADER_READ`
  barrier between consecutive RW-compute passes on the single `gClassify` ResId (count→scan→scatter).
  If the compiler coalesces same-access-type accesses without a barrier, split into distinct ResIds
  or record manual buffer barriers (the `cmd_fill_buffer` cull path is the manual-barrier precedent).
  A single ResId barriers the WHOLE buffer between passes (conservative-correct; drop the
  "sub-region barrier" wording).
- **P1-4 (perf policy — OWNER DECIDED):** classification is a ~0.3 ms LOSS for flat materials
  (unmeasured estimate) with the win only from textures. **Owner chose: build classified + KEEP the
  fused `vb_resolve` + measure on RTX + a host SELECTOR** — flat/non-textured frames use the fast
  fused `vb_resolve`, textured frames use `vb_shade`. Honors HYBRID-perf (fastest per decision) AND
  clean-architecture (textures land on the classified pipeline). Cost: two VB shade tails maintained
  (same duplication the 4 existing shading tails already pay). Selector predicate: reuse the
  `mesh_tex_active()` "any non-zero material texture slot this frame" gate (VB-P2 classified is
  selected exactly when TV0 textures are active).
- **P2-1:** no `create_compute_pipeline_vb1` — the generic `create_compute_pipeline(desc{layout:
  Some(vb_layout0)})` builds the 1-set count/scan/scatter pipelines (Set-0 layout object shared).
- **P2-4:** anchor fix — TV0 Set-3 = RENDER-PARITY-PLAN.md:350; 4-set floor = :147/:115.

## P2a byte-identity requirements (adding `b7` to the shared `vb_layout0`)
Byte-safe ONLY if: (1) `vb_layout0` is built with `b7` BEFORE all pipelines (already the order at
`gpu_scene/mod.rs:2949→3013`), so `vb_sky`/`vb_raster`/`vb_resolve` rebuild against the ONE new
layout object (R5 — a set on the new layout bound to a pipeline on the old layout = silent black,
validation is OFF on this box); (2) `vb_set0` grows 7→8 entries with `b7 = gclassify[fi]` WRITTEN
even though unread (an in-layout-but-unwritten binding a robustness path touches = silent black);
(3) the descriptor pool gains +1 STORAGE_BUFFER/set. Frozen `.spv` unchanged → bound-but-unread `b7`
cannot alter output.

## Data
`VbClassifyTargets` (targets.rs, sibling of `VbTargets`): per-FIF `gClassify: [BoundBuffer;
FRAMES_IN_FLIGHT]` (STORAGE | TRANSFER_DST, per-FIF → no cross-frame WAR, frame fence drains slot
reuse). Layout `[counts(MAX) | offsets(MAX) | cursors(MAX) | gbase(MAX) | group_to_mat(G+M cap) |
pixel_list(w*h)]`, host-computed byte offsets (a host↔shader sync-pin). ~8.4 MB/FIF @1080p, ~17 MB
total — RHI device memory, allocated once per extent, zero frame-loop allocation (Principle 0:
engine-owned, no std::Vec side store). `present_material_count` from `scene.mesh_draw` distinct ids.

## Rungs (each golden-gated; byte-identity is the primary oracle)
| Rung | Lands | Byte-identical | Size |
|---|---|---|---|
| **P2a** dark infra | `VbClassifyTargets`, `vb_layout0 +b7`, `vb_set0` writes `b7`, 4 shaders authored+compiled, `build_vb_classify_pipelines`. Nothing wired into record/declare — fused `vb_resolve` still shades. | `vb_mesh`/`vb_both`/`vb_sdf_only` + Deferred/Forward (b7 bound-but-unread) | **M** |
| **P2b** classify live | `fill`(+SENTINEL tail)+`count`+`scan`+`scatter` declared+recorded (populate `gClassify`); fused `vb_resolve` still writes `lit`. Validates on-device (no hang/corruption), output unchanged. **Verify P1-3 barrier chain here.** Add debug-assert `group_to_mat[g] == instance_materials[iid].id` (validates the uniformity invariant TV0 relies on, masked by byte-identity). | same | **M** |
| **P2c** `vb_shade` selectable | `vb_shade` becomes a selectable `lit` producer; host selector picks `vb_shade` vs fused `vb_resolve` (P1-4; for P2c verification, FORCE `vb_shade` on the flat goldens → must reproduce `f4719cbf`/`a1256bde` exactly). **Measure the classify tax on RTX** (criterion/GPU-capture, fused vs classified @ M∈{1,5}). | `vb_mesh`/`vb_both`/`vb_sdf_only` (forced-classified path) | **L** |

TV0 (textures) then lands on `vb_shade` with the uniform `mat=group_to_mat[g]` texture index (the
selector activates classified for textured frames). SV0 (SDF-shadow) adds `b8` on Set 0.

## Open items
- P2b: measured on-RTX classify tax feeds the P1-4 selector threshold (owner-informed).
- P2c open Q (architect): `vb_shade`'s TEXTURE index at TV0 must be the uniform `group_to_mat[g]`
  (the whole point), while the byte-identity shading tail keeps `instance_materials[iid].id` — the
  P2b debug-assert proves they're equal per group.
- `present_material_count == 0` (all-sentinel / no-mesh frame): `vb_shade` dispatches `G+0` groups,
  all early-out on `slot>=counts` — safe; confirm `mesh_leg` gates the whole classify chain like
  `vb_raster`/`vb_resolve` (graph_bridge.rs:2957).
