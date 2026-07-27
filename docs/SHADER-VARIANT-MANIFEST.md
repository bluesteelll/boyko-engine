# Shader `-D` Variant Manifest

**Single source of truth for the monolithic `-D`-preprocessor shader variants** that *cannot*
collapse to one `.spv` (they change the descriptor set / capability / emitted code), so they stay
**N `.spv` compiled from ONE `.hlsl`**. This is the A-3 half of the shader-growth remediation (see
[REFACTORING-PLAN.md](REFACTORING-PLAN.md) §A3): the registry de-dup (A-2 `embed_spirv!` macro) removed
the hand-counted embed sizes, and this table removes the *scavenger hunt* — a new variant is a row here,
not archaeology through `compute.rs` doc-paragraphs.

Contrast with the **spec-constant-collapsible** families (`GI_MAX_IT` — SHIPPED @3c10826) and the
**perf-justified** ones (SSAO quality — `[unroll]`, kept as 3 files; see the plan). A `-D` variant belongs
HERE only if it changes the *interface* (adds/removes a binding, a capability, or half the shader) — a
spec-constant cannot do that.

## The three axes

| Axis (`-D`) | Values | What it changes |
|---|---|---|
| `SHADOW_STAGE` | `RESOLVE_INLINE` (unset/0), `VIS` (1), `DENOISED` (2) | How the mesh shadow term is produced/consumed. INLINE combines the trace into lighting directly; VIS **writes** `gShadowVis` (the à-trous/temporal pre-pass) and strips lighting; DENOISED **reads** the final denoised `gShadowVis` and combines it. |
| `HWRT` | unset (0), `1` | 0 = software SDF-marched mesh shadows (no ray HW). 1 = hardware inline `rayQuery` against a TLAS — adds `OpCapability RayQueryKHR`, the acceleration-structure descriptor, and **requires `-T cs_6_5`** (vs `cs_6_0`). Gated by `feature = "hwrt"` + a runtime `ctx.ray_query_enabled()`. |
| `MOTION_VECTORS` | unset (0), `1` | 0 = no motion output. 1 = also emit per-pixel motion (Δuv) for the temporal shadow denoiser — a new storage binding (deferred) or a 4th MRT (gbuffer). Static camera ⇒ (0,0). |

## `deferred_pbr.hlsl` — the fullscreen deferred resolve (compute)

One source `shaders/deferred_pbr.hlsl`; the host selects a variant by **binding a different pipeline**,
never a dynamic branch. All share the base 0..11 binding block (G-buffer STORAGE images, material SSBO,
camera UBO, light table, cluster grid, SDF edit-list, SSAO). The deltas:

| Variant | `SHADOW_STAGE` | `HWRT` | `MV` | `TW` | `.spv` | dxc `-T` | Interface delta vs base (0..11) |
|---|---|---|---|---|---|---|---|
| RESOLVE_INLINE (software) | — | — | — | — | `deferred_pbr.comp.spv` | `cs_6_0` | none — software SDF `sdf_soft_shadow_ranged`; +CSM `gCsm`/`gCsmCmp` @12/13/14 + punctual atlas + (bound-unread) DDGI @16/17/18. |
| TERMINATOR_WRAP (software) | — | — | — | `1` | `deferred_pbr_wrap.comp.spv` | `cs_6_0` | **none — the interface is identical to the base row**, which is why it reuses the same `resolve_layout` (`gpu_scene/mod.rs:1654-1656`). The delta is arithmetic: `nol_wrapped` (`deferred_pbr.hlsl:587`) replaces the raw `NoL` at BOTH direct-diffuse accumulation sites — the directional one at `:1140` and the punctual one at `:1353` — specular keeping the physical clamp at each. Frozen-base discipline — with the flag undefined the source preprocesses **character-identical** to the pre-feature file, so the base row's `.spv` is untouched by construction. |
| RESOLVE_INLINE (hardware) | — | `1` | — | — | `deferred_pbr_hwrt.comp.spv` | `cs_6_5` | `+RaytracingAccelerationStructure gTlas` @19 + `OpCapability RayQueryKHR`; the `#if HWRT` Vogel-disk cone trace (`SHADOW_RAY_COUNT` spec-const) replaces the software march. |
| VIS | `1` | `1` | — | — | `deferred_pbr_hwrt_vis.comp.spv` | `cs_6_5` | hwrt layout **+ `RWTexture2D<float2> gShadowVis`** @21 (**write** `RG(mesh_vis, validity)`); lighting stripped (writes vis, not lit). The à-trous/temporal pre-pass. |
| DENOISED | `2` | `1` | — | — | `deferred_pbr_hwrt_denoised.comp.spv` | `cs_6_5` | same 22-binding VIS/DENOISED layout, but `gShadowVis` @21 is **read** (`mesh_vis = gShadowVis.Load().r`, the final denoised output) and combined `vis = min(vis, mesh_vis)`. Declares NO acceleration structure. |
| VIS + motion | `1` | `1` | `1` | — | `deferred_pbr_hwrt_vis_mv.comp.spv` | `cs_6_5` | VIS layout **+ `MotionCam` UBO** @22 **+ `RWTexture2D<float2> gMotionVec`** (`rg16f`, SIGNED) @23 — writes clip-space Δuv for the temporal reproject. |

`TW` = `TERMINATOR_WRAP`. Reachability note: `SHADOW_STAGE ∈ {VIS, DENOISED}` and `MOTION_VECTORS=1`
are only reachable **with** `HWRT=1` (the spatial/temporal shadow-vis denoise pipeline is built on the
hardware mesh-shadow trace); there is no software VIS/DENOISED/MV `.spv`. `TERMINATOR_WRAP` is
**software-resolve-only** and mutually exclusive with `HWRT` by scope decision — the combination is
never compiled and never selected (`deferred_pbr.hlsl:76-79`). The wrap variant is built
**unconditionally** (its accessor `compute.rs:1470` carries no `#[cfg(feature = "hwrt")]`, unlike the
four HWRT ones) and the host binds it only when `LightingConfig::terminator_softening > 0`; the
variant itself is the opt-in, so the wrap arm carries no runtime `if`.

*Provenance of this row: it was **missing** until 2026-07-26 — the variant has shipped since its own
introduction, so this table has been incomplete for that whole time. Found while enumerating the
family for the VB-SV0 gate that must prove all six unperturbed
(`docs/VB-SV0-SDF-SHADOW-PLAN.md` §5.4); all six were re-DXC'd byte-identical against the committed
artifacts at that time.*

*Byte gate (added at rung VB-P1k): all six rows — and both `forward_opaque.fs.hlsl` rows below —
are now re-DXC'd and byte-compared by `crates/boyko_rhi_vulkan/tests/cluster_grid_read_bound.rs`.
Until that rung these two families had **no `*_spv_sync` test at all**, so a stale artifact was a
silent failure rather than a loud one. Four of the six rows moved at VB-P1k (the `use_clusters`
capacity bound); `deferred_pbr_hwrt_vis` and `deferred_pbr_hwrt_vis_mv` did not, because
`SHADOW_STAGE=1` returns before lighting and DXC dead-strips the whole cluster block — the same
reason their `OpArrayLength` census row is 0.*

## `gbuffer_mrt.{vs,fs}.hlsl` — the mesh G-buffer raster

| Variant | `MV` | `MAT` | `.spv` | Interface delta |
|---|---|---|---|---|
| base | — | — | `gbuffer_mrt.vs.spv` / `gbuffer_mrt.fs.spv` | 3 MRT attachments (albedo / normal+id / material). |
| motion | `1` | — | `gbuffer_mrt_mv.vs.spv` / `gbuffer_mrt_mv.fs.spv` | **+ a 4th MRT** carrying per-pixel Δuv (prev-instance ring @1 + `MotionCam` UBO @2); static instance+camera ⇒ (0,0). |
| material | — | `1` | `gbuffer_mrt_pm.vs.spv` / `gbuffer_mrt_pm.fs.spv` | per-instance material PAYLOAD SSBO @1 (VERTEX) — `gAlbedo` sources the material's `base_color`, `gNormal.BA` packs the real material id; no 4th MRT. |
| motion + material (F8-mv) | `1` | `1` | `gbuffer_mrt_mvpm.vs.spv` / `gbuffer_mrt_mvpm.fs.spv` | both deltas above, combined: the 4th MRT Δuv AND the material-driven albedo/id. The nested `#if defined(MOTION_VECTORS)` inside the `PER_INSTANCE_MATERIAL` block moves `instance_materials` from binding 1 → binding 3 (bindings 1/2 stay `prev_instances`/`MotionCam`) to resolve the collision — the `motion`/`material` rows' own `.spv` are untouched by this move (the `#else` arm is byte-identical to their source). |

Reachability note: all four rows are independently reachable (`motion`/`material` are each opt-in
via one `-D`; `motion + material` needs both) — the host selects among them by binding a different
pipeline per frame (never a dynamic branch), gated on `mesh_mvpm_active()` checked BEFORE
`mesh_mv_active()`/`mesh_pm_active()` at the recorder's selection site (priority mvpm > mv > pm > base).

## `sdf_forward_march.comp.hlsl` — the Forward/VB fused SDF march+shade (compute)

One source `shaders/sdf_forward_march.comp.hlsl`; the `{HAS_MESH} x {VIEWT}` matrix, all four built
against ONE shared 14-binding Set-0 layout (+ Set 1 = the Forward shadow set verbatim): `t12`
(`gForwardDepth`) is HAS_MESH-referenced only and `u13` (`gViewT`) VIEWT-referenced only — each
bound-but-unread by the other variants (the R2 contract). Recorder selection: `mesh_leg` x
`GBufferScene::path_sdf_forward_writes_viewt()` (never a dynamic branch).

| Variant | `HAS_MESH` | `VIEWT` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|
| mesh-bounded | `1` | — | `sdf_forward_march.comp.spv` | `cs_6_0` | samples `gForwardDepth` @12 (march bounded at `t_mesh`; `sdf_owns = hit && t < t_mesh`). |
| mesh-less | — | — | `sdf_forward_march_sdfonly.comp.spv` | `cs_6_0` | no depth reference (`sdf_owns = hit`). |
| mesh-bounded + viewt | `1` | `1` | `sdf_forward_march_viewt.comp.spv` | `cs_6_0` | + `RWTexture2D<float>` `gViewT` @13 (r32f) — the TAA-under-VB gViewT producer: exactly-once per pixel (SDF `t` / mesh `t_mesh` / `1.0e30` background). |
| mesh-less + viewt | — | `1` | `sdf_forward_march_sdfonly_viewt.comp.spv` | `cs_6_0` | the depth-less sibling (SDF `t` / `1.0e30`). |

Reachability note: the VIEWT rows dispatch only under an SDF-carrying `VisibilityBuffer` leg set
with TAA armed (`path_sdf_forward_writes_viewt()`); the Forward family never arms them (no AA seam —
tripwired by a `debug_assert` in `declare_forward_graph`).

## `vb_resolve.comp.hlsl` / `vb_shade.comp.hlsl` — the VisibilityBuffer shading family (compute)

Two sources, each its own `{TEXTURED} x {FROXEL}` matrix against a shared VB-only Set-0 layout
(`vb_layout0` — 9 bindings since VB-SV0 — for the base/TEXTURED rows, `vb_layout0_froxel` — 11
bindings, `vb_layout0`'s own 0..7 plus `ClusterGrid`@8/`LightIndexList`@9 — for the FROXEL rows,
both carrying the SV0 edit-list `Buf`@10; Set 1 = the
Forward-family shadow set verbatim, Set 2 = the Decision-0 geometry table). `vb_resolve.comp.hlsl`
is the FUSED resolve (unpacks `vb_id`, re-fetches geometry, shades, writes `lit`);
`vb_shade.comp.hlsl` is its material-classified sibling (VB-P2 classification plan) — the shading
tail is character-identical between the two sources by construction (plan D3), so the `FROXEL` seam
is spliced identically into both. `TEXTURED` selects the bindless-material Set 3 eval (`vb_shade.comp.hlsl`
only — `vb_resolve.comp.hlsl` has no TEXTURED variant); `FROXEL` (VB-P1a, "dark infra") selects the
froxel-culled point/spot walk (`ClusterGrid`/`LightIndexList`) over the flat `[l0a_count,
light_count)` scan, gated at RUNTIME on the header's `clusters_enabled` bit (`use_clusters`) so an
unarmed frame on a FROXEL-compiled `.spv` still falls back to the identical flat walk. The base
(non-FROXEL) compile's tokens are byte-for-byte unperturbed by the `#else` arm — verified by re-DXC
(`vb_froxel_spv_sync.rs`).

**VB-SV0 interface delta, common to EVERY row in this family and in the split family below**
(`docs/VB-SV0-SDF-SHADOW-PLAN.md`, rung S2 "dark infra"): `+ StructuredBuffer<uint> Buf` @10
(`register(t0)`, space 0) — the SDF edit-list SSBO, the analytic `field_distance` source for the
inlined SDF soft-shadow + contact-AO terms. It sits OUTSIDE every `-D` guard, so it is present in
all ten rows and creates **no new variant**: the terms are gated at RUNTIME on light-header word 7
bits 5..6 (`load_vb_sdf_mesh_mode`), which the host writes as 0 through rung S2, and the geometry
the shadow-origin lift needs — the covered triangle's three world positions, `tri_p0`/`tri_p1`/
`tri_p2` — is armed by a SOURCE-level `#define VB_SV0` rather than a command-line `-D`.

The lift's `cross`/`dot`/`rsqrt`/orientation-flip chain is NOT in the geometry fetch: it lives in
`vb_geom_fetch.hlsli`'s `vb_sv0_face_normal` leaf, which each tail calls from INSIDE its
`sv0_mode & VB_SDF_MESH_SHADOW_BIT` gate, so an SV0-dark frame pays nothing for it beyond one
wave-uniform header read. That placement is a GATE, not a convention:
`vb_sv0_kill_switch.rs::vb_sv0_face_normal_chain_is_gated_not_straight_line` disassembles each of
the ten committed artifacts and requires the chain's basic block to be reached only under a
predicate derived from the runtime mode. It exists because nothing else in the rung can see a dark
cost — the image golden is blind to cost by construction, and the kill-switch digest compares a
compile in which the SV0 spans do not exist.

Per-row byte growth vs the pre-SV0 artifact is **+10 124 B**, uniform across all ten (e.g.
`vb_resolve.comp.spv` 47 824 → 57 952). Nearly all of it is the inlined march + AO behind the
runtime `if` — that is what "compiled in" means — plus 148 B for the face-normal guard's finiteness
test. `OpLoopMerge` goes 3 → 7 in the fused rows and 6 → 10 in the split rows and does NOT come back
down, for the same reason: the loops are present in the module, just unreachable while the mode is 0.
A later rung that widens these numbers has to say why.

All ten `.spv` were re-DXC'd and re-pinned ONCE at S2; `vb_geo.comp.spv` / `vb_geo_mv.comp.spv` and
all six `deferred_pbr` rows are byte-identical across that change and each of those is itself a gate
(`vb_sv0_kill_switch.rs`,
`cluster_grid_read_bound.rs::deferred_and_forward_families_spv_byte_identical`).

| Source | Variant | `TEXTURED` | `FROXEL` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|---|
| `vb_resolve.comp.hlsl` | base | — | — | `vb_resolve.comp.spv` | `cs_6_0` | none (the flat all-lights scan). |
| `vb_resolve.comp.hlsl` | froxel | — | `1` | `vb_resolve_froxel.comp.spv` | `cs_6_0` | `+ StructuredBuffer<uint2> ClusterGrid` @8, `+ StructuredBuffer<uint> LightIndexList` @9 (Set 0 widens to `vb_layout0_froxel`); point/spot loop walks the froxel slice when `use_clusters`, else the base flat scan. |
| `vb_shade.comp.hlsl` | base | — | — | `vb_shade.comp.spv` | `cs_6_0` | none — the classify-table pixel-selection prologue only (plan D3), otherwise character-identical to `vb_resolve.comp.hlsl`'s own base row. |
| `vb_shade.comp.hlsl` | textured | `1` | — | `vb_shade_tex.comp.spv` | `cs_6_0` | + `PerInstanceMaterialTex` ring @1 (48 B, replaces the base 32 B `PerInstanceMaterial`) + Set 3 bindless texture-array table (`gTextures[]`@0, `gTexSampler`@1) — the TV0 material-eval splice. |
| `vb_shade.comp.hlsl` | froxel | — | `1` | `vb_shade_froxel.comp.spv` | `cs_6_0` | same delta as `vb_resolve.comp.hlsl`'s own froxel row. |
| `vb_shade.comp.hlsl` | textured + froxel | `1` | `1` | `vb_shade_tex_froxel.comp.spv` | `cs_6_0` | both deltas above, combined — the two `#ifdef`s are independent, non-overlapping spans (TEXTURED touches binding 1 + Set 3; FROXEL touches bindings 8/9). |

Reachability note: every `FROXEL` row is UNBUILT-but-armable this rung — VB-P1a ("dark infra")
lands the machinery with `ResolvedRenderPath::froxel_light_cull` hardcoded OFF
(`boyko_app::runner`'s boot call site), so `GpuSceneBundles::build_froxel_light_cull` never runs in
production and no FROXEL pipeline is ever bound; a later rung (VB-P1b) reads a real
`LightingConfig`-sourced toggle to arm it. The `textured + froxel` row's host-side descriptor SET
(`vb_set0_tex_froxel`, the TEXTURED ring + `ClusterGrid`/`LightIndexList` combined) is not built
this rung either — a documented scope cut (`present/passes/vb.rs`'s own comment) for VB-P1b to
close if TEXTURED and FROXEL must co-occur.

## `vb_shade_split.comp.hlsl` — the R9 geo/shade-split lit producer (compute)

One source, a `{TEXTURED} x {HWRT}` matrix — the third VB lit producer, paired 1:1 with
`vb_geo.comp.hlsl`'s thin-aux geometry pass and selected instead of the fused
`vb_resolve`/`vb_shade` pair whenever `path_vb_split()` resolves (a pre-light consumer — SSAO,
DDGI, shadow-temporal, or the HWRT carrier — arms `mesh_geo_shade_split`). Set 0 is `vb_layout0`
verbatim; Set 1 is a DISTINCT 11-binding `vb_split_layout1` (the Forward shadow table @0-3 plus
`gSsao`@4, the DDGI atlas @5-9 and `gShadowVis`@10).

*Recorded rather than silently fixed:* these four rows had **no manifest entry at all** before
VB-SV0 S2 added this section — the same standing-rule gap `deferred_pbr_wrap.comp.spv` had until
`a4824a8`. They are added here because S2 re-pins all four and the SV0 interface delta has to be
documented somewhere. `vb_geo.comp.spv` / `vb_geo_mv.comp.spv` and `vb_raster.{vs,fs}.spv` are
STILL unlisted; SV0 does not perturb them, so closing that gap belongs in its own commit.

| Variant | `TEXTURED` | `HWRT` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|
| base | — | — | `vb_shade_split.comp.spv` | `cs_6_0` | none (the flat all-lights scan; SSAO/DDGI read at their runtime gates). |
| textured | `1` | — | `vb_shade_split_tex.comp.spv` | `cs_6_0` | + `PerInstanceMaterialTex` ring @1 (48 B) + Set 3 bindless texture-array table (`gTextures[]`@0, `gTexSampler`@1) — the same TV0 splice `vb_shade.comp.hlsl` carries. |
| hwrt | — | `1` | `vb_shade_split_hwrt.comp.spv` | `cs_6_0` | the denoised mesh-shadow visibility `gShadowVis` (Set 1 @10) REPLACES the CSM sample for the primary directional; no ray is traced here (the `shadow_vis` producer pass owns the TLAS), so `cs_6_0` still suffices. |
| textured + hwrt | `1` | `1` | `vb_shade_split_tex_hwrt.comp.spv` | `cs_6_0` | both deltas above — the two `#ifdef`s are independent, non-overlapping spans. |

Reachability note: the two `HWRT` rows require `hwrt_denoise_or_vis_on`, which is exactly the
condition `ShadowSources::SDF_SOFT_MARCH` requires to be FALSE — so VB-SV0 is compiled into them
but can never be armed while they are bound. As of rung **S4** that exclusion is MECHANICAL, not
argued: `boyko_render::render_path_config::tests::sv0_never_arms_under_hwrt` runs the truth table
through both `resolve_rules` and the production `resolve_render_path`, and its red mutation
(delete the `&& !consumers.hwrt_denoise_or_vis_on` term from `resolve_rules`' `SDF_SOFT_MARCH`
arming) was demonstrated at that rung. The runtime half is
`boyko_render::light::sync_sv0_light_gate`, which clamps the owner's request to
`ResolvedRenderPath::vb_sdf_mesh_armable()` and can therefore only ever CLEAR a bit; these two
rows are additionally covered by `record_vb`'s `note_vb_lit_producer` line, which names the bound
producer in the run log.

## `vb_geo.comp.hlsl` — the R9 thin-aux geometry pre-pass (compute)

One source, ONE axis. Paired 1:1 with `vb_shade_split.comp.hlsl` above: the split producer's geometry
half, dispatched when `path_vb_split()` resolves. It re-fetches the covered triangle through
`vb_geom_fetch.hlsli` and writes the thin aux targets the pre-light consumers read.

| Variant | `MOTION` | `.spv` | dxc `-T` | Delta |
|---|---|---|---|---|
| base | — | `vb_geo.comp.spv` | `cs_6_0` | the thin-aux write (`gThinNormal` + depth-derived view-space data) that SSAO / DDGI / the shadow-temporal reproject consume. |
| motion (R9d) | `1` | `vb_geo_mv.comp.spv` | `cs_6_0` | **+ the per-pixel camera-reprojected motion vector** for static geometry. No `rayQuery`, so the SAME `cs_6_0` target as base suffices — unlike the `deferred_pbr` HWRT rows, this axis does not move the profile. |

*Provenance: this section was **missing** until 2026-07-26. `vb_geo` has shipped since rung R9 with a
real `-D` axis and no row — the same standing-rule violation `deferred_pbr_wrap` carried until
`a4824a8`. Found while enumerating the SV0 blast radius at rung S2, which re-DXCs both rows as its
gate (b′) and must therefore prove them **unperturbed**: SV0's `#ifdef VB_SV0` seam in
`vb_geom_fetch.hlsli` is compiled only by the three lit-producer tails, never by `vb_geo`, so both
artifacts are byte-identical by construction — and that is a gate, not a hope, because `dxc -P` of
`vb_geo.comp.hlsl` with and without the guard must differ.*

## Deliberately ABSENT from this manifest: `vb_raster.{vs,fs}.hlsl`

Recorded so the next audit does not re-derive it. `vb_raster.vs.hlsl` → `vb_raster.vs.spv` (`vs_6_0`)
and `vb_raster.fs.hlsl` → `vb_raster.fs.spv` (`ps_6_0`) each compile to exactly ONE artifact and
contain **zero** preprocessor conditionals — `grep -c '#ifdef\|#if '` returns 0 on both. This file is
the registry of sources that compile to **N `.spv` via `-D`**; a single-artifact source has no
variant axis to document, so listing it would be padding rather than coverage.

That is worth stating rather than leaving implicit, because the id-encoding seam lives in exactly
these two files (`vb_raster.vs.hlsl` exports the flat instance id, `vb_raster.fs.hlsl` writes
`uint2(instance, SV_PrimitiveID)`). A future virtual-geometry rung re-encoding that pair to
`(instance, meshlet, local_tri)` would give them a real axis for the first time — at which point they
earn a section here, and every VB golden re-blesses with them.

## Shadow-denoise compute (separate shaders, not `-D` variants of the resolve)

These are distinct `.hlsl`, listed here for the temporal/spatial pipeline picture, not because they are
`-D` variants: `shadow_atrous.comp` (spatial edge-stopping à-trous over `gShadowVis`) and
`shadow_temporal.comp` (reproject + variance-clamp + accumulate against the cross-frame history pool).
The mode matrix (None / Spatial / Temporal / Both) is a **host** selection of which of these passes run
between the VIS producer and the DENOISED consumer — see `BOYKO_SHADOW_DENOISE`.

## SSAO à-trous denoise compute — `ssao_atrous.comp.hlsl` (3 format-pin variants)

Render P7 POLISH follow-up: the SSAO denoise moved OUT of `deferred_pbr.hlsl`'s resolve (the former
inline 15x15 bilateral blur) into a dedicated multi-pass edge-avoiding à-trous compute chain, mirroring
`shadow_atrous.comp.hlsl` — Dammertz 5-tap B3-spline, `step = 1 << level` per dispatch — but
TRANSCENDENTAL-FREE (integer/mul/div/clamp/min/max only, no `exp`/`pow` edge-stops) so the bit-exact
host oracle (`golden_ssao_atrous`, `boyko_rhi_vulkan::goldens`) survives. Software (NOT `hwrt`-gated).

| Variant | `-D` flag | `.spv` | `gAoIn` pin | `gAoOut` pin | Used for |
|---|---|---|---|---|---|
| interior | (none) | `ssao_atrous.comp.spv` | `r16` | `r16` | every level except the first and last |
| read8 | `SSAO_ATROUS_READ_R8=1` | `ssao_atrous_read8.comp.spv` | `r8` | `r16` | level 0 (reads the raw `sdf_ssao` R8_UNORM gather) |
| write8 | `SSAO_ATROUS_WRITE_R8=1` | `ssao_atrous_write8.comp.spv` | `r16` | `r8` | the LAST level (writes back into the frozen `gSsao` R8_UNORM the resolve reads) |

The bind-group LAYOUT is IDENTICAL across all three (4 bindings: `gAoIn`/`gAoOut`/`gViewT`/Camera UBO
+ a 4-byte `{ uint step; }` push) — only the two `OpTypeImage` pins differ. Recipe: `cs_6_0`, e.g.
`dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 -D SSAO_ATROUS_READ_R8=1 ssao_atrous.comp.hlsl
-Fo ssao_atrous_read8.comp.spv`.

## `cluster_cull.hlsl` — the Lighting-L1 clustered froxel light cull (compute)

One source `shaders/cluster_cull.hlsl`; VB-P1e rung H2 ("dark infra") grew an `#ifdef HIER`
seam: the hierarchical two-level cull (groupshared coarse AABB reduction + coarse-cull +
groupshared bitmask + fine walk over only the set bits — see
[docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md](VB-P1E-HIERARCHICAL-CULL-PLAN.md) section 4),
compiled in only under `-D HIER=1`. The base (no `-D`) arm is the flat one-thread-per-froxel
scan, UNCHANGED token-for-token by construction — the seam is physically inert on that
compile (H2 gate (b), re-DXC-verified).

| Variant | `HIER` | `.spv` | numthreads | Interface delta vs base |
|---|---|---|---|---|
| base (flat) | — | `cluster_cull.comp.spv` | `[64,1,1]` | none. |
| hierarchical | `1` | `cluster_cull_hier.comp.spv` | `[256,1,1]` | + 6 `groupshared` arrays (coarse AABB reduction + bitmask, 6 276 B) + a 2-word push tail (`ClusterCullPush` widens 16 B → 24 B: `cluster_dims_packed`, `cluster_capacity` — boot-snapshot dims/capacity, D11). Same cull-set bindings (camera UBO @0, light table @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4) — no binding added or removed. |

Reachability note: the HIER pipeline is **built but never selected** this rung — no pipeline
object is created, and no host record site's dispatch is armed to choose it (that is H3/H4).
Every golden pin stays byte-identical for the trivial reason that the module is never loaded.
The output-set equality between the two arms is a `[P1]`-class **theorem** (plan section 5),
not a spec-constant collapse — a `-D` variant here changes the entry point's `numthreads` and
groupshared declarations, which a specialization constant cannot do.

## Why these stay N `.spv` (do NOT try to spec-const-collapse)

A specialization constant is resolved at *pipeline-create* and can only change a **value** (a loop bound,
a count). It cannot: add or remove a descriptor-set binding (VIS's `gShadowVis`, HWRT's TLAS, MV's
`gMotionVec`), add a SPIR-V capability (`RayQueryKHR`), or delete half a shader (VIS strips lighting).
Every variant here does exactly one of those, so it is a genuine separate module. The good pattern is
already in place: **one `.hlsl`, N `.spv` via `-D`, ONE embed macro per `.spv`** — the source is
single, only the compiled artifact multiplies, and each artifact is a row above.

## Adding a new variant — checklist

1. **Shader**: guard the new code with `#if <FLAG>` in the existing `.hlsl` (never fork the file).
2. **Recipe**: compile offline with the frozen recipe + your `-D <FLAG>=<v>` (and `-T cs_6_5` if it needs
   `rayQuery`); commit the new `.spv` next to its siblings.
3. **Embed**: add ONE `embed_spirv! { /// doc … [#[cfg(feature = "hwrt")]] NAME_SPV, concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/<file>.spv") }` in `compute.rs` + a `pub fn <name>_spirv()` accessor. (No hand-counted size — the macro derives it.)
4. **Layout**: if the variant adds/removes a binding, add its pipeline-layout arm; keep the binding
   numbers consistent with the table above.
5. **Host**: select it by binding the right pipeline for the mode — never a runtime uniform branch.
6. **Row**: add it to this table. Gate byte-identity via the golden (`58f6c6c3`, GI-OFF) + the relevant
   `*_edsl_sync` re-emit test if the body is eDSL-generated.
