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
spec-constant cannot do that. (That SSAO *quality* axis is a source-text re-emit, not a `-D`, and
correctly stays out of the tables — but the SAME shader's `VB_THIN` axis IS a `-D` and DOES delete a
binding, so it has its own section below. Do not read "SSAO quality is excluded" as "the SSAO gather
has no variant axis".)

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
(`vb_layout0` — 8 bindings — for the base/TEXTURED rows, `vb_layout0_froxel` — 10 bindings,
`vb_layout0`'s own 0..7 plus `ClusterGrid`@8/`LightIndexList`@9 — for the FROXEL rows; Set 1 = the
Forward-family shadow set verbatim, Set 2 = the Decision-0 geometry table). `vb_resolve.comp.hlsl`
is the FUSED resolve (unpacks `vb_id`, re-fetches geometry, shades, writes `lit`);
`vb_shade.comp.hlsl` is its material-classified sibling (VB-P2 classification plan) — the shading
tail is character-identical between the two sources by construction (plan D3), so the `FROXEL` seam
is spliced identically into both. `TEXTURED` selects the bindless-material Set 3 eval (`vb_shade.comp.hlsl`
only — `vb_resolve.comp.hlsl` has no TEXTURED variant); `FROXEL` (VB-P1a, "dark infra") selects the
froxel-culled point/spot walk (`ClusterGrid`/`LightIndexList`) over the flat `[l0a_count,
light_count)` scan, gated at RUNTIME by `use_clusters` so an unarmed frame on a FROXEL-compiled
`.spv` still falls back to the identical flat walk. Since VB-P1k that gate is THREE terms, uniform
across all four `ClusterGrid` readers: `clusters_enabled != 0 && cluster_count != 0 &&
cluster_count <= grid_capacity`, where the capacity comes from `ClusterGrid.GetDimensions(...)` —
the BOUND descriptor's own element count (SPIR-V `OpArrayLength`), not a host-side mirror of it.
The two terms past the enabled bit are an out-of-bounds guard, not a style choice:
`robustBufferAccess` is OFF in this engine and no GPU-assisted validation runs, so an out-of-range
`ClusterGrid` read is real UB that no layer would report. `cluster_grid_read_bound.rs` pins the
bound's presence per artifact (`OpArrayLength` census) and closes the consumer set over the
committed HLSL roots. The base
(non-FROXEL) compile's tokens are byte-for-byte unperturbed by the `#else` arm — verified by re-DXC
(`vb_froxel_spv_sync.rs`).

⚠️ **A dark feature is not a free feature — MEASURED.** The VB-SV0 stage compiled an SDF
soft-shadow + contact-AO march into all ten rows of this family and the split family below, behind
a runtime light-header gate that shipped writing `0`. Every image golden held, and the cost was
real anyway: with **both** A/B phases dark, the fused lit-producer dispatch went 24 576 / 23 552 ns
before to 41 984 / 41 984 ns after — **+17.4..18.4 µs, ≈ +75 %**, on every VB frame of every VB
scene, purely for CARRYING the term. It was reverted for that (the ten rows are back at their
pre-SV0 bytes, `vb_resolve.comp.spv` 47 824 B). The lesson belongs in this manifest rather than
only in the plan: neither gate that stage ran could see it — image byte-identity is blind to cost
by construction, and the paired A/B cancelled it because both arms carried the same module. A rung
that widens a shipped `.spv` owes a dark-path measurement, not only a byte-identity pin.

| Source | Variant | `TEXTURED` | `FROXEL` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|---|
| `vb_resolve.comp.hlsl` | base | — | — | `vb_resolve.comp.spv` | `cs_6_0` | none (the flat all-lights scan). |
| `vb_resolve.comp.hlsl` | froxel | — | `1` | `vb_resolve_froxel.comp.spv` | `cs_6_0` | `+ StructuredBuffer<uint2> ClusterGrid` @8, `+ StructuredBuffer<uint> LightIndexList` @9 (Set 0 widens to `vb_layout0_froxel`); point/spot loop walks the froxel slice when `use_clusters`, else the base flat scan. |
| `vb_shade.comp.hlsl` | base | — | — | `vb_shade.comp.spv` | `cs_6_0` | none — the classify-table pixel-selection prologue only (plan D3), otherwise character-identical to `vb_resolve.comp.hlsl`'s own base row. |
| `vb_shade.comp.hlsl` | textured | `1` | — | `vb_shade_tex.comp.spv` | `cs_6_0` | + `PerInstanceMaterialTex` ring @1 (48 B, replaces the base 32 B `PerInstanceMaterial`) + Set 3 bindless texture-array table (`gTextures[]`@0, `gTexSampler`@1) — the TV0 material-eval splice. |
| `vb_shade.comp.hlsl` | froxel | — | `1` | `vb_shade_froxel.comp.spv` | `cs_6_0` | same delta as `vb_resolve.comp.hlsl`'s own froxel row. |
| `vb_shade.comp.hlsl` | textured + froxel | `1` | `1` | `vb_shade_tex_froxel.comp.spv` | `cs_6_0` | both deltas above, combined — the two `#ifdef`s are independent, non-overlapping spans (TEXTURED touches binding 1 + Set 3; FROXEL touches bindings 8/9). |

Reachability note (**re-derived 2026-07-27 — supersedes BOTH the VB-P1a text and its first
"correction", which mislocated the hardcode and the date**). The timeline, from the commits:

* **At VB-P1a (`78d0534`, 2026-07-24) the original note was ACCURATE as written.** The hardcode sat
  exactly where it said — `boyko_app`'s boot call site: `crates/boyko_app/src/runner.rs:454` in that
  commit is the literal `clusters_wanted: false`, whose own comment named VB-P1b as the rung that
  would read the real toggle. The RESOLVER was never the hardcoding site: at VB-P1a
  `render_path_config.rs:914` already carried today's live expression, `consumers.clusters_wanted &&
  matches!(path, RenderPath::VisibilityBuffer)`. Refuting the note by pointing at `resolve_rules`
  refutes a claim it never made.
* **It went STALE at VB-P1b (`d60d95b`, the same day)**, which replaced that literal with the live
  probe `app.world().try_resource::<LightingConfig>().is_some_and(|c| c.clusters_enabled)`. From
  that commit onward "never runs in production" was false for any scene that opts in — the note was
  simply never revisited.

Today: `froxel_light_cull` is a per-boot resolved bit (`consumers.clusters_wanted && path ==
VisibilityBuffer`, unchanged since VB-P1a; its `froxel_light_cull_is_vb_only` test pins the VB-only
scoping, and its FIELD doc in `render_path_config.rs` WAS updated at VB-P1b and is current), the
runner threads `clusters_wanted` from the booted scene's `LightingConfig::clusters_enabled`, and
`GpuSceneBundles::build_froxel_light_cull` runs under `if resolved_render_path.froxel_light_cull`
there — creating all three FROXEL pipelines together. The DEFAULT is unarmed (`clusters_enabled`
defaults `false`), so a scene that never opts in builds, declares and records nothing here and every
pre-VB-P1b golden stays byte-identical. "Defaults off" is not "never runs".

**Image-pin coverage is TWO rows of three, not "every FROXEL row"** (checked against
`goldens/PINS.toml`, which contains exactly two FROXEL-bearing pins, both blessed to real hashes on
the software AND hwrt legs):

| FROXEL row | Blessed image pin |
|---|---|
| `vb_resolve_froxel.comp.spv` | `[vb_mesh_froxel]` — VB-P1b (`d60d95b`), `fb220ff3…`, the FUSED arm |
| `vb_shade_tex_froxel.comp.spv` | `[vb_mesh_tex_froxel]` — VB-P1c (`b2b1240`), `6d7ea00d…`, the CLASSIFIED+TEXTURED arm |
| `vb_shade_froxel.comp.spv` | **NONE** — no image golden executes it (below) |

Both are EQUALITY pins: the same scene re-rendered with `BOYKO_VB_FROXEL_FORCE_OFF`
(`clusters_enabled = false`) must hash identically. `vb_shade_froxel` is the classified-but-UNTEXTURED
cell of the `(textured, froxel)` match, and `vb_use_classified = vb_force_classified ||
vb_tex_active` (`gpu_scene`) — so reaching it needs clustering armed AND the
`BOYKO_VB_FORCE_CLASSIFIED` dev override, since a textured armed frame lands in `(true, true)`
instead. No `PINS.toml` row sets that variable. Its pipeline IS created on every armed boot and its
match arm IS live, but no blessed pin ever dispatches it; the only gate on those bytes is the re-DXC
test below.

The `textured + froxel` row's host-side descriptor SET (`vb_set0_tex_froxel`, the TEXTURED ring +
`ClusterGrid`/`LightIndexList` combined) WAS the VB-P1a scope cut, and **VB-P1c (`b2b1240`) closed
it**: the set is built in `present/targets.rs` (the `vb_set0_tex_froxel` builder — `Some` iff
`vb_layout0_froxel` + `cluster_grid` + `light_index` + `vb_tex_instance_material_ring` +
`vb_shade_tex_froxel_pipeline` are all `Some`), and `present/passes/vb.rs`'s `(textured, froxel)`
match arm `expect`s BOTH `scene.vb_shade_tex_froxel_pipeline` and `targets.vb_set0_tex_froxel` to
be `Some` — a mis-resolved boot panics loudly there instead of silently falling back to the
non-froxel pipeline. ⚠️ The prose at BOTH of those sites is itself VB-P1a-era and never refreshed:
each still says the arm bit is "hardcoded OFF" and the set "ALWAYS `None` in production this rung",
and each cites `ResolvedRenderPath::froxel_light_cull`'s doc — which VB-P1b DID update, so the
comments now contradict their own referent. Trust the `expect`, the field doc and the pins, not the
prose beside them.

`.spv` byte gate: `crates/boyko_rhi_vulkan/tests/vb_froxel_spv_sync.rs` —
`vb_froxel_variant_spv_byte_identical` re-DXCs the three FROXEL rows (`vb_resolve_froxel`,
`vb_shade_froxel`, `vb_shade_tex_froxel`), and `vb_base_variant_spv_unperturbed_by_the_froxel_seam`
re-DXCs the three base rows to prove the `#else` arm is physically inert. ⚠️ Both tests **SKIP
rather than fail** when no `dxc` resolves on the host — and `vb_shade_froxel` has nothing else.

## `vb_shade_split.comp.hlsl` — the R9 geo/shade-split lit producer (compute)

One source, a `{TEXTURED} x {HWRT}` matrix — the third VB lit producer, paired 1:1 with
`vb_geo.comp.hlsl`'s thin-aux geometry pass and selected instead of the fused
`vb_resolve`/`vb_shade` pair whenever `path_vb_split()` resolves (a pre-light consumer — SSAO,
DDGI, shadow-temporal, or the HWRT carrier — arms `mesh_geo_shade_split`). Set 0 is `vb_layout0`
verbatim; Set 1 is a DISTINCT 11-binding `vb_split_layout1` (the Forward shadow table @0-3 plus
`gSsao`@4, the DDGI atlas @5-9 and `gShadowVis`@10).

*Provenance: these four rows had **no manifest entry at all** until 2026-07-26 — the same
standing-rule gap `deferred_pbr_wrap.comp.spv` carried until `a4824a8`. Found while enumerating the
VB-SV0 blast radius, which had to re-pin all four; the section outlived that stage's revert because
the rows themselves ship regardless of it.*

| Variant | `TEXTURED` | `HWRT` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|
| base | — | — | `vb_shade_split.comp.spv` | `cs_6_0` | none (the flat all-lights scan; SSAO/DDGI read at their runtime gates). |
| textured | `1` | — | `vb_shade_split_tex.comp.spv` | `cs_6_0` | + `PerInstanceMaterialTex` ring @1 (48 B) + Set 3 bindless texture-array table (`gTextures[]`@0, `gTexSampler`@1) — the same TV0 splice `vb_shade.comp.hlsl` carries. |
| hwrt | — | `1` | `vb_shade_split_hwrt.comp.spv` | `cs_6_0` | the denoised mesh-shadow visibility `gShadowVis` (Set 1 @10) REPLACES the CSM sample for the primary directional; no ray is traced here (the `shadow_vis` producer pass owns the TLAS), so `cs_6_0` still suffices. |
| textured + hwrt | `1` | `1` | `vb_shade_split_tex_hwrt.comp.spv` | `cs_6_0` | both deltas above — the two `#ifdef`s are independent, non-overlapping spans. |

Reachability note: the two `HWRT` rows require `hwrt_denoise_or_vis_on`, which is exactly the
condition `ShadowSources::SDF_SOFT_MARCH` requires to be FALSE — so no SDF-march-sourced shadow
term can ever be armed while they are bound. That exclusion rests on the boot resolver's predicate
alone and has no mechanical check at the site where the pipeline is selected.

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
`a4824a8`. Found while enumerating the VB-SV0 blast radius, which had to prove both rows
**unperturbed**; the section outlived that stage's revert because the rows themselves ship
regardless of it.*

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

## `sdf_ssao.comp.hlsl` — the screen-space AO gather (compute)

One source `shaders/sdf_ssao.comp.hlsl`, ONE `-D` axis: `VB_THIN` (six `#if VB_THIN` spans). The
artifact count is 6, and it takes TWO independent multiplications to get there — only the second
one belongs to this file:

* The **quality** axis is NOT a `-D`. `boyko_shaderdsl::ssao::variant_hlsl` (driven by the
  `emit_ssao_variants` bin) re-emits the base text with ONLY the `static const SSAO_*` tuning header
  swapped, producing three committed sources `sdf_ssao_{low,medium,high}.comp.hlsl`. That is the
  "perf-justified, kept as 3 files" family the intro excludes — the `[unroll]` bounds must be
  compile-time literals.
* The **`VB_THIN`** axis IS a `-D`, and it belongs here: it DELETES a binding and renumbers the
  rest, which no specialization constant can do. Each of the three quality `.hlsl` is DXC'd twice —
  bare, and with `-D VB_THIN=1` — so 3 sources become 6 `.spv`.

The base `sdf_ssao.comp.spv` was RETIRED at Render P7-Q2: the base `.hlsl` is the single-source
glue template, not a shipped artifact (the engine loads the per-quality `.spv`), which is why the
`-Fo sdf_ssao.comp.spv` line still in that file's own header names no committed file.

| Variant | quality `.hlsl` | `VB_THIN` | `.spv` | dxc `-T` | Interface delta vs the base 5-binding table |
|---|---|---|---|---|---|
| base, Low | `sdf_ssao_low.comp.hlsl` | — | `sdf_ssao_low.comp.spv` | `cs_6_0` | none — `gNormal`@0 / `gMaterial`@1 / `gViewT`@2 / `ssao`@3 / Camera UBO@4 (`ssao_layout`). 2 slices × 3 steps. |
| base, Medium | `sdf_ssao_medium.comp.hlsl` | — | `sdf_ssao_medium.comp.spv` | `cs_6_0` | none. 2 × 4; bakes today's shipped module consts, so its `.hlsl` is byte-identical to the base template (the no-op proof). |
| base, High | `sdf_ssao_high.comp.hlsl` | — | `sdf_ssao_high.comp.spv` | `cs_6_0` | none. 8 × 6. |
| VB_THIN, Low | `sdf_ssao_low.comp.hlsl` | `1` | `sdf_ssao_vb_low.comp.spv` | `cs_6_0` | `gMaterial` **DROPPED ENTIRELY** (no slot reserved — unlike `sdf_forward_march`'s bound-but-unread precedent, because this variant gets its OWN layout, not a shared one); `gNormal` → `vb_geo`'s `gThinNormal` at the SAME slot 0; the mask test becomes `view_t < SSAO_VIEWT_BG` (`1e30`) off the already-bound `gViewT`. The survivors RENUMBER into a DENSE 4-binding table — `gThinNormal`@0 / `gViewT`@1 / `ssao`@2 / Camera@3 — a DIFFERENT host layout (`vb_ssao_layout`), not a hole in the 5-binding one. |
| VB_THIN, Medium | `sdf_ssao_medium.comp.hlsl` | `1` | `sdf_ssao_vb_medium.comp.spv` | `cs_6_0` | same delta. |
| VB_THIN, High | `sdf_ssao_high.comp.hlsl` | `1` | `sdf_ssao_vb_high.comp.spv` | `cs_6_0` | same delta. |

The march/dither/slice math, the tuning header, and the eDSL-GENERATED `ssao_horizon_step` span are
byte-identical TEXT on both sides of the axis — the span reads only function-local names
(`Pp`, `P`, `n`, `hc`), never a binding, so the renumbering cannot touch it. Recipe (cwd = the
shaders dir, so the relative `#include "ray_gen.hlsli"` resolves), frozen `dxc` 1.4.350.0:

```
dxc -spirv -T cs_6_0 -E main [-D VB_THIN=1] -fspv-target-env=vulkan1.3 \
    sdf_ssao_<quality>.comp.hlsl -Fo sdf_ssao_[vb_]<quality>.comp.spv
```

**Gating — every row is byte-gated, not merely described.** All in
`crates/boyko_rhi_vulkan/tests/ssao_edsl_sync.rs`:

* `ssao_spv_byte_identical` — the three **base** rows. Per quality it asserts three things: the
  committed variant `.hlsl` equals a fresh `variant_hlsl(base, preset)` re-emit, the committed
  `.spv` equals a fresh re-DXC of that `.hlsl`, and (Medium only) the variant `.hlsl` equals the
  base `.hlsl` byte-for-byte.
* `ssao_vb_thin_spv_byte_identical` — the three **`VB_THIN`** rows: a fresh `-D VB_THIN=1` re-DXC of
  the SAME committed non-VB quality `.hlsl` must equal the committed VB `.spv`. No separate VB
  `.hlsl` exists; the define is the entire difference.
* `ssao_horizon_step_matches_edsl_emit` — the eDSL `.contains` drift gate, iterated over the base
  template AND all three quality `.hlsl`.

⚠️ Both re-DXC tests **SKIP rather than fail** when `dxc` is absent from the host. A green run on a
machine without the Vulkan SDK proves nothing about these six blobs.

Reachability note: selection is by binding a different pipeline, never a dynamic branch. The base
rows go through `compute::sdf_ssao_spirv_variant(q)` against the 5-binding `ssao_layout`; the
`VB_THIN` rows through `compute::sdf_ssao_vb_spirv(q)` against `vb_ssao_layout`, gated by
`path_vb_ssao()` (`mesh_geo_shade_split && ssao.is_some()`) — so they are reachable only on the R9
VB geo/shade split, where there is no material G-buffer to read a mask from. All three `VB_THIN` pipelines are created at boot (a loop
over `SSAO_QUALITY_COUNT` in `boyko_app::gpu_scene`); `ResolvedSsao::variant` picks one per frame.

**Image-golden coverage is ONE row of six, and that is worth stating.** `[vb_mesh_ssao]` boots
`SsaoConfig { quality: High, atrous_levels: 3 }`, so it pins `sdf_ssao_vb_high.comp.spv` and only
that. `vb_mesh_ssao_screenshot_dump` is the ONLY SSAO-arming `test_name` in `PINS.toml`: no pin row
sets `BOYKO_SSAO`, so `grand_showcase_2mat` (the one pinned scene that reads it) resolves
`SsaoQuality::Off` under its blessed env (the 0%-gate), and `window_present_gbuffer`'s own six
`engine_ssao_*` dumps are unpinned. So **no blessed pin ever executes the base gather at all**. The other five rows rest entirely on the re-DXC byte gates
above plus the host-oracle mirrors (`ssao_horizon_step_host_matches_edsl_eval`,
`ssao_consts_host_match_edsl`, `ssao_params_table_host_match_edsl`).

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

## Cluster-buffer capacity bounds — census, and one TRACKED OPEN GAP

Two device buffers carry the L1 froxel cull's output: `ClusterGrid` (`uint2 {offset, count}` per
froxel) and `LightIndexList` (the flat surviving-index slices). Both are sized ONCE at scene boot
(`boyko_app/src/gpu_scene/mod.rs`, `build_froxel_light_cull`) and never re-allocated, while the
light-table header they are indexed through is republished from the LIVE `ClusterConfig` every
frame — so both need a bound that cannot drift from the allocation. `robustBufferAccess` is OFF
and no GPU-assisted validation runs, which is why a miss here is silent.

`ClusterGrid` has that bound on both sides. **`LightIndexList` does not, and that gap is open.**

| Buffer | Side | Bound | Anchored on |
|---|---|---|---|
| `ClusterGrid` | write (`cluster_cull.hlsl` base arm) | `min(dim_x*dim_y*dim_z, GetDimensions())` | **the allocation** (`OpArrayLength`) — VB-P1j |
| `ClusterGrid` | write (`cluster_cull.hlsl` HIER arm) | `fi < pc.cluster_capacity` | a pushed BOOT snapshot minted from the same `ClusterConfig` binding the buffer was allocated from (D11) |
| `ClusterGrid` | read (4 consumers) | `use_clusters &&= cluster_count <= GetDimensions()` | **the allocation** (`OpArrayLength`) — VB-P1k |
| `LightIndexList` | write (both cull arms) | `offset + write_count <= pc.index_list_cap` | a pushed BOOT snapshot (same construction as D11) |
| `LightIndexList` | read (4 consumers) | *(none local)* — `ps_offset`/`ps_count` are taken from `ClusterGrid[cluster]` verbatim | **nothing in the reading shader** — see below |

### Why the reads are in bounds today

A three-hop chain, no hop of which is local to the reading shader: the reader touches at most
`cell.x + cell.y - 1`; the cull publishes a cell only after clamping `offset + write_count <=
pc.index_list_cap`; and the host mints that push word and allocates the buffer from the SAME
`cluster_config.index_list_cap` in the same function, so the pushed cap equals the allocation's
element count and is never re-minted. **No reachable out-of-bounds read is claimed.** What is
claimed is that the reading shader bounds nothing itself: were a `ClusterGrid` cell ever read that
this cull did not produce, `cell.x`/`cell.y` are arbitrary and the read is bounded by nothing,
while the `ClusterGrid` read beside it would still be bounded by VB-P1k. The only guard against
that state today is the header's `cluster_count != 0` term, since `boyko_render::light` publishes
the dims lane only on a VB-froxel boot — an *enabled-bit* guard, not an allocation one.

### What closing it costs

MEASURED, not estimated. A probe compile of `vb_resolve.comp.hlsl` with the clamp added
(`LightIndexList.GetDimensions(ilc, ils); ps_count = (ps_offset >= ilc) ? 0u : min(ps_count, ilc -
ps_offset);`) compiles clean under the frozen recipe and emits the expected second
`OpArrayLength %uint %LightIndexList 0`, moving that one artifact **49 996 → 50 180 B (+184)**.

* **8 committed reader `.spv` move**: `deferred_pbr{,_wrap,_hwrt,_hwrt_denoised}.comp.spv`,
  `forward_opaque_froxel.fs.spv`, `vb_resolve_froxel.comp.spv`, `vb_shade{,_tex}_froxel.comp.spv`.
  (`deferred_pbr_hwrt_vis{,_mv}` do not — `SHADOW_STAGE=1` returns before lighting and DXC
  dead-strips the block. The cull's own two arms already clamp on the write side.)
* **All 8 are byte-gated** — five by `tests/cluster_grid_read_bound.rs`, three by
  `tests/vb_froxel_spv_sync.rs` — so the edit reds those re-DXC gates by construction and needs
  all eight re-blessed together.
* **Golden pins**: the clamp is inert on every consistent frame, so the pins are *expected* not to
  move — but expected is not measured, and confirming it needs fresh GPU runs of `forwardplus_mesh`,
  `vb_mesh_froxel` and `vb_mesh_tex_froxel`.
* **Not an eDSL edit**: all four read sites were VERIFIED outside every
  `// === GENERATED … BEGIN/END ===` region, so this is a legal hand-edit of the HLSL rather than a
  `boyko_shaderdsl` re-emit.

The gap is pinned executably by `the_light_index_list_capacity_bound_is_a_tracked_open_gap`
(`tests/cluster_grid_read_bound.rs`), which asserts 0 `OpArrayLength` on `%LightIndexList` across
all 10 artifacts that reference it. **A rise to 1 is the fix landing, not a regression** — re-bless
the moved `.spv`, re-run the froxel pins, and re-pin that test.

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
