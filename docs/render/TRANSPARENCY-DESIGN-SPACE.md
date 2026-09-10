# Transparency — the design for THIS engine

> Status: architect's design, 2026-09-10, **pass 1 — critiqued once** (§15 holds the critic's
> sixteen findings and what this revision did with each; the five blocking ones each moved text
> above, and every repair re-opened its own `file:line`). The survey it rests on is
> [`TRANSPARENCY-RESEARCH.md`](TRANSPARENCY-RESEARCH.md) (§0 there lists what the tree holds today;
> every `file:line` below was re-opened at commit **`ed0bed45`** on `feat/multi-paradigm-render`).
> Numbers are either measured elsewhere and cited with their rig, or **labelled estimates** —
> nothing was timed for this document and no `cargo` command was run. §13 separates the
> PERF/ARCHITECTURE decisions taken here from the VALUES/SCOPE decisions that go to the owner.
> This directory is not machine-anchored (`tests/internal_docs_anchors.rs:231`).
>
> **Anchor re-verification (a second architect pass, same day, same commit).** The pass that
> wrote this document did not return, so a second pass re-opened every `file:line` here before
> accepting it. Four anchors had drifted (`RESEARCH.md` §13 lists them with the lines they moved
> to); one participation-matrix row was missing (the HW-RT TLAS, §7) and one number's precondition
> was under-stated (R7's fp16 figure, §14). Nothing in §13.A changed.

## 0. The shape in one paragraph

Transparency is **one exit, one seam, and a ladder of what the seam can carry**. The exit is at
the **gather**, not in a shader: an entity whose material routes to `Translucent` carries a
structural marker (the `OcclusionCulling` shape, `crates/boyko_render/src/occlusion_marker.rs:1-8`)
and is bucketed by a second gather into its own `ScratchColumn` scratch (the `CsmCasterScratch`
shape, `crates/boyko_render/src/csm_caster.rs:1-30`) — so it never enters a VB `DrawBatch`, never
writes `vb_id`, never feeds the HZB or the classifier, and the visibility buffer's **opaque
variant** stays what `vb_raster.fs.hlsl:1-5` says it is: one id per pixel, no discard (R2's
`MASKED` variant adds a `discard` and is a separately pinned module — §9). **Cutout stays on the
opaque rail** as that `MASKED` raster variant with a hashed threshold, because a masked pixel is
still one surface. The seam is the slot that already exists on all three declarators — after the path's last
`lit` producer, before `taa_resolve` (`graph_bridge.rs:2447-2462`, `:2983-2991`, `:6170-6185`) —
where a path-agnostic forward pass draws the sorted translucent meshes into `lit` with the
premultiplied blend that already exists (`enums.rs:913`), depth-tested read-only against the path's
own depth with D7's compare table (`docs/PARTICLES-PLAN.md:461-476`), lit through the **same
three-term `use_clusters` runtime gate every lit producer in the tree already carries**
(`forward_opaque.fs.hlsl:368-390`, `deferred_pbr.hlsl:1264-1290`, `vb_resolve.comp.hlsl:423`,
`vb_shade.comp.hlsl:578`) — the froxel lists where a cull is built (VB only,
`render_path_config.rs:1224`), the flat `[l0a_count, light_count)` table everywhere else; the
lists themselves are built without reading depth (`cluster_cull.hlsl:436-455`) — shadowed by
`shadow_apply.hlsli`, and probed by `ddgi_probe_sample` under its `ddgi_mode != 0` header gate.
Particles keep their own draw immediately after it; UI text stays outside. Every rung above that —
hashed cutout, a scene-colour mip chain for refraction, glTF transmission/volume/IOR as a **cold
extension table** beside the frozen 48-B `MaterialGpu`, a TAA coverage mask, moment-based OIT (the
one family the current blend surface can express, RESEARCH §4.2), translucent shadow transmittance,
reflections — is a **new pass or a new column, never a new path**, and every **leaf** body is eDSL
with an `f32` oracle, wrapped in skeletons that §9 names file by file (the eDSL authors no
sampling, `discard`, stores or atomics — `emit_particles.rs:34-36` — so a skeleton is either a
generator-owned template or hand-written HLSL pinned by an `*_spv_sync`).

## 1. Data model

### 1.1 The route — three markers, one authority

```
MaterialRoute { Opaque, Masked { hashed: bool }, Translucent }   // derived, never authored twice

Material::route(&self) -> MaterialRoute:
    Translucent  if blend ∈ {Blend, Additive, Multiply} || x.transmission > 0.0   // glTF: transmission is NOT alphaMode
    Masked       if blend == Mask { .. }
    Opaque       otherwise
```

`blend` carries the three mesh blend classes every surveyed engine ships (Bevy `AlphaMode::{Blend,
Premultiplied, Add, Multiply}` [D docs.rs/bevy/latest/bevy/prelude/enum.AlphaMode.html, opened by
the critique 2026-09-10]): `Blend` = premultiplied OVER (the only class the first pass had),
`Additive` = `BlendState::ADDITIVE` (exists, with its commutativity proof, `enums.rs:935-948` — so
this class ships **unsorted**), `Multiply` = `(DstColor, Zero)` — behind TK-1's enum growth, so it
lands with R9, not R1. The route is the same for all three; the class is a `MaterialXGpu` flag
(§1.2) the gather reads into the run key (§1.3).

- **`Translucent`** — a ZST component; presence = "leaves the opaque geometry path, rejoins at
  the seam". **`AlphaMasked`** — a ZST; presence = "opaque rail, masked raster/caster pipelines".
  Both are ordinary components, not `EnableTag`s: over a flag `With<F>` never matches and
  `Without<F>` excludes nothing (`memory: reference-flag-filter-polarity`), and the gathers below
  *are* `With`/`Without` filters.
- **One authority derives them**: the spawn site (Gaia bake for authored scenes, §5; the
  `spawn_mesh` helper for code) reads `Material::route()` and inserts the marker — the
  `MATERIAL_FLAG_TEXTURED` discipline, where one boundary re-derives the bit
  (`crates/boyko_render/src/material.rs:112-121`). A debug system asserts marker ⇔ route each
  frame (G3), so the second authority is a *check*, never a *source* (RESEARCH P32).
- A material whose route changes at runtime (an editor edit) is a `Commands` insert/remove of
  the marker — cold path, never per frame.

### 1.2 The material extension table — hot/cold split around the frozen 48 B

`MaterialGpu` stays byte-identical (`material.rs:82-102`: 48 B, 12 words, every golden). The
transparency parameters are a **second SSBO** indexed by the same 16-bit `MaterialId`
(`material_table.rs:1-30`), dense, one row per material, zero-filled for opaque rows:

```
MaterialXGpu {                    // 48 B, #[repr(C, align(16))], 3 std430 lanes, const-asserted
    lane0: [transmission, ior, thickness, dispersion],                 // glTF names, glTF defaults
    lane1: [attenuation_color.rgb, attenuation_distance],              // T(x) = c^(x/d); +inf = none
    lane2: [specular_factor, packed_specular_color(u32→f32 bitcast), flags, sort_bias],
}
// flags bits: 0 = MASK_HASHED, 1 = TWO_SIDED_SORTED, 2 = SHADOW_TRANSLUCENT, 3 = SHADOW_NONE,
//             4..5 = BLEND_CLASS {0 premultiplied OVER, 1 additive, 2 multiply (TK-1)}
// sort_bias: authored order override (UE "Translucency Sort Priority" [D]), applied in the key
// diffuse_transmission (glTF KHR_materials_diffuse_transmission, Bevy `diffuse_transmission`)
// is NOT in this row: it needs a fourth lane (64 B, a 4.2 MB ceiling) — ballot 13, priced there.
```

Numbers: 48 B × 65 536 rows (the `MaterialId` ceiling) = **3.1 MB** worst case, 48 B × the live
row count in practice; the opaque paths **never bind it**, so the default-path golden argument
(`docs/MULTI-PARADIGM-RENDER-PLAN.md:26`) is untouched. A 64-B base row would have changed
`MATERIAL_GPU_WORDS = 12` in every shader that reads `Materials` and re-blessed every pin for a
lane most rows never use — that is the number that decides the split (D2).

The CPU authority is `Material { gpu, textures, x: MaterialXGpu }` with `x` defaulting to the glTF
defaults (`transmission 0, ior 1.5, thickness 0, dispersion 0, attenuation (1,1,1,+inf), specular
(1, white)`); `Material::new` keeps its signature and `with_x(..)` sets the lane, mirroring
`with_textures` (`material.rs:228`). `base_color.w` is the cutoff/alpha it already claims to be
(`material.rs:64`) and gains its **first reader** in rung 2.

### 1.3 The translucent gather scratch — the sort key is a column

```
TranslucentRenderScratch(MeshRenderScratch)   // the CsmCasterScratch newtype shape
    + sort_keys: ScratchColumn<u32>   // 16-bit inverted log-depth key << 16 | (mesh_id & 0xFFFF)
    + order:     ScratchColumn<u32>   // the permutation the two-pass radix writes
    + runs:      ScratchColumn<DrawRun { mesh_id, first_slot, count, two_sided, blend_class }>
```

Every lane is a `ScratchColumn` (`crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:39-44`
— a `ComponentPool`, never a `Vec`), cleared and refilled per frame under the same
`pool_reserve_rows` ceiling `mesh_draw.rs:24-32` describes.

**The key leaf does not exist yet — TK-7 creates it.** The first pass called the key "the
particle sort key's own eDSL leaf"; the tree has no such leaf. The particle key is a hand-written
HLSL string constant printed into two files (`emit_particles.rs:216-241`, `SORT_KEY_FN` — "spelled
ONCE and printed into BOTH the histogram and the scatter"); it has no `FieldScalar` body, no `f32`
instantiation and no host oracle, and the eDSL's op vocabulary has neither `log2` nor a
float→integer quantise (`crates/boyko_shaderdsl/src/scalar.rs:56-172`: `lit / add / sub / mul /
div / neg / min / max / clamp01 / lerp / abs / sqrt / select` and the compares). TK-7 is
therefore, in order: (a) two op growths, `log2` and `quantize_u(x, max) = (uint)(x·max + 0.5)`
(the `+ 0.5` is the particle key's own rounding, `:240`), each with its `f32` and `Emit` arm;
(b) the leaves `log_depth_unit(d, log_near, inv_log_span) → u ∈ [0,1]` and
`sort_key(u, bias, KEY_BITS)`; (c) `SORT_KEY_FN` re-emitted **from** those leaves and
`particle_edsl_sync` re-run — the two committed sort `.spv` are re-blessed only if the tokens move.
Only after (c) does "one body, one oracle, the CPU and GPU keys cannot drift" hold (D7 amended).

**The range is the particle key's fixed range, stated here as a contract — not the camera's
`far/near`.** `d ∈ [0.125, 4096]` world units: `SORT_LOG_NEAR = −3`, `SORT_LOG_SPAN = 15` octaves,
`saturate`d at both ends (`emit_particles.rs:184-192`, `:231`). Per-bin precision on the real
leaf: 8 bits ⇒ 15/256 = **0.0586 octaves per bin ⇒ 4.15 %** relative depth (the generator's own
figure, `:188`); 16 bits ⇒ 15/65536 = **2.29 × 10⁻⁴ octaves ⇒ 0.0159 %** (1.6 mm at 10 m,
5.6 mm at 35 m) [D-arith]. The first pass's `ln(10⁴)`-based 3.7 % / 0.014 % assumed a range the
leaf does not have and is withdrawn. Consequences stated rather than hidden: every translucent
farther than 4096 units or nearer than 0.125 units keys to the same saturated bin and **ties with
every other saturated one** (the stable sort then leaves them in gather order); a diagnostic
counter of saturated keys (the G9 shape, G15) makes that visible. A camera-derived
`(log_near, inv_log_span)` is a later knob the leaf already takes as inputs — the body does not
change, only the constants the two instantiations pass; v1 passes the particle constants so the
mesh bucket and the alpha-particle bucket key the same depth the same way. The key is inverted so
an ascending sort is back-to-front — the particle scatter's own convention
(`PARTICLES-PLAN.md:527`: "The KEY is INVERTED (bin 0 = farthest) so the ascending scatter is
back-to-front"). `sort_bias` (§1.2) is added in log space before quantisation, so a bias of ±1.0
moves an object one full depth-range in the order without touching its geometry.

The sort is a **two-pass LSD counting sort over bits 16..31** of the 32-bit key word — the two
8-bit digits of `depth16`; the low half (`mesh_id & 0xFFFF`) is **payload, never a sort digit**,
so it is two passes, not four (2 × 256 counters, 2 × N scatters): at N = 4 096 translucent
instances that is ≈ 17 k simple ops — **estimate < 20 µs**, below the gather's own count → prefix →
scatter cost. The permutation is then walked once to emit `DrawRun`s: consecutive slots with the
same **`(mesh_id, two_sided, blend_class)`** merge into one `vkCmdDrawIndexed` (`base_instance`
addressing, `mesh_draw.rs:1-12`). The run key is not `mesh_id` alone: `two_sided` and the blend
class are per-**material** flags (§1.2) and materials are per-**instance**
(`PerInstanceMaterialTex`, `mesh_draw.rs:155-182`), so one mesh's instances can carry both values,
and a mixed run could be issued neither as one `Front`-then-`Back` pair nor under one blend
pipeline. The worst case is one draw per instance. **Estimate, uncited** (no source exists for the
per-draw figure and none is claimed): ≈ 0.2–0.5 µs CPU per `vkCmdDrawIndexed` ⇒ 1 024 instances
≈ 0.2–0.5 ms; the v1 budget is therefore **≤ 1 024 translucent instances per frame** as a
*placeholder line* (a diagnostic counter, not a silent clamp — G9) that a measured per-draw cost
on this box replaces, and the GPU-sorted multi-draw-indirect route is the rung that lifts it (D6).

### 1.4 Per-instance bits reach the device where they already have a home

- Masked instances stay in the **opaque** gather, keyed `bucket = (mesh_id << 1) | masked` — the
  count/prefix-sum/scatter core is untouched and the batch count at most doubles; `DrawBatch`
  gains the low bit so each recorder binds the masked pipeline per batch. No new lane.
- `VbInstanceRow.flags` bit 1 = `VB_INST_FLAG_MASKED` (`instance_model.rs:238-248`: "a word rather
  than a bool so piece 3 adds a BIT, not a column"). The batch cull reads nothing new; the late
  raster binds the masked pipeline for masked batches exactly as the early one does.
- Translucent instances never reach `VbInstanceRow` at all; their instance ring is the
  `InstanceModelCol` 48-B row (`instance_model.rs:1-12`) plus the existing
  `PerInstanceMaterialTex` 48-B payload (`mesh_draw.rs:155-180`), which already carries the five
  bindless slots the forward FS needs.

### 1.5 The new images (appended last, `FRAMEGRAPH_IMAGE_COUNT` discipline)

| Rung | Image | Format | Bytes/px | 1080p | 1440p | Ringed |
|---|---|---|---|---|---|---|
| 1 | — (writes `lit`, reads the path's depth) | — | 0 | 0 | 0 | — |
| 6 | `taa_cov` | `R8_UNORM` | 1 | 2.1 MB | 3.7 MB | per-FIF (the `ssao` shape) |
| 4 | `scene_color` (mip chain, `add_image_mipped`) | `R8G8B8A8_UNORM` (or `B10G11R11` after ballot 1) | 4 × 4/3 | 11.1 MB | 19.7 MB | single, seeded (the HZB shape, `graph.rs:365-380`) |
| 7 | `oit_b0` | `R32_SFLOAT` **if the boot probe says it blends** (TK-13), else `R16_SFLOAT` | 4 (2) | 8.3 (4.1) MB | 14.7 (7.4) MB | single |
| 7 | `oit_moments` | `R16G16B16A16_SFLOAT` | 8 | 16.6 MB | 29.5 MB | single |
| 7 | `oit_accum` | `R16G16B16A16_SFLOAT` | 8 | 16.6 MB | 29.5 MB | single |
| 9 | `csm_trans` (per cascade) | `R8_UNORM` grey (`R8G8B8A8` colour) | 1 (4) per cascade texel | 4.2 MB per 2048² cascade (16.8 colour) | — | as `csm` |

All `[D-arith]`; 1080p = 2 073 600 px, 1440p = 3 686 400 px. Rung 7's full set is **41.5 MB** at
1080p (37.3 with a 16-bit `b0`), in the published MBOIT-4 fp16 range (~20 MB moments + the
accumulator the measurement did not count, RESEARCH §4.3); the half-resolution moment variant is
22.8 MB and is the *second* knob, after precision (RESEARCH P29). Every image is allocated at boot
by the boot-frozen consumer set (§2.4) — zero bytes when the rung is disarmed, never lazily
(RESEARCH P13).

**Summed against the owner's box.** The GPU oracle is an RTX 3060 Laptop with 6 GB
(`docs/OPTIMIZATION-PLAN-RENDER.md:44`, `docs/LIGHTING-PLAN.md:39`). At 1440p the top of the
ladder holds R7's 14.7 + 29.5 + 29.5 = 73.7 MB, `scene_color` 19.7 MB and `taa_cov` 3.7 MB ×
`FRAMES_IN_FLIGHT = 2` (`present/mod.rs:92`) = 7.4 MB — **≈ 101 MB ≈ 1.6 % of 6 GB** [D-arith].
Memory decides no rung here; what decides R7 is the fit table (all-additive blend, RESEARCH §4.2)
and the in-engine GPU zone, not the published millisecond (D11, §14).

`oit_b0`'s format is probed, not assumed: whether `VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT`
is in the mandatory set for `R32_SFLOAT` is **UNVERIFIED** (three fetches of the spec's formats
chapter — two by the critique, one by this pass — returned without the required-support tables;
§14). The engine already probes optional format features at boot (`device.rs:3264-3484`, the
`optimal_tiling_features & VK_FORMAT_FEATURE_*` idiom; `enums.rs:362-364` records which formats
needed one), so TK-13 adds `VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT = 0x100` to `ffi.rs`
beside `COLOR_ATTACHMENT_BIT = 0x80` (`ffi.rs:1154`; the value from `vulkan_core.h:2676` of the
installed SDK 1.4.350.0 [D]) and probes `R32_SFLOAT` for it; the fallback is `R16_SFLOAT` for
`b0`, whose blend support is what the moments already rely on.

## 2. The seam — where transparents leave and rejoin, per path

### 2.1 Exit: the gather

```
gather_mesh_draws      Query<(MeshHandle, &InstanceModelCol, Option<&GpuTransform3D>, MaterialRef,
                              Option<&OcclusionCulling>, Option<&AlphaMasked>), Without<Translucent>>
gather_translucents    Query<(… same …), With<Translucent>>          → TranslucentRenderScratch
gather_shadow_casters  Query<…, (With<ShadowCaster>, Without<Translucent>)>   // rung 1: glass casts nothing
```

The `Without<Translucent>` term is the whole exit: no VB batch, no `vb_id`, no HZB occluder, no
classify bin, no `vb_geo` thin-aux, no caster — and, under `hwrt`, **no TLAS instance**: the
`tlas_pack` pass consumes the same gather ring and its parallel `mesh_ids` lane
(`mesh_draw.rs:36-50` — "instance `i` maps to (`ring[i]`, `mesh_ids[i]` = its BLAS)";
`graph_bridge.rs:1665-1672`), so a translucent that never enters the ring is never a ray-traced
shadow caster either. The participation matrix (§7) follows from this one filter. The translucent gather applies the same frustum verdict the opaque gather applies and
nothing more in v1 (occlusion against the HZB for translucents is D14).

### 2.2 Rejoin: one pass, three declarators, the D7 table verbatim

The pass is `translucent_draw`, declared immediately **before** `particle_draw` at the slot each
declarator already names:

| Path | declare at | `lit` | depth | compare | derived at the slot | early-Z |
|---|---|---|---|---|---|---|
| Deferred | `graph_bridge.rs:2447-2462` | ResId 5, `COLOR_ATTACHMENT` R\|W | `depth` (3), read-only attachment | `LESS`, `-D DEPTH_LINEAR` VS+FS pair (`particle_draw.fs.hlsl:22-42`) | `SRO → DEPTH_ATTACHMENT_OPTIMAL` transition at R1; **from R3** the same depth is also sampled through the push descriptor, so the slot's layout becomes `DEPTH_STENCIL_READ_ONLY_OPTIMAL` (TK-2) and the access gains `FRAGMENT_SHADER \| SHADER_READ` | **off** (writes `SV_Depth`); the **64-unit horizon** applies (`gbuffer_mrt.fs.hlsl:97-114`) |
| Forward | `:2983-2991` | ResId 0 | `forward_depth` (1) | `GREATER` | availability barrier | on |
| ForwardPlus | same | ResId 0 | `forward_depth` (1) | `GREATER` | nothing — free | on |
| VisibilityBuffer | `:6170-6185` | ResId 0 | `vb_depth` (2) | `GREATER` | availability barrier (or a transition under an SDF leg) | on |

**Forward and ForwardPlus have no TAA and no pre-light consumers**: `cap_forward_v1_consumers`
(`render_path_config.rs:1351-1381`, applied to `Forward | ForwardPlus`) forces `taa_on`, `ddgi_on`,
`ssao_on` and `ssr_on` to `false` with a logged degrade. On those two paths the seam's downstream
is `present_sample` directly — §2.3's `taa_resolve` and R6's `taa_cov` exist on Deferred and VB
only, and R1's DDGI bindings are bound-but-unread there (§2.4).

The recorder is `passes/translucent.rs`, path-agnostic like `passes/particles.rs:53-66`
(`color_view = lit`, `depth_view = the path's depth`, `depth_write = false`); the compare op and the
SPIR-V pair are chosen once at boot from the same `deferred_path` predicate the particle bundle
uses (`PARTICLES-PLAN.md:1517` — `particle_depth_compare_for` + `particle_draw_spirv_for`).
Declaration order is record order ("declaration order is execution order",
`graph_bridge.rs:1651`; RESEARCH P24): the insert lands in `declare_deferred_graph` (`:1288`),
`declare_forward_graph` (`:2585`), `declare_vb_graph` (`:4074`) and the three recorders, with the
`particle_barrier_stream`-style pin (G6) proving the stream moved by exactly the new pass.

### 2.3 Frame order at the seam (every path; brackets = rung-gated)

```
… last opaque lit producer (resolve | forward_opaque | vb_shade/vb_resolve) → [sdf_forward_march]
→ [R4] scene_color_copy (lit → mip 0) → scene_color_mips[1..k]            // compute average chain
→ [R7 armed]  translucent_draw (-D REFRACT, SORTED: transmissive meshes ONLY — never a moment fragment, R5)
              → oit_moments (premultiplied meshes + alpha particles) → oit_accum (same) → oit_composite (lit)
              → additive_draw (additive meshes) → particle_draw (additive)   // unsorted, AFTER the composite
   [R7 off]   translucent_draw (sorted premultiplied meshes, transmissive included) → particle_draw (alpha)
              → additive_draw (additive meshes) → particle_draw (additive)
→ [R6] the blended draws also write taa_cov                                // Deferred + VB only
→ [Deferred, VB]          taa_resolve → present_sample → post chain (FXAA/SMAA/RCAS/SSAA) → present_blit (+ UI rects)
  [Forward, ForwardPlus]  present_sample → post chain → present_blit          // no TAA on these paths (§2.2)
```

Three fixed facts: transparents are **inside the TAA loop** where a TAA loop exists (the tree's
choice for particles, `graph_bridge.rs:2447-2449`, and the survey's: after-TAA jitters against a
jittered depth, RESEARCH §8); the additive class — particles *and*, from R1, additive meshes —
stays additive and unsorted under every arm and draws **last**, after `oit_composite` when R7 is
armed, so its contribution stays the pure sum `dst + Σ` the particle recorder's own argument rests
on (`passes/particles.rs:772-778`: putting an attenuating draw after it would make the additive
pin's order-independence conditional) — its proof (`enums.rs:935-948`) is untouched by anything
below `lit` going HDR (ballot 1); and transmissive surfaces draw **before** the moment passes
under R7 (R5 says why: a fragment with `a = 1` has infinite absorbance), which is HDRP's own order
— refractive draws, then the rest (RESEARCH §2) — so alpha *in front of* glass composites
correctly and alpha *behind* glass is the ballot-7 loss R4 already states.

### 2.4 Boot-frozen arming — a consumer arms its producer

`RenderPathConsumers` gains three bits, resolved once at `WindowHost::boot` like every other
(`render_path_config.rs:1-30`): `translucent_wanted`, `oit_mode: OitMode::{Off, Mboit}`,
`translucent_shadows: {None, Grey, Colour}`. The rules, stated so the W4 hole class
(`render_path_config.rs:96-118`) cannot re-open:

- **The translucent FS never arms the cull; it reads the gate.** The first pass wrote
  `translucent_wanted ⇒ froxel_light_cull` on every path, and that rule contradicts the tree:
  `froxel_light_cull = consumers.clusters_wanted && matches!(path, RenderPath::VisibilityBuffer)`
  (`render_path_config.rs:1224`), pinned VB-ONLY by `froxel_light_cull_is_vb_only`
  (`:3385-3406`, "even with clusters_wanted"); on the other three paths `sync_cluster_light_gate`
  writes `(0.0, 0.0, 0)` into the header's dims lane (`light.rs:1011-1015`), the `light_cull` pass
  is declared only under ForwardPlus with the cull built (`graph_bridge.rs:2745-2750` — "meaningless
  (and unusable) under plain `Forward`"), and `ClusterGrid`/`LightIndexList` are bound to the
  light table as a placeholder (`targets.rs:208`; `deferred_pbr.hlsl:1246-1249`: "`ClusterGrid` is
  ALWAYS the light-table placeholder here"). `clusters_enabled` is moreover a **per-frame header
  word** (`light_table.hlsli:341`), flipped per frame under `ClusterSelectMode::Auto`
  (`light_policy.rs:190-193`) — not boot-frozen state a boot rule can imply. A froxel-only FS
  would therefore have read the placeholder on three paths: exactly the W4 "consumer without
  producer" hole D8 claimed to close. **The fix is the tree's own**: the FS carries the three-term
  runtime gate every lit producer carries — `use_clusters = clusters_enabled ≠ 0 ∧ dim_x·dim_y·dim_z
  ≠ 0 ∧ cluster_count ≤ GetDimensions(ClusterGrid)`, else the flat `[l0a_count, light_count)` loop
  — byte-for-byte the `forward_opaque_froxel` body (`forward_opaque.fs.hlsl:368-390`; the same
  three terms at `deferred_pbr.hlsl:1264`, `vb_resolve.comp.hlsl:423`, `vb_shade.comp.hlsl:578`).
  `translucent_wanted` implies **nothing** about the cull: under VB with `clusters_wanted` the
  froxel arm is taken, everywhere else the flat arm — which is the ALL-LIGHTS body those paths
  already shade opaque geometry with. Arming L1 on the other three paths was priced and not taken:
  it is a config, header, test and declarator change plus a `_froxel` pipeline family per path
  (`gpu_scene/mod.rs:5369-5380` builds "the ENTIRE froxel light-cull machinery" for VB alone), for
  a pass whose flat loop is already correct there. G11 is restated to gate the fallback, not an
  arming.
- **Pre-light consumers follow the bound-but-unread discipline, explicitly.** R1 binds the DDGI
  atlas at @8/9/10 and `shadow_apply`'s Set 1 on every path; under Forward/ForwardPlus the resolver
  forces `ddgi_on = false` (`render_path_config.rs:1351-1381`), and the probe sample runs only under
  the `ddgi_mode != 0` header-word gate (`vb_shade_split.comp.hlsl:38-41`; the descriptor "MUST be
  valid even on the OFF path … bound-but-unread", `scene_types.rs:2568-2577`). No arming rule is
  implied for DDGI, SSAO, SSR or TAA by this design; each stays the resolver's.
- `oit_mode != Off ⇒ translucent_wanted` and ⇒ `ParticleSortMode::None` for the alpha class
  (moments accumulate in any order) — which, by R10 (`gpu_scene/particle.rs:400-412`), makes
  particle motion vectors *eligible* again; a rung-7 side effect worth its own line.
- `translucent_shadows != None ⇒ translucent_wanted ∧ csm armed`.

## 3. The ladder

Each rung: what it adds, its prerequisites **in this tree** (file:line), its cost (labelled), and
the fork it closes. Rungs are independently green; none re-blesses the default-path goldens except
where stated.

### R1 — Sorted alpha, premultiplied, per-instance sort-key column

- **Adds**: `Translucent` marker + `Material::route()` (§1.1); `TranslucentRenderScratch` (§1.3);
  `translucent_draw` in three declarators/recorders (§2.2); `forward_translucent.{vs,fs}.hlsl`
  (§9); `Without<Translucent>` on the opaque and caster gathers; the boot bit
  `translucent_wanted` (§2.4 — it arms **no** producer); the `two_sided_sorted` flag = two draws
  per run with `CullMode::Front` then `Back` (both exist on the desc, `descriptor.rs:401-403`) —
  the convex-glass mitigation of RESEARCH §3 at 2× draws and 0 B; the **additive mesh class**
  (`BLEND_CLASS = 1`, §1.1): a second pipeline from the same desc closure with
  `BlendState::ADDITIVE` — the particle bundle's own shape (`gpu_scene/particle.rs:659-698`,
  "ONE descriptor, TWO pipelines: the blend is the only field that differs") — drawn unsorted
  after the sorted set (§2.3) under the existing commutativity proof (`enums.rs:935-948`);
  holograms, energy shields, glow at 0 B and 0 RHI growth. `Multiply` waits on TK-1 (R9).
- **Prerequisites**: `BlendState::PREMULTIPLIED_ALPHA` (`enums.rs:913`) and `ADDITIVE` (`:949`)
  ✔; the seam (`graph_bridge.rs:2447`, `:2983`, `:6170`) ✔; D7's compare table
  (`PARTICLES-PLAN.md:461-476`) ✔ and `create_graphics_pipeline_particle`'s parameterised builder
  (`rhi_impl/device.rs` — the F3b precedent) ✔; the froxel FS body **with its three-term
  `use_clusters` gate and flat fallback** (`forward_opaque.fs.hlsl:308-390` — the bindings at
  `:118-128` are the declaration, the gate is the body) ✔; the light-table placeholder for
  `ClusterGrid`/`LightIndexList` where no cull is built (`targets.rs:208`) ✔; `shadow_apply.hlsli`
  ✔; `ddgi_probe_sample` (`ddgi_resolve.hlsli:107`) under its `ddgi_mode` gate ✔; the
  `-D DEPTH_LINEAR` VS+FS idiom (`PARTICLES-PLAN.md:1509`, `:1515`) ✔. **Nothing new in the RHI.**
- **Bake contract** (§5): the albedo of a `Blend` material is **premultiplied before** the T2
  `vkCmdBlitImage` mip chain (`device.rs:635-637`), or its mips fringe (RESEARCH P9). The FS
  outputs `(rgb·a, a)`; the STRAIGHT state is never used for meshes (particles keep it,
  `gpu_scene/particle.rs:689-693`, because their source is straight).
- **Cost** (estimate): GPU ≈ `D × C_fs` where `D` = translucent depth complexity per pixel and
  `C_fs` = the froxel forward FS's per-fragment cost (a light loop + the 13-tap PCF disc per
  shadowed light, `shadow_apply.hlsli:8` + one probe sample); at `D = 2` over 1080p that is 4.1 M
  fragment shades. This is UE's "Surface ForwardShading" tier — "the most expensive translucency
  lighting method" (RESEARCH §7) — and it is the **only** tier in v1; smoke-class content with
  `D ≫ 2` pays it per fragment. A cheap tier (per-vertex lighting, or UE's translucency-volume
  shape) is R1c below, not v1. Unmeasured — the instrument is the GPU zone the recorder registers
  (the particles' gate #17 precedent). CPU: §1.3. Memory: 0.
- **Closes**: the "where do transparents go" question for every path; the VB path gains nothing
  and loses nothing (G1).

### R2 — Hashed / dithered cutout through the VB (and every other path)

- **Adds**: `AlphaMasked` marker; the `(mesh_id << 1) | masked` bucket key (§1.4); `-D MASKED`
  variants of `vb_raster.{vs,fs}`, `gbuffer_mrt.{vs,fs}`, `forward_opaque.{vs,fs}` and the CSM /
  atlas caster pipelines (foliage must shadow); the VS exports `uv` and object-space position; the
  FS samples the bindless albedo's alpha × `base_color.w` and `discard`s below the threshold;
  threshold = `cutoff` (plain) or the **hashed** leaf (§9, Wyman & McGuire) under
  `MASK_HASHED`, whose noise TAA integrates (RESEARCH §4.1 item 8).
- **Prerequisites**: the VB VS today exports only `IID` (`vb_raster.fs.hlsl:19-22`) **and
  consumes a position-only vertex stream** — `VsIn` is `position : POSITION` plus `color`/`normal`
  declared "for `VertexAttribute` parity but unread by this position-only pass"
  (`vb_raster.vs.hlsl:169-177`), and its Set 0 holds only `instances` @0 and `visible_instances`
  @11 (`:158`, `:167`); UVs live in the geometry table the **compute** fetch reads
  (`vb_geom_fetch.hlsli`'s `gMeshVerts[]`/`gMeshIndices[]`/`gMeshMeta` at `vb_shade.comp.hlsl:68`,
  sampled at `geo.uv` `:359-412`). The masked VB VS therefore needs a UV source it does not have:
  **a geometry-table pull by `SV_VertexID` from the same `gMeshVerts[]` SSBO, bound to the VS** —
  a Set-0 binding growth on the vertex side (no second vertex-stream binding, no `VertexAttribute`
  change), which is an interface change on **both** stages and one manifest row for the pair, not
  an FS export alone. `PerInstanceMaterialTex` already carries the albedo slot
  (`mesh_draw.rs:155-180`). Under Deferred the masked FS already writes `SV_Depth`,
  so nothing is lost; under the reverse-Z paths `discard` keeps the early depth **test** and defers
  only the write — the conservative-depth contract, not a full early-Z loss.
- **Cost**: one bindless sample + ~20 ALU (hash) per masked fragment; **0 B**; pipelines: +1
  raster variant per path × {early, late} under VB, +1 caster variant. TAA: hashed cutout is
  *correct in expectation* only under accumulation (P-TAA); with TAA off it is stable noise, which
  is the published trade.
- **Closes**: foliage / hair / chain-link / LOD dissolve stay Nanite-class geometry; the first
  reader of `base_color.w` (RESEARCH P30).

### R3 — Sampled depth in the transparent FS (D13, shared with `-D SOFT`)

- **Adds**: `VK_KHR_push_descriptor` enabled by the `supports_*` idiom
  (`crates/boyko_rhi_vulkan/src/device.rs:2816-2969`), the read-only depth layout
  (`VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL`) and the `SampledDepthAtReadOnly` bind-group
  entry beside `SampledImageAtGeneral` (`crates/boyko_rhi/src/device.rs:352-401`); the depth
  access gains `FRAGMENT_SHADER | SHADER_READ` — exactly `PARTICLES-PLAN.md:1509`'s list, taken
  once for both consumers.
- **Prerequisites**: D13's decision (`PARTICLES-PLAN.md:1888-1902`) stands; this rung lands it.
- **Cost**: one depth sample per fragment where used; 0 B.
- **Closes**: soft particles (`-D SOFT`), depth-fade on meshes (water edges), and the refraction
  depth test of R5 (a refracted sample **in front of** the surface is rejected — Godot's documented
  leak, RESEARCH §6).

### R4 — The opaque colour mip chain

- **Adds**: `scene_color` via `add_image_mipped` with a cross-frame seed (`graph.rs:365-380` —
  required, not optional); pass `scene_color_copy` (a `vkCmdBlitImage` or a compute copy of `lit`
  into mip 0, after the last opaque producer and **before** `translucent_draw`); passes
  `scene_color_mips[k]` — the `hzb_build` shape (`graph_bridge.rs:3447-3455`) with a 2×2 average
  instead of a min; `k = floor(log2(min(w,h)))` levels; a bilinear sampler.
- **Prerequisites**: the mipped-resource API ✔; the blit fn ✔; the HZB's per-level storage views
  (`targets.rs:1440-1447`) as the template.
- **Cost** (estimate): the chain is `4/3 × 8.3 MB = 11.1 MB` at 1080p. Traffic, counted
  honestly [D-arith]: `scene_color_copy` reads 8.3 MB and writes 8.3 MB (2 073 600 × 4 B per
  direction); the downsample reads every level once (`Σ S/4^k = 4/3 S` = 11.1 MB) and writes every
  level above mip 0 (`S/3` = 2.8 MB) — **≈ 30 MB per frame**, not the 11 MB the first pass wrote.
  At an effective bandwidth of ~100–200 GB/s — an **uncited estimate** for an RTX-3060-class part,
  no measurement behind it — that is **≈ 0.15–0.3 ms**; the GPU zone measures it. Doom Eternal
  uses this chain in place of a half-res transparency tier (RESEARCH §2) — the same choice here
  (D10).
- **Which scene colour** (RESEARCH P12): the chain holds **opaque + sky only**. Translucents
  behind a refractive surface are not refracted in v1 — HDRP's pre-refraction sub-pass and depth
  copy are ballot 7, priced there.
- **Closes**: refraction, distortion, frosted glass — one texture, `LOD = f(roughness, ior)`.

### R5 — Thin transmission (`KHR_materials_transmission` + `ior` + `specular`)

- **Adds**: `MaterialXGpu` (§1.2) bound at the translucent FS only; the `-D REFRACT` variant of
  `forward_translucent.fs` (a binding change ⇒ a manifest row); the leaves `fresnel_f0(ior)`,
  `transmission_split(E, diffuse, transmission)` and `refraction_lod(roughness, ior, mips)` (§9);
  the blend semantics: a transmissive fragment **replaces** the background it sampled, so the
  premultiplied output carries `a = base_color.a` (1.0 for pure transmission) and the transmitted
  radiance rides in `rgb`.
- **Under OIT a transmissive fragment is never a moment fragment.** "Replace the background" is
  a sorted-pass semantic: MBOIT's stage 1 accumulates absorbance `−ln(1 − a)` (RESEARCH §4.1 item
  4, "works on log-transmittance (absorbance) … b0 = Σ absorbance"), which at `a = 1.0` is `+∞`,
  so `b0 = ∞`, `exp(−b0) = 0`, and `lit·exp(−b0) + accum` blacks every pixel behind a transmissive
  surface [D-arith]. Clamping `a` and re-deriving the refracted sample as transmittance × background
  would be a different shader body with a different meaning; this design does not write it.
  Instead, when `OitMode::Mboit` is armed, instances with `transmission > 0` draw **sorted**, with
  `-D REFRACT` and `OIT_STAGE = 0` only, **before** the moment passes (§2.3) — so `REFRACT ×
  OIT_STAGE ∈ {1, 2}` are not variants (§4), R7 does not close layered *refractive* glass (its
  "Closes" line is amended), and the participation matrix (§7) says "sorted, before the moments"
  in the OIT column of the refraction row.
- **Prerequisites**: R1, R4, R3 (the depth reject); the extension table's SSBO staging ring
  (the `MaterialTable` discipline, `material_table.rs:1-30`).
- **Cost**: one chain sample + the BTDF terms per transmissive fragment; **+3.1 MB** VRAM ceiling
  for the table.
- **Closes**: glTF-conformant glass without thickness, manifoldness or a shape model (RESEARCH
  P14/P15 avoided entirely) — the owner decides whether v1 stops here (ballot 3).

### R6 — TAA coverage mask ("responsive" transparents)

- **Adds**: `taa_cov` (§1.5) as a **second colour attachment** of `translucent_draw` and
  `particle_draw`; the FS writes `SV_Target1 = (a, a, a, a)`; the **same replicated blend state**
  yields `cov' = a + cov·(1−a) = 1 − Π(1−a_i)` under `PREMULTIPLIED_ALPHA`, and `sat(Σ a)` under
  `ADDITIVE` — both are a coverage, and neither needs `independentBlend` (that is the number: the
  mask costs **0 new RHI state**). `taa_resolve` reads it and shrinks the feedback:
  `k *= 1 − r · cov`, with `r = 0.75` an **uncited starting value** (no source; ballot 12) that
  the motion fixture tunes (G8). Deferred and VB only — Forward/ForwardPlus have no TAA (§2.2).
- **Prerequisites**: the particle pipelines' `color_formats: &[RASTER_COLOR_FORMAT]`
  (`gpu_scene/particle.rs:667`) widen to two formats when armed — a second boot-frozen pipeline
  pair, the D10 amendment's shape (`PARTICLES-PLAN.md:537`); `taa_resolve.comp.hlsl` gains one
  binding (a manifest row); `TAA-PLAN.md`'s open item 8 is touched only by ballot 1.
- **Cost**: 2.1 MB per FIF; one extra 8-bit write per translucent fragment; one read per pixel in
  the resolve.
- **Closes**: the ghost trail behind a moving translucent over a static background (the
  "wrong only in motion" class) without motion vectors, which R10 forbids for sorted sets and the
  tree does not wire for meshes (`taa_config.rs:264-270`).

### R7 — Order-independent transparency: **MBOIT**, chosen with numbers

The fit table (RESEARCH §4.2) leaves one candidate the RHI at `ed0bed45` can build:

| Candidate | Blocked by | Published cost @1080p [B] | Memory |
|---|---|---|---|
| WBOIT | `independentBlend` off (`device.rs:3798-3801`) + one blend state for all MRTs (`rhi_impl/device.rs:1902-1917`); revealage needs `(ZERO, 1−SRC_A)` on RT1 while RT0 is `(ONE, ONE)` | not in the set | 9–20 B/px |
| MLAB / AOIT | `VK_EXT_fragment_shader_interlock` not enabled or queried (zero hits under `crates/`) | 4.4 ms (k=2) | 35 MB |
| PPLL | FS atomics + unbounded allocation (Principle 5); early-Z off; the resolve's divergence | 5.5 ms | 200 MB |
| Depth peeling | N/2+1 geometry passes; `BlendOp::{Add}` only (`enums.rs:862-867`) | — | — |
| Stochastic | `SAMPLE_COUNT_1`, `alpha_to_coverage = FALSE` (`rhi_impl/device.rs:1849-1853`) | — | 0 |
| **MBOIT-4 fp16** | **nothing** — every target blends `ADDITIVE`, which `Option<BlendState>` replicates onto all MRTs by construction | **3.5 ms** (vs 1.37 sorted) | **~20 MB** (+ accumulator) |

- **Adds**: `oit_b0`, `oit_moments`, `oit_accum` (§1.5); `-D OIT_STAGE=1` (moments) and `=2`
  (accumulate) variants of `forward_translucent.fs` (**`REFRACT = 0` only** — R5 says why) **and**
  `particle_draw.fs` — one FS serves both particle classes (`emit_particles.rs:10-12`; both
  pipelines take `&draw_fs`, `gpu_scene/particle.rs:659-663`), so the alpha class binds the
  stage-1/stage-2 pipelines and the additive class keeps binding the `OIT_STAGE = 0` one, drawn
  after the composite (§2.3); `oit_composite` compute: `lit = lit · exp(−b0) + accum`; the
  moment warp is the **same log-depth leaf as the sort key** (`u ∈ [0,1]`, warped to `[−1,1]`, the
  fixed 15-octave range of §1.3), so one oracle covers both; the reconstruction leaf (power-moment
  transmittance bound with the paper's bias) has an `f32` oracle pinned against a brute-force
  per-pixel sort on a fixture (G10).
- **Why two passes is a feature here**: the two-bucket compromise the particle recorder concedes
  (`passes/particles.rs:772-778`) disappears — meshes and alpha particles accumulate moments in
  any order across **different pipelines**, which no permutation sort can do (D11). And
  `ParticleSortMode::None` becomes legal again for the alpha class, re-enabling motion vectors
  under R10.
- **The overflow observable** (RESEARCH P5): MBOIT does not overflow, it *biases*. The
  mechanical error term is energy conservation: an exact reconstruction gives
  `accum.a = Σ a_i·T(z_i) = 1 − exp(−b0)`; the composite writes
  `max_px |accum.a − (1 − exp(−b0))|` by atomic max into a 4-byte readback the gate reads. A
  scene above the paper's bias envelope is red, not quietly wrong.
- **Prerequisites**: R1; three images; the 16-bit targets are **`SFLOAT`** (`R16_SFLOAT` for
  `b0`, `R16G16B16A16_SFLOAT` for the moments) — a precision reduction the hardware adder performs
  unchanged, which is the configuration the published 3.5 ms measures (precision before
  resolution, RESEARCH P29 — 8.1 → 3.5 ms). The paper's *non-linearly quantised* UNORM16 moments
  are **not** assumed: their compatibility with a `(ONE, ONE, ADD)` pipeline is unverified
  (RESEARCH §4.1 item 4, §14 here), and G10's precision fixture decides SFLOAT16 vs `R32` per
  target rather than a paper claim; `b0`'s own format is the TK-13 probe's answer (§1.5). Boot arm
  `OitMode::Mboit` (§2.4).
- **Cost**: two geometry passes over the translucent set (`2 × D × C_fs'` with `C_fs'` the
  lighting cost only in stage 2) writing 12 B/px (stage 1: `b0` + moments) and 8 B/px (stage 2)
  blended per fragment × depth complexity, + one fullscreen composite; 41.5 MB at 1080p full-res,
  73.7 MB at 1440p, 22.8 MB half-res moments (§1.5). **The published 3.5 ms does not decide this
  rung**: it is one 1080p scene on an RTX 3080 laptop (RESEARCH §4.3 [B]); the owner's 1440p on a
  3060 laptop is 1.78× the pixels on a slower part, and the cost scales with depth complexity ×
  resolution by construction. What decides R7 is the fit table — every target blends `ADDITIVE`,
  the one OIT family this RHI expresses (RESEARCH §4.2) — and the in-engine GPU zone before it is
  armed by default (RESEARCH P28). D11 is amended to say so.
- **Closes**: smoke / hair / layered **non-refractive** alpha correctness across meshes and
  particles; the sort budget of §1.3 for that set (no sort under OIT). It does **not** close
  layered refractive glass: transmissive surfaces stay a sorted pass in front of the moments (R5,
  §2.3), and transparents behind them are ballot 7.

### R8 — Volume, Beer-Lambert, dispersion

- **Adds**: `thickness`, `attenuation_*` and `dispersion` lanes become live; the leaves
  `beer_lambert(c, d, x) = c^(x/d)` (one body, oracle-tested against the three published
  spellings, RESEARCH §5) and `refract_offset(n, v, ior, thickness)` with the parallel-exit
  approximation (Filament's `ray.direction = r`); dispersion = three chain samples at
  `ior ± halfSpread`, `halfSpread = (ior − 1)·0.025·dispersion`, **not with thin** (Filament [D]).
- **Prerequisites**: R5; a bake-time **manifoldness check** for `thickness ≠ 0` (glTF requires a
  closed mesh; the loader makes no claim — RESEARCH §13) — refusal vs warn is ballot 4.
- **Cost**: dispersion = 3× the chain sample; the shape model is **box/thin only** (Unity's sphere
  turns the scene upside-down, RESEARCH P14).

### R9 — Translucent shadows (transmittance lanes)

- **Adds**: `csm_trans` per cascade (and per atlas tile for punctual lights), cleared to 1.0,
  drawn after `csm_depth` by the translucent casters (`With<ShadowCaster>, With<Translucent>`)
  depth-tested read-only against the cascade depth, `depth_write = false`, blend
  **`(ZERO, ONE_MINUS_SRC_ALPHA)`** ⇒ `T' = T·(1 − a)` — **both factors already exist**
  (`enums.rs:838-847`), so grey transmittance costs 0 RHI growth. Colour transmittance needs
  `dst = ONE_MINUS_SRC_COLOR` with `src = a·(1 − tint)` ⇒ the `BlendFactor` family grows by
  **`{SrcColor = 2, OneMinusSrcColor = 3, DstColor = 4, OneMinusDstColor = 5}`** — the
  `VkBlendFactor` values (`vulkan_core.h:2401-2412` of the installed SDK 1.4.350.0 [D]; the
  critique's spec fetch agrees [D docs.vulkan.org framebuffer chapter, opened 2026-09-10]). The
  first pass wrote `{4, 5, 8, 9}`, which are `DST_COLOR`, `ONE_MINUS_DST_COLOR`, `DST_ALPHA`,
  `ONE_MINUS_DST_ALPHA`: because the backend lowers by `as_i32()` with no table
  (`enums.rs:849-855`; `rhi_impl/device.rs:1886-1897` "each lowering is an `as_i32()` no-op") that
  would have bound `OneMinusSrcColor` as `ONE_MINUS_DST_COLOR` silently — a wrong blend, not an
  error — and `abi_guard.rs:378-385` asserts only the four existing variants, so nothing would
  have caught it before a golden. TK-1 therefore carries the four `ffi.rs` constants
  (`:799-802` defines only the existing four) and the four `const _: () = assert!` guards as part
  of its scope, not as a follow-up. The same growth gives meshes the `Multiply` class
  (`(DstColor, Zero)`, §1.1). `shadow_apply.hlsli` multiplies one bilinear `csm_trans` fetch into
  `csm_visibility`.
- **Order-independence, stated honestly**: `Π(1 − a_i)` is commutative in ℝ; under 8-bit
  quantisation `q(q(x·a)·b)` and `q(q(x·b)·a)` differ by ≤ 1 LSB — the casters ship unsorted with
  a **1-LSB** tolerance, unlike the additive proof, which is exact.
- **Prerequisites**: R1; the caster gather's filter split (§2.1); SDF shadows (`sdf_mesh_shadow`,
  the soft march) remain **opaque-only** — no published precedent for a transmissive SDF occluder
  (RESEARCH §7), and none is designed here.
- **Cost**: 4.2 MB per 2048² cascade grey (16.8 colour); one extra fetch per shadowed fragment.

### R10 — Reflections on transparents

- **Today**: the translucent FS uses the opaque BRDF's own ambient/specular term — analytic sky +
  `EnvBRDFApprox` (`docs/PBR-MATERIALS-PLAN.md` Decision 6) — so glass reflects exactly what an
  opaque surface reflects. **No SSR pass exists** (only the consumer bit, `FEATURE_MAP.md:125`).
- **When SSR lands**: front-layer only, matched by a depth threshold (UE Lumen's shipped
  compromise, RESEARCH §7) — the front layer is the first `translucent_draw` fragment to pass the
  depth test, which R3's sampled depth identifies.
- **Cost**: deferred to the SSR campaign; ballot 8.

### R11 — HDR scene colour (the coupled decision)

Nothing above needs it for the **seam**; R5/R8 need it for the **physics** to mean what they say
(Beer-Lambert and Fresnel in display-referred 8-bit are approximations, RESEARCH P10). The move
is one decision with four costs: `lit` → `B10G11R11_UFLOAT` or `R16G16B16A16_SFLOAT`
(`enums.rs:364-376`, both exist), tonemap + OETF move from `deferred_pbr.hlsl:1395-1409` (and the
forward/VB producers) to the present tail, TAA moves pre-tonemap with luma weighting
(`TAA-PLAN.md:315`), the additive proof at `enums.rs:938-944` is rewritten for float saturation,
and **every golden re-blesses** (30 pins, `FEATURE_MAP.md:1059`). That is a scope event — ballot 1.

### Rungs the critique added (each a separate landing, none in v1)

The first pass shipped one mesh blend state and one lighting tier; every surveyed engine ships
more, and the gaps are named here with their prerequisites rather than left implicit:

- **R1c — a cheap lighting tier for translucents.** Per-vertex lighting in the translucent VS
  (the same froxel/flat loop, evaluated at vertices and interpolated), selected per material; UE's
  translucency-volume shape is the second candidate (RESEARCH §7: volume vs "Surface
  ForwardShading — the most expensive"). Prerequisite: R1's GPU zone shows `D × C_fs` is the
  cost for the owner's content. Ballot 15.
- **R3b — transparent depth prepass / postpass per material** (HDRP "Transparent depth prepass",
  "Transparent PostPass" [D docs.unity3d.com HDRP rendering-execution-order, opened by the
  critique]; Godot "Depth Pre-Pass"): the front layer writes depth first, fixing intra-object order
  for **concave** meshes beyond the convex two-sided trick, and a postpass writes depth for
  post-effects. Cost: one extra draw per opted-in instance, and it breaks the draw-run merge for
  that instance (RESEARCH §3). Prerequisite: R3 (the read-only depth layout), since the prepass
  writes the depth the main draw then reads. Ballot 14.
- **R5b — diffuse transmission** (glTF `KHR_materials_diffuse_transmission`; Bevy
  `StandardMaterial::diffuse_transmission` [D docs.rs/bevy/latest/bevy/pbr/struct.StandardMaterial.html,
  opened by the critique]): leaves, paper, lampshades — a wrap-lit diffuse lobe from the back
  hemisphere, no chain sample. Cost: a fourth `MaterialXGpu` lane (64 B/row, 4.2 MB ceiling) and
  one more lobe in the FS. Prerequisite: R5. Ballot 13.
- **`Multiply` meshes** — stains, decal-like darkening: `(DstColor, Zero)`; lands with TK-1 in
  R9 (§1.1), drawn in the sorted bucket (it is not commutative).

## 4. The pass in detail — `forward_translucent.fs.hlsl`

One TU, generator-owned skeleton (the `emit_particles` idiom, `emit_particles.rs:34-40`), eDSL
spans between sentinels, `#include`s of `pbr_lighting.hlsli`, `shadow_apply.hlsli`,
`light_table.hlsli` (froxel helpers), `ddgi_resolve.hlsli`:

```
Set 0: Camera @0, Instances @1, InstanceMaterials @2, LightBuf @3, Materials @4,
       ClusterGrid @5, LightIndexList @6              (= forward_opaque_froxel's 7, byte-identical shape;
                                                       the light-table placeholder where no cull is built,
                                                       targets.rs:208 — read only under use_clusters, §2.4)
       MaterialsX @7                                    (R5)
       DDGI atlas @8/9/10                               (R1; the vb_shade_split shape)
       scene_color + sampler @11/12                     (R4/R5, -D REFRACT)
       oit_b0 / oit_moments @13/14                      (R7, -D OIT_STAGE=2)
Set 1: CSM + atlas (the forward Set-1 shape)   Set 2: bindless textures
Push (R3): the path's depth view via push descriptor
```

≤ 15 bindings on the worst set against `MAX_BIND_GROUP_BINDINGS = 24` (plan §G's worst set is
13). Variant axes and the count they produce (RESEARCH P25), re-derived after B1 and B5:

- `forward_translucent.fs`: `DEPTH_LINEAR` {0,1} (Deferred only; interface-identical, the
  `TERMINATOR_WRAP` precedent, so **not** a manifest row but still a sync pin) × the reachable
  `(REFRACT, OIT_STAGE)` pairs — `(0,0) (0,1) (0,2) (1,0)`; `REFRACT × OIT_STAGE ∈ {1,2}` are
  **not built** (a transmissive fragment is never a moment fragment, R5; and `OIT_STAGE = 1`
  neither lights nor refracts, so those two would have been dead builds twice over) = **8 `.spv`,
  4 manifest rows, 2 `.spv` at R1**. The first pass's "12" counted the four dead builds.
- `particle_draw.fs`: the existing base + `-D DEPTH_LINEAR` pair (`emit_particles.rs:16-18`) ×
  the designed `-D SOFT` {0,1} (`PARTICLES-PLAN.md:1509`) × `OIT_STAGE` {0,1,2} (R7: the alpha
  class joins) = **12 `.spv` at the top of the ladder (6 without `SOFT`)**, each with a manifest
  row where the interface changes (`SOFT` and `OIT_STAGE` do; `DEPTH_LINEAR` does not). The first
  pass counted none of these.
- Total new/regrown `.spv` at the top of the ladder for the two FS families: **20** (D18).

`FROXEL` and `TEXTURED` are **not** axes, for a reason the first pass mis-stated: not because the
pass is "froxel-only" (it is not — §2.4), but because bindings 5/6 are always declared and always
bound — to the cull's buffers under VB, to the light-table placeholder elsewhere — and the
three-term `use_clusters` gate chooses the arm at runtime, exactly as `forward_opaque_froxel`
does on every ForwardPlus boot (`forward_opaque.fs.hlsl:316-320`: "bound on the boots it
describes … against a Set-0 whose bindings 5/6 fall back to the light table"); and the pass is
bindless-always with the `!= 0` slot gate every consumer uses.

Per fragment, in order: material fetch → (R2 n/a here) → the point/spot loop under `use_clusters`
(froxel slice or flat block) with `shadow_apply` → `ddgi_probe_sample` into ambient under
`ddgi_mode != 0` → (R5) `transmission_split` with the chain sample at `refraction_lod` →
tonemap-space premultiply → `SV_Target0 = (rgb·a, a)`, `SV_Target1 = a` (R6). Under
`OIT_STAGE=1` the body is replaced by the moment write; under `=2` the lighting runs and the
output is `premul · T(z)`.

## 5. Aether constructs and Gaia profiles

**Aether.** `material` is parked in v2 pending the shader-policy decision
(`docs/aether-v2/CONSTRUCTS.md:275-277`); this design does not unpark it, it **records the keys the
construct must accept** so the shader-policy campaign designs around them, not against them:

```
material glass {
    base: (0.9, 0.95, 1.0, 1.0), roughness: 0.05, transmission: 1.0, ior: 1.5,
    blend: opaque,                 // opaque | mask(cutoff: 0.5, hashed: true) | blend
    shadow: translucent,           // opaque | translucent | none
    two_sided: sorted, sort_bias: 0.0,
    thickness: 0.0, attenuation: ((1, 1, 1), inf), dispersion: 0.0, specular: (1.0, (1, 1, 1)),
}
```

Emission stays a builder fn (`AETHER-LANG-PLAN.md:663-680`): `Material::new(…).with_x(MaterialXGpu
{ … })` — the route is derived by `Material::route()`, never written by the author (§1.1).
Defaults are the glTF defaults, so a material that names none of the new keys emits a
byte-identical `MaterialXGpu::default()` and routes `Opaque`.

**Gaia** (`profile=data`, the animation design's §6 shape): a `material` row bakes into the two
tables (`MaterialGpu`, `MaterialXGpu`) by blit; the bake **(a)** premultiplies the albedo of every
`blend` material before mip generation and marks the texture premultiplied (the loader refuses a
straight-alpha texture on a `Blend` material — eager, closed), **(b)** emits `Translucent` /
`AlphaMasked` on every scene entity from `route()` (one authority), **(c)** for `thickness ≠ 0`
runs the manifoldness check (ballot 4 decides refuse vs warn), **(d)** refuses `dispersion ≠ 0`
with `thickness == 0` (Filament's rule, stated at bake instead of silently ignored).

## 6. Particles and MSDF text — folded or apart, decided

- **Particles — same seam, same key leaf, same OIT, separate draw (D11).** The billboard VS and
  the mesh VS cannot share one indirect draw, so under the sorted arm they remain two buckets
  (meshes, then alpha particles, then additive) — the compromise the recorder already documents,
  now with meshes as a third bucket in front. Under MBOIT the buckets vanish. Soft particles
  (`-D SOFT`) ride R3. Their LDR floor (`PARTICLES-PLAN.md:486`) is ballot 1's.
- **MSDF text / UI — apart (D12).** It draws onto the swapchain after every AA pass
  (`present_blit.rs:185-200`), which is the *right* place for screen UI (never temporally filtered,
  never tonemapped twice). World-anchored UI is screen rects projected per frame
  (`crates/boyko_ui/src/world/`); depth-occluded world text would be a `translucent_draw` client
  with the MSDF leaf — ballot 6, not v1.

## 7. Participation matrix (what a translucent is and is not part of)

| Effect | Opaque | Masked (R2) | Translucent sorted (R1) | Transmissive (R5) | OIT (R7) | Alpha particles | UI text |
|---|---|---|---|---|---|---|---|
| VB `vb_id` / classify / `vb_geo` | in | in | **out** | out | out | out | out |
| HZB occluder | yes | yes (kept texels) | **no** | no | no | no | no |
| HZB occludee | yes | yes | frustum only (D14) | same | same | no | no |
| Depth write | yes | yes | **no** | no | no | no | no |
| SSAO | receives + occludes | same | **neither** | neither | neither | neither | — |
| DDGI | receives + (SDF leg) contributes | same | **receives** (probe sample) on Deferred/VB; bound-but-unread on Forward/ForwardPlus (`cap_forward_v1_consumers`) | receives | receives | P3 (froxel) | — |
| CSM / atlas | receives + casts | receives + casts (masked caster) | receives; casts **nothing** (R1) → grey/colour transmittance (R9) | same | same | receives (P3) | — |
| SDF shadows (`sdf_mesh_shadow`) | receives + casts | same | receives (R9b, one march per fragment — priced there) | same | same | no | — |
| HW-RT TLAS / `shadow_vis` (`hwrt`) | packed (the gather ring + `mesh_ids` lane, `mesh_draw.rs:36-50`) + casts | packed; masked caster **UNVERIFIED** (an any-hit for cutout is not designed here) | **not packed**, casts nothing; receives the denoised `shadow_vis` only if the FS samples it (not in R1 — CSM/atlas only) | same | same | not packed | — |
| TAA (Deferred, VB only — none on Forward/ForwardPlus, §2.2) | history + camera MV | history; hashed noise integrates | history; **no MV**; `taa_cov` (R6) | same | same; MV eligible again | same as translucent | **outside** |
| `scene_color` chain (R4) | in | in | **out** (ballot 7) | out | out | out | out |
| Refraction (samples the chain) | — | — | R5 | yes | **sorted, before the moments — never a moment fragment** (R5) | no | — |
| Additive class (R1) | — | — | unsorted, after the sorted set | — | unsorted, after `oit_composite` | additive particles: same | — |
| Reflections | sky + EnvBRDF | same | same (front-layer SSR later, R10) | same | same | none | — |

## 8. Migration and byte-identity

- Default path, default consumers: `translucent_wanted = false` ⇒ no pass declared, no image
  allocated, no pipeline created, no gather registered — the 0 %-gate (`render_path_config.rs:14-19`
  shape) and the F9 rule "a declared ResId no pass names routes zero barriers"
  (`PARTICLES-PLAN.md:192`). Every existing golden holds by construction (G1 asserts it).
- The `Without<Translucent>` term on the opaque gather is a filter over a component no existing
  entity carries — the query's result set is unchanged.
- The Deferred 64-unit horizon (RESEARCH P17) is **pinned, not hidden**: G7 places a translucent
  at 70 units on Deferred and asserts the documented disappearance, so moving `MESH_DEPTH_T_MAX`
  is a visible two-site edit (`gbuffer_mrt.fs.hlsl:114` and its host mirror
  `crates/boyko_rhi_vulkan/src/compute.rs:3250`; the particle `-D DEPTH_LINEAR` pair reads the
  host constant, so it moves with them).

## 9. eDSL leaves and their gates

| Leaf (generic body, `f32` oracle + `Emit`) | Used by | Gate |
|---|---|---|
| **`log2`**, **`quantize_u(x, max)`** — two `FieldScalar` op growths (`scalar.rs:56-172` has neither) | the two leaves below | `f32` arm == `core`; `Emit` arm prints `log2(...)` / `(uint)(... + 0.5)` |
| `log_depth_unit(d, log_near, inv_log_span) → u ∈ [0,1]` — **new** (the particle key is a hand-written string today, `emit_particles.rs:216-241`) | sort key (CPU + GPU), MBOIT warp | `SORT_KEY_FN` re-emitted from it; `particle_edsl_sync` re-run; the CPU sort's key == the leaf over the fixture |
| `sort_key(u, bias, KEY_BITS)` — **new** | particles (8), meshes (16) | bin monotonicity + inversion pinned at both widths; saturation at both range ends pinned (G15) |
| `hashed_alpha_threshold(obj_pos, pixel_scale)` | R2 `MASK_HASHED` | oracle grid vs HLSL (`*_edsl_sync`); stability under translation (the paper's property) |
| `premul_over(src, dst)` | tests only (the blend HW does it) | oracle == the HW result on a 2-fragment fixture, per format |
| `fresnel_f0(ior)`, `transmission_split(E, diffuse, t)` | R5 | oracle vs the glTF formulas (§RESEARCH 5) |
| `refraction_lod(roughness, ior, mips)` | R5 | oracle vs three.js's mapping |
| `beer_lambert(c, d, x)` | R8 | three spellings agree to 1 ULP |
| `refract_offset(n, v, ior, thickness)` | R8 | parallel-exit invariant (`dir == view ray`) |
| `mboit_moments(absorbance, z_warped)`, `mboit_transmittance(b0, moments, z_warped)` | R7 | oracle vs brute-force sorted compositing on a 16-layer fixture; the bias envelope reported |
| `coverage_accumulate` | R6 | HW blend == `1 − Π(1−a)` on a fixture |

New shader TUs and their pins, **with who owns the skeleton** (the eDSL authors no sampling,
`discard`, stores or atomics, `emit_particles.rs:34-36`; of the 17 sync tests in
`crates/boyko_rhi_vulkan/tests/`, the `*_edsl_sync` ones pin generator-owned files and the
`*_spv_sync` ones pin hand-written files to a re-DXC — `vb_raster_geo_classify_spv_sync.rs` is
spv-only, `gbuffer_mrt_edsl_sync.rs` is eDSL-owned):

| TU | Skeleton | Pin |
|---|---|---|
| `forward_translucent.{vs,fs}` | **generator-owned** template (the `emit_particles` idiom) with eDSL spans for the §9 leaves; the sampling/`discard`-free lighting body is `#include`d | `forward_translucent_edsl_sync` |
| `scene_color_mips.comp` | generator-owned (the `hzb_build` shape) | `scene_color_mips_spv_sync` + host downsample oracle |
| `oit_composite.comp` | generator-owned; the reconstruction leaf is eDSL | `oit_edsl_sync` |
| `-D MASKED gbuffer_mrt.{vs,fs}` | **re-emitted** — the pair is eDSL-owned today | `gbuffer_mrt_edsl_sync` extends |
| `-D MASKED vb_raster.{vs,fs}` | **hand-edited** — the pair is hand-written HLSL pinned by `vb_raster_geo_classify_spv_sync` only; the hashed-threshold leaf is spliced between sentinels, the `discard` and the geometry-table UV pull (R2) are hand-written | `vb_raster_geo_classify_spv_sync` extends (spv-only) |
| `-D MASKED forward_opaque.{vs,fs}` | **hand-edited** (hand-written pair, no `_edsl_sync` exists for it) | a new `forward_opaque_spv_sync` |
| `-D MASKED` CSM / atlas casters | **hand-edited** | their existing spv pins extend |
| `csm_trans` caster pair | hand-written (a depth-tested blended write, nothing to author in eDSL) | a new spv pin |

So the "`*_edsl_sync`" promise holds for one of the four masked raster pairs; the other three are
legitimate hand-edited HLSL under `CLAUDE.md`'s rule (only eDSL-OWNED HLSL is generated), and the
first pass's blanket "every shader body is an eDSL leaf" is withdrawn (§0). Manifest rows:
`REFRACT`, `OIT_STAGE`, `MASKED`, `SOFT` (each changes an interface); `DEPTH_LINEAR` is
interface-identical (the particle precedent) and gets a pin only.

## 10. Gates (all red-first)

- **G1 exit**: on every path, a scene with one `Translucent` instance dumps `vb_id` (VB) / the
  G-buffer (Deferred) with **zero** texels of that instance id, and the default-config goldens
  are byte-identical with the feature compiled in.
- **G2 rejoin**: the same scene's `lit` differs from the opaque-only golden **only** inside the
  instance's screen bounds, per path (four pins).
- **G3 route**: the marker ⇔ `Material::route()` debug system fires on a fixture that mutates a
  material's `blend` without re-deriving the marker.
- **G4 sort**: two overlapping panes composite identically to the analytic premultiplied OVER,
  and swapping their spawn order changes nothing. The 8-bit **control must be able to go red**,
  and the first pass's spacing (1.0 m / 1.05 m) cannot make it: a quantiser maps two depths to
  different bins whenever their log gap is ≥ one bin, and `log2(1.05) = 0.0704` octaves exceeds
  the 8-bit bin of 0.0586 (`emit_particles.rs:188`) — so the 8-bit arm sorts those panes correctly
  and the control passes, a check that could not fail. The fixture is instead built **from the
  leaf's own bin edges**: at `d = 1.0 m`, `t = (log2 1 + 3)/15 = 0.2` and `t·255 = 51.0` exactly,
  a bin centre; the second pane at `d = 2^0.02 = 1.014 m` (1.4 cm away) has `t·255 = 51.34`, which
  the key's `+ 0.5` rounding (`:240`) sends to the same bin 51 — a **deterministic tie** — while
  at 16 bits the same pair is 87 bins apart (`0.02/15 × 65535 = 87.4`) [D-arith]. Tied keys leave
  the stable sort in gather order, so swapping spawn order swaps the composite: 8-bit **red by
  construction**, 16-bit green — the number of §1.3 made visible by a gate that can fail.
- **G5 premultiply**: a mip-2 texel of a fixture albedo with a hard alpha edge shows no fringe
  under the bake path and a fringe under a straight-alpha control (the control proves the gate
  can fail).
- **G6 barrier stream**: the `particle_barrier_stream` pin re-measured with `translucent_draw`
  armed moves by exactly the declared accesses, per path.
- **G7 horizon**: Deferred, a translucent at 70 units: **not drawn** (the documented divergence),
  and drawn on the three reverse-Z paths.
- **G8 TAA mask**: the motion fixture (a translucent pane sliding over a static wall) with
  `taa_cov` off vs on — the trail length metric, and a **control** with `r = 0` equal to off
  (two fingerprints, "wrong only in motion").
- **G9 budget**: the translucent instance counter reports `N > 1 024` on a crowd fixture (count,
  not clock) — the diagnostic, not a clamp.
- **G10 OIT oracle**: MBOIT's `f32` reconstruction vs brute-force sorted compositing on a
  16-layer fixture; the energy-conservation readback (§R7) under the paper's envelope; a fixture
  above the envelope reads **red**.
- **G11 fallback, not arming**: on each of the three non-VB paths with `translucent_wanted`, the
  header's dims lane is `0` (`sync_cluster_light_gate`, `light.rs:1011-1015`) and the translucent
  FS takes the flat arm — pinned by the L1 discipline's own ON==OFF equality: the translucent
  `lit` under `clusters_enabled = true` on Deferred/Forward/ForwardPlus is byte-identical to the
  same scene under `clusters_enabled = false` (the enabled bit is stopped by the dims term, as
  `forward_opaque.fs.hlsl:361-366` documents for ForwardPlus). Under VB with `clusters_wanted`
  the froxel arm's result equals the flat arm's on a fixture where every light reaches every
  froxel (the `lighting_l1_host_oracle` shape). `translucent_wanted` arms nothing, so there is no
  W4-class boot failure to fixture; the hole is closed by the gate, not by a rule.
- **G14 blend ABI**: `abi_guard.rs` gains the four `const _: () = assert!(BlendFactor::X.as_i32()
  == VK_BLEND_FACTOR_X)` guards for TK-1's four variants beside the existing four (`:378-385`);
  a deliberately wrong discriminant fails to compile (the red-first proof of the guard).
- **G15 saturation**: a translucent at 5 000 units and one at 0.1 units both key to the saturated
  bins; the saturated-key counter reports 2; swapping their spawn order swaps their composite
  (the tie is real and visible, §1.3).
- **G12 hashed**: a masked quad at 2× and 8× distance keeps its coverage fraction within 5 %
  (the "disappears with distance" failure the hash exists to fix), and a plain-cutoff control
  loses it.
- **G13 shadow transmittance**: two translucent casters in either order agree to ≤ 1 LSB; an
  opaque caster over them is exactly 0.

## 11. Kernel and crate requests born from the design (each gets its own pass)

| # | Item | Why | Zero when unused |
|---|---|---|---|
| TK-1 | `boyko_rhi`: `BlendFactor::{SrcColor = 2, OneMinusSrcColor = 3, DstColor = 4, OneMinusDstColor = 5}` (`vulkan_core.h:2401-2412` [D]) **+ the four `ffi.rs` `VK_BLEND_FACTOR_*` constants + the four `abi_guard.rs` asserts** (G14) | R9 colour transmittance and the `Multiply` mesh class; enum growth, `as_i32` lowering unchanged — which is exactly why the discriminants must be right and guarded | yes |
| TK-2 | `boyko_rhi_vulkan`: `VK_KHR_push_descriptor` + read-only depth layout + `SampledDepthAtReadOnly` (D13, `PARTICLES-PLAN.md:1888-1902`) | R3; shared with `-D SOFT` | yes |
| TK-3 | `boyko_rhi_vulkan`: a colour mip chain via `add_image_mipped` + `scene_color_mips.comp` | R4 | yes |
| TK-4 | `boyko_render`: `MaterialXGpu` + its `MaterialTable` mirror and staging ring | R5+ | 3.1 MB ceiling, bound by the translucent FS only |
| TK-5 | `boyko_render`: `RenderPathConsumers::{translucent_wanted, oit_mode, translucent_shadows}` + the two arming rules (`oit_mode`, `translucent_shadows`); `translucent_wanted` arms nothing and `froxel_light_cull_is_vb_only` stays as written | §2.4 | yes |
| TK-6 | `boyko_render`: the two-pass counting sort over bits 16..31 of a `ScratchColumn<u32>` key lane (in `mesh_draw`, no new crate); runs keyed `(mesh_id, two_sided, blend_class)` | §1.3 | yes |
| TK-7 | `boyko_shaderdsl`: **create** the sort-key leaves — `log2` + `quantize_u` op growths, `log_depth_unit` + `sort_key(…, KEY_BITS)`, `SORT_KEY_FN` re-emitted from them with `particle_edsl_sync` re-run — then the remaining leaves of §9 | §1.3, §9 | — |
| TK-13 | `boyko_rhi_vulkan`: `VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT = 0x100` in `ffi.rs` + a boot probe of `R32_SFLOAT` for it (the `device.rs:3264-3484` idiom); `oit_b0` falls back to `R16_SFLOAT` | §1.5, R7 | yes |
| TK-8 | Gaia bake: premultiply-before-mips for `blend` albedo + the premultiplied texture flag; marker emission; manifold check | §5 | — |
| TK-9 | `boyko_image` / texture upload: honour the premultiplied flag on the T2 blit chain | §5 (a) | yes |
| TK-10 | TAA: `taa_cov` read + the feedback shrink | R6 | yes |
| TK-11 | Aether (parked `material`): the key list of §5 recorded in `aether-v2/OPEN.md` | §5 | — |
| TK-12 | Frame graph: `translucent_draw`, `scene_color_*`, `oit_*`, `csm_trans_depth` declared in **all three** declarators and recorders | §2.2 | zero barriers when undeclared (F9) |

## 12. Migration path (order of landing)

R1 → R2 → R3 → R4 → R5 → R6 → R7 → R9 → R8 → R10 → R11 — R6 before R7 because the mask is cheaper
than OIT and its fixture (G8) is the one that shows whether OIT is even needed for the owner's
content; R9 before R8 because shadows from glass are visible in every scene and volume is visible
in few. R11 sits last as a scope event, but the owner may pull it forward (ballot 1), in which case
R5 and later are built once, in linear light. The rungs the critique added slot after their
prerequisites and none before its ballot: R1c after R1's GPU zone reports (ballot 15), R3b after
R3 (ballot 14), R5b after R5 (ballot 13), `Multiply` with TK-1 in R9. TK-7 (the key leaves) and
TK-13 (the `R32_SFLOAT` blend probe) precede R1 and R7 respectively.

## 13. Decisions

### 13.A PERF / ARCHITECTURE — taken here, with the numbers

| # | Decision | The number that decides it |
|---|---|---|
| D1 | The exit is at the **gather** (`Without<Translucent>`), never in a shader | one id per pixel, no discard (`vb_raster.fs.hlsl:1-5`); every shipping VB engine forks at the draw level (RESEARCH §2); a filter term costs the query nothing |
| D2 | Transparency parameters in a **cold extension table** (`MaterialXGpu`), the 48-B base frozen | 3.1 MB ceiling vs re-pinning `MATERIAL_GPU_WORDS = 12` in every shader and re-blessing every golden for a lane most rows never read |
| D3 | The route is **derived** (`Material::route()`), markers are structural, one authority + a debug check | the `MATERIAL_FLAG_TEXTURED` precedent (`material.rs:112-121`); RESEARCH P32 |
| D4 | Cutout is a **masked raster variant on the opaque rail**, bucketed by `(mesh_id << 1) \| masked` | 0 new lanes; batches at most ×2; the masked pixel is one surface (Nanite [D]) |
| D5 | Premultiplied OVER is the **sorted** mesh blend state and the bake premultiplies before mips; **additive** is the second class (unsorted, existing proof); **multiply** is the third, behind TK-1 | filtering is where straight alpha fails (RESEARCH P9); `PREMULTIPLIED_ALPHA` and `ADDITIVE` exist (`enums.rs:913`, `:949`); every surveyed engine ships all three (Bevy `AlphaMode` [D]) |
| D6 | v1 sort = CPU two-pass counting sort over bits 16..31 of a 32-bit `(depth16, mesh_id)` word, draw runs keyed `(mesh_id, two_sided, blend_class)`; GPU sort + MDI is the rung that lifts the budget | 0.0159 % vs 4.15 % per bin on the leaf's real 15-octave range; < 20 µs at 4 096 (estimate); the 1 024 line is a **placeholder** — its 0.2–0.5 µs per draw is uncited and G9 + a measured per-draw cost replace it |
| D7 | The sort key is a **new** leaf pair (`log_depth_unit`, `sort_key`) that TK-7 creates and the particle key is **re-emitted from**, at `KEY_BITS = 16` for meshes and 8 for particles, over the particle key's fixed `[0.125, 4096]` range stated as a contract | today the particle key is a hand-written string with no oracle (`emit_particles.rs:216-241`); "one body, one oracle" is a property TK-7 **establishes**, not one the tree has |
| D8 | The translucent FS carries the **three-term `use_clusters` runtime gate** with the flat-table fallback and arms **nothing**; froxel lists under VB only | `froxel_light_cull` is VB-only by code and by test (`render_path_config.rs:1224`, `:3385-3406`); the dims lane is pinned to 0 elsewhere (`light.rs:1011-1015`); `clusters_enabled` is per-frame (`light_policy.rs:190-193`); the fallback body already exists (`forward_opaque.fs.hlsl:368-390`); the cull never reads depth (`cluster_cull.hlsl:436-455`) so the list is valid for non-depth-writing geometry |
| D9 | Transparents draw **before TAA** without motion vectors; the mask (R6) is a second attachment under the **same** blend state | the coverage identity `a + c(1−a) = 1 − Π(1−a)` needs no `independentBlend`; R10's sort ⇔ MV exclusion (`gpu_scene/particle.rs:400-412`) |
| D10 | Refraction = the opaque colour mip chain, opaque + sky only; no half-res transparency tier | 11.1 MB, ≈ 0.05–0.15 ms (estimate); Doom Eternal's choice [B]; HDRP's pre-refraction sub-pass is ballot 7 |
| D11 | **MBOIT** is the OIT rung; alpha particles and premultiplied meshes merge under it; transmissive meshes stay a sorted pass in front of it; the additive class draws after its composite | RESEARCH §4.2: the only family with all-additive blend on this RHI — **that fit table decides it, not the millisecond**: the published 3.5 ms / ~20 MB is one 1080p scene on a 3080 laptop [B] and the owner's box is 1440p on a 3060 (§1.5, 1.78× the pixels); two pipelines cannot share a permutation, but they share a moment sum; `a = 1` has no finite absorbance (R5) |
| D12 | UI / MSDF text stays outside the frame graph | it must be neither temporally filtered nor tonemapped (`present_blit.rs:185-200`) |
| D13 | Grey translucent shadows first, colour behind one enum growth | `(ZERO, 1−SRC_A)` exists; colour needs `OneMinusSrcColor`; ≤ 1 LSB order dependence stated |
| D14 | Translucents are frustum-culled only in v1; HZB occludee test is a later rung | the batch cull is GPU over VB batches (`vb_batch_cull`) and translucents are CPU-gathered; an occlusion-only cull mode is a separate piece |
| D15 | No depth peeling, no PPLL, no MLAB, no stochastic — rejected, not deferred | geometry passes / unbounded memory / an extension not queried / MSAA literals (RESEARCH §4.2) |
| D16 | Thin-only transmission before volume; box/thin shape only | avoids the manifold contract, the thickness column and the upside-down sphere (RESEARCH P14/P15) |
| D17 | SDF shadows and SDFDDGI never see a translucent as an occluder or emitter | no published precedent for a transmissive SDF occluder (RESEARCH §7); receiving is one probe sample |
| D18 | The variant space is bounded to **8** `forward_translucent.fs` `.spv` (4 manifest rows) + **12** `particle_draw.fs` `.spv` at the top of the ladder, 2 at R1 | the four `REFRACT × OIT_STAGE ∈ {1,2}` builds are dead (R5) and removed; `particle_draw.fs` gains `OIT_STAGE` on top of `DEPTH_LINEAR` × `SOFT` (§4); `FROXEL`/`TEXTURED` are not axes because the bindings are always bound and the gate is runtime (§2.4); every row in the manifest |

### 13.B VALUES / SCOPE — to the owner

1. **HDR scene colour (R11)** — now, before R5, or never: it re-blesses 30 pins, moves TAA
   pre-tonemap and rewrites the additive proof; without it transmission and absorption are
   display-referred approximations, which the particles already accept. Recommended: **after R6**,
   before R7 is armed by default.
2. **Deferred's 64-unit horizon** for translucents — accept (pinned by G7) or move
   `MESH_DEPTH_T_MAX` at both sites and re-bless every Deferred pin.
3. **Thin-only v1** (stop at R5) or the full glTF stack (R8).
4. **Manifoldness on `thickness ≠ 0`** — bake **refusal** (recommended, eager/closed) or a warn.
5. **OIT in this campaign or its own** — R7 is the largest rung (three images, two passes, one
   composite, new variants of two FS families) and its published number is one scene on one laptop.
6. **World-space text occluded by depth** (a `translucent_draw` client with the MSDF leaf) —
   scope or not.
7. **Transparents behind refractive surfaces** (HDRP's pre-refraction sub-pass + depth copy) —
   the chain holds opaque + sky only in v1.
8. **Reflections on glass** — the front-layer SSR rung waits on an SSR pass that does not exist;
   accept sky + EnvBRDF until then.
9. **The 1 024-instance v1 budget** and whether the MDI rung is planned now.
10. **Coloured translucent shadows** (the enum growth) in R9 or later.
11. **Two-sided sorted draw** authored per material (2× draws) — keep as the v1 intra-object
    mitigation or wait for OIT.
12. **`r = 0.75`** TAA mask strength as the starting value the fixture tunes (uncited).
13. **Diffuse transmission** (R5b) — a fourth `MaterialXGpu` lane (64 B/row) for leaves, paper,
    lampshades; in this campaign or its own.
14. **Transparent depth prepass / postpass per material** (R3b) — concave-mesh intra-object
    order and depth for post effects, at one extra draw per opted-in instance.
15. **A cheap translucent lighting tier** (R1c, per-vertex or a translucency volume) — whether
    the owner's content has the smoke-class depth complexity that makes the per-pixel tier the
    wrong default.

## 14. Unverified / open

- Every millisecond above is either the published RTX-3080-laptop set [B] or bandwidth/count
  arithmetic labelled as an estimate; the translucent FS cost `C_fs`, the CPU draw cost per
  `vkCmdDrawIndexed` (0.2–0.5 µs), the hash's "~20 ALU", the "100–200 GB/s effective" and
  `r = 0.75` are **uncited numbers** — none decides a rung (D6 and D11 now say what does), and
  each is replaced by the GPU zone or the fixture that measures it on this box.
- **`R32_SFLOAT` colour-attachment blend support** is not confirmed to be in Vulkan's mandatory
  format set: three fetches of the spec's formats chapter (two by the critique, one by this pass,
  2026-09-10) returned without the required-support tables. TK-13's boot probe settles it per
  device; the design does not depend on the answer.
- What the opaque producers write into `lit.a` was not read; the design uses a separate `taa_cov`
  rather than repurposing it.
- The GLB loader / meshlet build make no manifoldness claim that was checked (R8's contract).
- The CSM cascade resolution was not read; R9's memory is per cascade texel.
- The Forge 15a node layout, Volition's weight, AVBOIT's structure and Cyberpunk's parallel slab
  are abstract-level only; none is relied on.
- Whether `discard` on this device class keeps the early depth *test* under the reverse-Z
  pipelines (the conservative-depth contract) is the spec's promise, not a measurement here.
- `ddgi_probe_sample`'s cost per fragment and the probe atlas's on-screen contribution
  ("bound-unread" in the deferred resolve, `SHADER-VARIANT-MANIFEST.md:34`) — the translucent FS
  samples it, but what it adds on screen is the SDFDDGI campaign's I5 verdict, not this one's.
- MBOIT's bias envelope for this engine's content is what G10 measures; the paper's own is
  quoted, not reproduced.
- **MBOIT at 16 bits (second pass).** The published 3.5 ms is a 16-bit-per-channel configuration;
  under hardware additive blending that is `SFLOAT16`, and that is what R7 assumes. The paper's
  non-linearly quantised UNORM16 moments may need a per-fragment change of variable or a
  non-hardware accumulation — not confirmed (the PDF exceeded the fetch limit). If SFLOAT16 loses
  precision on this engine's depth ranges, the fallback is `R32` moments at the published
  8.1 ms class, not NLQM.
- **A stale doc comment observed, not repaired** (the architect edits no code): the module doc of
  `crates/boyko_render/src/gbuffer_depth.rs:1-16` says the perspective gbuffer fragment divides by
  `T_MAX = 10.0`; `gbuffer_mrt.fs.hlsl:99-114` divides by `MESH_DEPTH_T_MAX = 64.0` under
  perspective and writes `position.z` under ortho, with `T_MAX` living only in the marcher's
  decode. The pinned constants (`GBUFFER_T_MAX == SDF_TRACE_T_MAX == 10`) are still consistent
  with each other; only the prose is wrong. Worth one line in whichever pass next touches that file.
- **Masked geometry under HW-RT** (§7's new row): the TLAS build packs every opaque-gather instance;
  whether a cutout caster needs an any-hit alpha test in the `hwrt` shadow ray, and what that costs
  in `shadow_vis`, is not designed here — R2 is scoped to the raster and CSM/atlas casters.

## 15. Critique log — pass 1 (2026-09-10)

Each finding of the `architecture-critic`'s first pass, with what this revision did about it —
the `ANIMATION-DESIGN-SPACE.md:667-693` shape. "Fixed" means the text above now says something
different; "refuted" means the finding's evidence was re-opened and does not support it; "partly"
means part of the chain was wrong and the rest was fixed. Every `file:line` here was re-opened by
this pass on `feat/multi-paradigm-render` (working tree of 2026-09-10; the first pass's anchors at
`ed0bed45` were re-confirmed where cited).

| # | Finding (short) | Action |
|---|---|---|
| B1 | D8 / §2.4: "froxel-only FS" + "`translucent_wanted ⇒ froxel_light_cull` on every path" contradicts the tree; the cull is VB-only and three sites enforce it; the FS would read a placeholder on three paths — the W4 class D8 claimed to close | **Fixed.** Re-opened: `render_path_config.rs:1224` (`&& matches!(path, VisibilityBuffer)`), the test `froxel_light_cull_is_vb_only` `:3385-3406`, `light.rs:1011-1015` (dims pinned to 0), `graph_bridge.rs:2745-2750`, `light_table.hlsli:341`, `light_policy.rs:190-193`, `targets.rs:208` (the placeholder). The critic's first option is taken: the FS carries the three-term `use_clusters` gate with the flat fallback — which the design's own cited FS already has at `forward_opaque.fs.hlsl:368-390` (the first pass cited its bindings at `:118-128` and missed the body). Arming L1 elsewhere was priced (`gpu_scene/mod.rs:5369-5380`) and not taken. §0, §2.4, R1, §4, D8, G11, TK-5 rewritten; D18 re-derived; "FROXEL is not an axis" stands for the corrected reason |
| B2 | R9 / TK-1: `{SrcColor = 4, OneMinusSrcColor = 5, DstColor = 8, OneMinusDstColor = 9}` are the wrong `VkBlendFactor` values; `as_i32()` lowering would bind the wrong factor silently; `abi_guard` guards only the existing four | **Fixed.** Verified against the installed SDK: `C:\VulkanSDK\1.4.350.0\Include\vulkan\vulkan_core.h:2401-2412` — `SRC_COLOR = 2, ONE_MINUS_SRC_COLOR = 3, DST_COLOR = 4, ONE_MINUS_DST_COLOR = 5, DST_ALPHA = 8, ONE_MINUS_DST_ALPHA = 9` [D]; `enums.rs:849-855`, `rhi_impl/device.rs:1886-1897`, `ffi.rs:799-802`, `abi_guard.rs:378-385` re-opened. R9, TK-1 corrected; the four `ffi.rs` constants and four asserts are in TK-1's scope; G14 added |
| B3 | §1.3 / D7 / TK-7: no eDSL leaf for the particle key exists (a hand-written string, `emit_particles.rs:216-232`); the range is fixed `[0.125, 4096]`, not `far/near = 10⁴`; the per-bin figures are 4.15 % / 0.0159 % | **Fixed.** Re-opened `emit_particles.rs:184-192`, `:216-241`, `:34-36`; `scalar.rs:56-172` shows the eDSL also lacks `log2` and a float→uint quantise. §1.3 now states the fixed range as a contract with saturation and a counter (G15); the figures are the leaf's; TK-7 is "create the leaves + two op growths + re-emit `SORT_KEY_FN`"; D7 says "one body, one oracle" is established by TK-7, not present; the camera-derived range is a later knob the leaf's inputs already admit |
| B4 | G4's 1.0 m / 1.05 m 8-bit control cannot go red: `log2(1.05) = 0.0704 > 0.0586` | **Fixed.** [D-arith] confirmed, and the key's `+ 0.5` rounding (`emit_particles.rs:240`) does not change the argument. G4's fixture is now built from the leaf's bin edges: 1.000 m is exactly bin centre 51, 1.014 m rounds to the same bin (8-bit tie, deterministic) and is 87 bins away at 16 bits. §1.3's "5 cm at 1.5 m" example is withdrawn |
| B5 | Refraction × MBOIT is promised but undefined: `a = 1` ⇒ absorbance `+∞` ⇒ `exp(−b0) = 0` blacks the pixel | **Fixed.** [D-arith] confirmed against RESEARCH §4.1 item 4. Transmissive fragments are excluded from the moments and draw sorted, `-D REFRACT`, `OIT_STAGE = 0`, **before** the moment passes (HDRP's order), so alpha in front of glass composites and alpha behind glass is the ballot-7 loss already stated. R5, R7 ("Closes" narrowed to non-refractive), §2.3, §4 (four dead variants removed), §7, D11, D18 updated |
| NB1 | DDGI @8/9/10 and `shadow_apply` bound on every path while the resolver forces DDGI/TAA off under Forward/ForwardPlus; the design should say it follows the bound-but-unread + `ddgi_mode != 0` discipline; no TAA on those paths | **Fixed.** `render_path_config.rs:1351-1381`, `scene_types.rs:2568-2577`, `vb_shade_split.comp.hlsl:38-41` re-opened. §2.4 states the discipline explicitly; §2.2 and §2.3 say Forward/ForwardPlus have no TAA and no `taa_cov`; §7's DDGI and TAA rows carry it; R6 scoped to Deferred/VB |
| NB2 | `R32_SFLOAT` `COLOR_ATTACHMENT_BLEND_BIT` may not be mandatory; name a probe and a fallback | **Fixed (the probe), still UNVERIFIED (the table).** A third fetch of the spec's formats chapter also returned without the required-support tables. TK-13 adds the bit (`vulkan_core.h:2676` = `0x100` [D], beside `ffi.rs:1154`'s `0x80`) and a boot probe in the `device.rs:3264-3484` idiom; `oit_b0` falls back to `R16_SFLOAT`; §1.5, R7, §14 |
| NB3 | 1440p cells never summed, machine never named; the 3.5 ms is a 1080p/3080 figure used as the decider | **Fixed.** `OPTIMIZATION-PLAN-RENDER.md:44`, `LIGHTING-PLAN.md:39`, `present/mod.rs:92` (`FRAMES_IN_FLIGHT = 2`) re-opened; §1.5 sums to ≈ 101 MB ≈ 1.6 % of 6 GB; R7 and D11 say the fit table decides, not the millisecond |
| NB4 | Four uncited numbers used as deciders; R4's 11 MB undercounts the copy | **Fixed.** Each is labelled uncited at its site (§1.3, R2 unchanged as "~20 ALU" estimate, R4, R6) and §14 lists them; D6 demotes the 1 024 line to a placeholder; R4's traffic recounted to ≈ 30 MB [D-arith] and its estimate to 0.15–0.3 ms |
| NB5 | D5 omits additive and multiply meshes, diffuse transmission, transparent depth prepass/postpass, a cheap lighting tier | **Fixed.** `enums.rs:935-948`, `gpu_scene/particle.rs:659-698`, `shadow_apply.hlsli:8` re-opened; Bevy `AlphaMode` / `StandardMaterial` and HDRP execution order cited as opened by the critique [D]. §1.1 gains the three-class `blend`; R1 gains the additive class at 0 cost; `Multiply` lands with TK-1; R1c / R3b / R5b added as rungs with prerequisites; ballots 13–15; D5 rewritten; R1's cost names the single-tier consequence |
| NB6 | R2 under VB understates the vertex-side change (position-only `VsIn`, UVs in the geometry table); §0's "no discard" contradicted by the MASKED variant | **Fixed.** `vb_raster.vs.hlsl:158`, `:167`, `:169-177` and `vb_shade.comp.hlsl:68`, `:359-412` re-opened. R2 specifies a geometry-table UV pull by `SV_VertexID` bound to the VS (a Set-0 growth on both stages, one manifest row); §0 restates the invariant for the opaque variant |
| NB7 | Draw runs keyed by `mesh_id` alone cannot honour per-material `two_sided`; the sort's pass count over a 32-bit word is ambiguous | **Fixed.** `mesh_draw.rs:155-182` re-opened. Run key is `(mesh_id, two_sided, blend_class)`; §1.3 states two passes over bits 16..31 with the low half as payload; TK-6 and D6 follow |
| NB8 | §2.3's `[R7 armed]` line dropped the additive particle draw | **Fixed.** `passes/particles.rs:771-778` re-opened; the additive class (particles and, now, meshes) draws after `oit_composite` under every arm so its pure-sum property survives; §2.3, §7 |
| NB9 | D18's 12 is wrong both ways (dead `REFRACT × OIT_STAGE=1`; `particle_draw.fs` gains `OIT_STAGE`); the Deferred D7 row changes at R3 | **Fixed.** `emit_particles.rs:10-18`, `PARTICLES-PLAN.md:1509`, `:1888-1902` re-opened. §4 re-derives 8 + 12; D18 follows; §2.2's Deferred row carries the R3 read-only-layout amendment |
| NB10 | §0 overclaims "every shader body is an eDSL leaf"; three of four masked raster pairs are hand-written | **Fixed.** The 17 sync tests listed (`crates/boyko_rhi_vulkan/tests/`), `shaderdsl/src/lib.rs:1-30`, `emit_particles.rs:34-36` re-opened. §0 restated; §9 gains a per-TU ownership table saying which files are re-emitted and which hand-edited |
| NB11 | RESEARCH §0 says "default off" under VB but not that the cull is structurally absent on Deferred/Forward and dims-pinned on ForwardPlus; the `gbuffer_depth.rs:1-16` stale-doc note is correct | **Fixed in RESEARCH** (§0 row, §14 there); the stale-doc observation stays recorded, unrepaired (the architect edits no code) |

Nothing was left open with a decision pending: every finding is fixed above or partly refuted
with the evidence stated. Two facts remain **UNVERIFIED** and are carried in §14 rather than
decided — the spec's mandatory blend support for `R32_SFLOAT` (a boot probe removes the
dependence) and the manifoldness claim of the loader (unchanged from pass 0).
