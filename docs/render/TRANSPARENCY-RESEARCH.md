# Transparency — the comparative survey (research record)

> Status: research record for the transparency campaign, 2026-09-10. Companion: the design for
> THIS engine is [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md). The tree holds
> **no world-geometry transparency** today — two blended draws exist (UI rects, particle
> billboards) and nothing else (§0).
>
> **Provenance discipline.** Every claim about another engine or a paper carries a tag: **[S]**
> source code read, **[D]** official docs / spec / paper, **[B]** blog / talk abstract / marketing
> (recorded, not relied on), **[T]** a file in this tree re-opened by the author of this document,
> **[D-arith]** arithmetic derived from a tagged fact. External URLs were opened by the three
> research lenses of this session and are carried through; every in-tree `file:line` was
> **re-opened by the architect** on branch `feat/multi-paradigm-render` at commit **`ed0bed45`**
> and corrected where a lens was off (§13 lists the corrections). Anything neither source-read
> nor doc-read is marked **UNVERIFIED**. This directory is not in
> `tests/internal_docs_anchors.rs`'s `GATED_DOCS` (`tests/internal_docs_anchors.rs:231` — the
> list is `FEATURE_MAP`, `SYSTEMS`, `ARCHITECTURE`, `MESHLET-VIRTUAL-GEOMETRY-PLAN`), so no anchor
> below is machine-checked — treat a line number here as "true on 2026-09-10 at `ed0bed45`".
>
> No timings were taken and no `cargo` command was run for this record. Every millisecond figure
> is a published measurement quoted with its rig, or an arithmetic estimate labelled as one.
>
> **Second verification pass (2026-09-10, later the same day, same commit `ed0bed45`).** The
> first pass wrote this record and did not return; a second architect run re-opened **every**
> in-tree `file:line` below independently before accepting it. Five anchors had drifted or
> pointed at the wrong fact and are corrected in place; §13 lists each with the line it moved to.

## 0. The tree today — what is true before any design (all [T])

| Fact | Where |
|---|---|
| The RHI blend vocabulary is `BlendFactor::{Zero=0, One=1, SrcAlpha=6, OneMinusSrcAlpha=7}` and `BlendOp::{Add}` — "the family grows per phase" — with three presets `PREMULTIPLIED_ALPHA`, `STRAIGHT_ALPHA`, `ADDITIVE` | `crates/boyko_rhi/src/enums.rs:836-847` (factors), `:862-867` (op), `:913`, `:926`, `:949` (presets) |
| `GraphicsPipelineDesc::blend: Option<BlendState>` is lowered onto **the same factors for ALL colour attachments**; the doc names "a per-target slice" as the future widening | `crates/boyko_rhi/src/descriptor.rs:393-400`; the lowering `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:1882-1917` (`core::array::from_fn` replicates one state) |
| Blend state is **boot-frozen pipeline state**: `VK_EXT_extended_dynamic_state3` is not enabled, so the two particle classes are two `VkPipeline`s from one desc closure | `crates/boyko_app/src/gpu_scene/particle.rs:655-698`; `docs/PARTICLES-PLAN.md:537` |
| The device enables **only** `sampler_anisotropy` among core features; `independent_blend` exists as a field of the FFI struct and is never set | `crates/boyko_rhi_vulkan/src/device.rs:3794-3801`; `crates/boyko_rhi_vulkan/src/ffi.rs:2829` |
| `rasterization_samples = VK_SAMPLE_COUNT_1_BIT`, `alpha_to_coverage_enable = VK_FALSE` are literals in the one graphics-pipeline builder | `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:1845-1855` |
| `VK_EXT_fragment_shader_interlock`, `VK_KHR_shader_atomic_int64`, `VK_KHR_push_descriptor`: **zero occurrences** under `crates/` | `grep -rn` over `crates/` (three negative greps, 2026-09-10) |
| Exactly **three** `blend: Some(..)` pipelines exist: UI rects (premultiplied, onto the swapchain), particles ADDITIVE and STRAIGHT_ALPHA (into `lit`); 47 `blend: None` sites | `crates/boyko_render/src/ui/resources.rs:237`; `crates/boyko_app/src/gpu_scene/particle.rs:674` (one closure, two blends). A third `blend: Some` grep hit, `crates/boyko_demo/src/render/mod.rs:205`, is the wgpu demo backend — not this RHI, not counted |
| The transparent **composite slot already exists on every path**: `particle_draw` is declared after the last `lit` producer and before `taa_resolve` / `present_sample` — Deferred, Forward/Forward+, VisibilityBuffer | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:2447-2462` (Deferred), `:2983-2991` (Forward family), `:6170-6185` (VB); the declarator `:568-590` |
| The per-path depth truth table at that slot: Deferred `LESS` + `SRO → DEPTH_ATTACHMENT_OPTIMAL` transition; Forward `GREATER` + availability barrier; ForwardPlus `GREATER`, free; VB `GREATER` + availability barrier | `docs/PARTICLES-PLAN.md:461-476` (D7); `rhi_impl/device.rs` `create_graphics_pipeline_particle` |
| Deferred depth is **not hardware depth**: `SV_Depth = length(eye_rel) / MESH_DEPTH_T_MAX`, `MESH_DEPTH_T_MAX = 64.0`; a fragment that writes it loses early-Z, and anything past 64 units fails `LESS` — "Deferred particles disappear at 64 units" | `crates/boyko_rhi_vulkan/shaders/gbuffer_mrt.fs.hlsl:99-114` (the literal, `:114`); host mirror `crates/boyko_rhi_vulkan/src/compute.rs:3250` (`MESH_DEPTH_T_MAX: f32 = 64.0`); `crates/boyko_rhi_vulkan/shaders/particle_draw.fs.hlsl:22-42`; `docs/PARTICLES-PLAN.md:1519`. **Not** `gbuffer_depth.rs:1-16` — that module's doc describes the *other* normalizer (`T_MAX = 10.0`, `GBUFFER_T_MAX` at `:50`, pinned equal to the marcher's `SDF_TRACE_T_MAX`) and its opening paragraph still says the perspective fragment divides by `T_MAX`; the shader divides by `MESH_DEPTH_T_MAX` under perspective (`gbuffer_mrt.fs.hlsl:99-114`, `particle_draw.fs.hlsl:27`). A stale doc comment, recorded in §13, not repaired here |
| `lit` is an `R8G8B8A8` ring written **after** `tonemap_select` + manual gamma OETF; the only RGBA16F lanes are `gPbr` (a material MRT lane) and `taa_hist` (history) | `crates/boyko_rhi_vulkan/src/present/targets.rs:123-127`, `:145-153`, `:2368-2375`; `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:1395-1409`; `crates/boyko_render/src/material.rs:56-60` |
| The additive-particle **commutativity proof is written against 8-bit saturation** (`sat(sat(x)+y) == min(1, x+y)`) | `crates/boyko_rhi/src/enums.rs:935-948` |
| LDR consequence already recorded: "additive clips at white and contributions below 1/255 round to zero. Effects must be authored with contributions ≥ 2/255" | `docs/PARTICLES-PLAN.md:486` |
| **No colour mip chain exists**: the HZB is "the engine's FIRST storage image with a mip chain; every other `TextureDesc` call site passes `mip_levels: 1`"; `add_image_mipped` is the only route for a mipped resource and requires a cross-frame seed | `crates/boyko_rhi_vulkan/src/present/targets.rs:1440-1447`; `crates/boyko_rhi_vulkan/src/framegraph/graph.rs:365-380`; `vkCmdBlitImage` is loaded (`crates/boyko_rhi_vulkan/src/device.rs:635-637`) |
| **No SSR pass** exists — only an `ssr` consumer bit in the pre-light union | `docs/FEATURE_MAP.md:125` (the union: SSAO ∥ DDGI ∥ spatial denoise ∥ shadow temporal ∥ SSR) |
| The forward FS already has a **`-D FROXEL=1` compile** that walks `ClusterGrid`/`LightIndexList` at Set 0 bindings 5/6 — the same SSBOs `cluster_cull.hlsl` writes and `deferred_pbr.hlsl` reads at 8/9 — plus inline CSM + punctual-atlas shadows via `shadow_apply.hlsli` at Set 1 | `crates/boyko_rhi_vulkan/shaders/forward_opaque.fs.hlsl:1-50`, `:118-128`; `crates/boyko_rhi_vulkan/shaders/shadow_apply.hlsli:1-25` |
| The froxel cull is **purely geometric**: exp-Z slice bounds (`slice_view_z`) and a world AABB per froxel from the tile's corner rays — it never reads a depth buffer, so its light list is valid for geometry that writes no depth | `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl:99-103`, `:436-455` |
| Under VB the froxel cull is a boot-frozen consumer, **default off** ("VB v1 is fused-only… ALL-LIGHTS — no cluster/froxel lookup"). **On the other three paths it is not "default off", it is structurally absent** (critique pass 1, NB11): `froxel_light_cull = clusters_wanted && path == VisibilityBuffer` with the test `froxel_light_cull_is_vb_only` pinning it; `sync_cluster_light_gate` pins the header's dims lane to `0` whenever the bit is false; the `light_cull` pass is declared only under ForwardPlus with the cull built, and on Deferred `ClusterGrid` "is ALWAYS the light-table placeholder". Every lit producer therefore carries a **three-term runtime gate** `use_clusters = clusters_enabled ∧ dims ≠ 0 ∧ count ≤ GetDimensions` with the flat `[l0a_count, light_count)` fallback — including the `-D FROXEL` forward compile, which is bound on every ForwardPlus boot against placeholder bindings. A translucent FS inherits that gate; it does not arm a cull | `crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl:329-333`; `docs/FEATURE_MAP.md:127`; `crates/boyko_render/src/render_path_config.rs:1224`, `:3385-3406`; `crates/boyko_render/src/light.rs:1011-1015`; `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:2745-2750`; `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:1246-1249`, `:1264`; `forward_opaque.fs.hlsl:308-390`; `light_table.hlsli:341` (`clusters_enabled` is a per-frame header word, flipped by `light_policy.rs:190-193` under `Auto`) |
| DDGI probe sampling is a shared leaf (`ddgi_probe_sample`) any forward shader can include; in the deferred resolve the atlas is "(bound-unread) DDGI @16/17/18" and I5 multi-bounce is owner-eval | `crates/boyko_rhi_vulkan/shaders/ddgi_resolve.hlsli:68-107`; `docs/SHADER-VARIANT-MANIFEST.md:34`; `docs/RENDER-SDFDDGI-PLAN.md:136-140` |
| Path, legs and the pre-light consumer set are resolved **once at boot**; a per-frame toggle "would re-allocate fixed-size images/pipelines mid-stream"; the W4 hole (a consumer armed without its producer) is the documented failure class | `crates/boyko_render/src/render_path_config.rs:1-30`, `:90-120` |
| `MaterialGpu` is 48 B, three lanes: `base_color.w` = "alpha / cutoff", `mrr[3]` = flags (one bit, `MATERIAL_FLAG_TEXTURED`); `MATERIAL_GPU_WORDS == 12` is const-asserted host-side and pinned shader-side; **no shader reads `base_color.w`** (grep over `shaders/`: zero hits) | `crates/boyko_render/src/material.rs:47-121`; `crates/boyko_render/src/material_table.rs:1-30` (`Assets<Material>` authority + SSBO mirror, 16-bit `MaterialId`) |
| The mesh gather is count → prefix-sum → scatter into one instance ring, **one `DrawBatch` per mesh in mesh-id order**, every lane a `ScratchColumn` (a `ComponentPool`, never a `Vec`); there is **no per-object sort key** | `crates/boyko_render/src/mesh_draw.rs:1-40`, `:270-282`; `crates/boyko_ecs/src/ecs/core/component/scratch/scratch_column.rs:39-44` |
| A capability is **a component's presence**: `OcclusionCulling` is a ZST probed as `Option<&ZST>` per row in the gather and folded into `VbInstanceRow.flags` (bit 0; bits 1..31 reserved, "a word rather than a bool so piece 3 adds a BIT, not a column") | `crates/boyko_render/src/occlusion_marker.rs:1-30`; `crates/boyko_render/src/mesh_draw.rs:798-812`; `crates/boyko_render/src/instance_model.rs:224-256` |
| The shadow-caster gather is the same core with a `With<ShadowCaster>` filter term (`CsmCasterScratch` newtype over `MeshRenderScratch`) | `crates/boyko_render/src/csm_caster.rs:1-30` |
| The VB raster FS writes only `uint2(instance_id, SV_PrimitiveID)` — "NO `SV_Depth`, NO `discard`, NO UAV"; `VB_ID_SENTINEL = 0xFFFFFFFF` is sky/SDF; `vb_shade`/`vb_resolve` write `lit` as a **storage image** and leave sentinel pixels untouched | `crates/boyko_rhi_vulkan/shaders/vb_raster.fs.hlsl:1-27`; `vb_pack.hlsli:1-22`; `vb_resolve.comp.hlsl:123-127`, `:254-258` |
| `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` contains **zero** occurrences of `transparen|translucen|alpha`; the multi-paradigm plan reserves the seam verbatim: "Transparency/OIT, native MSAA … seam left open, not implemented. Extension points: `vb_classify` before `vb_shade`; forward-transparent pass reusing froxel SSBOs + shared depth" | grep count = 0; `docs/MULTI-PARADIGM-RENDER-PLAN.md:499` |
| Particles: two blend classes (`PARTICLE_BLEND_ADDITIVE = 0`, `PARTICLE_BLEND_ALPHA = 1`); the alpha class is GPU radix-sorted back-to-front over an inverted 8-bit log-depth key (hist → scan → scatter, three modules with no `-D`); `SortMode::Wboit` deliberately **not** landed; `-D SOFT` designed, blocked on the depth descriptor (D13: push descriptor); the recorder draws ALPHA then ADDITIVE and concedes "two draws cannot interleave two classes by depth" | `crates/boyko_render/src/particle.rs:95-101`; `docs/PARTICLES-PLAN.md:521-542`, `:1503-1509`, `:1798-1802`, `:1880-1902`; `crates/boyko_rhi_vulkan/src/compute.rs:5825-5854`; `crates/boyko_shaderdsl/src/bin/emit_particles.rs:8-10`, `:164-168`; `crates/boyko_rhi_vulkan/src/present/passes/particles.rs:763-780` |
| **R10 is live**: "a ParticleSortMode that permutes `p_render` may not carry motion vectors" — a boot `debug_assert` over `ParticleSortMode::motion_vectors_allowed` | `crates/boyko_app/src/gpu_scene/particle.rs:400-412`; `crates/boyko_render/src/particle_config.rs:193`, `:233-236` |
| TAA: camera-only reprojection from `gViewT` + the `MotionCam` pair; per-object MV **declared, deliberately not wired**; no responsive / stencil mask anywhere (grep `responsive` over the TAA config, pass and shader: zero); particles are drawn **inside** the TAA loop; "TAA ghosting until P3" is a recorded limitation; the LDR-`lit` coupling is recorded | `crates/boyko_rhi_vulkan/shaders/taa_resolve.comp.hlsl:1-32`; `crates/boyko_render/src/taa_config.rs:253-272`; `crates/boyko_render/src/taa_state.rs:24-32`; `docs/PARTICLES-PLAN.md:994`; `docs/TAA-PLAN.md:142-150`, `:315` |
| MSDF text / UI: premultiplied output with an in-shader premultiplied OVER for the border ring; **one instanced draw onto the swapchain after the composite, outside the frame graph** — never temporally filtered | `crates/boyko_render/shaders/ui_rect.fs.hlsl:1-6`, `:140-154`; `crates/boyko_rhi_vulkan/src/present/passes/present_blit.rs:185-200`; world-anchored UI exists as screen rects (`crates/boyko_ui/src/world/visibility.rs:1-20`) |
| The eDSL: one generic body over `FieldScalar`, instantiated as `f32` (oracle) and `Emit` (HLSL printer), no transpiler; "no atomics, no `groupshared`, no stores and no texture sampling" — skeletons are generator-owned `format!` templates; 17 `*_edsl_sync`/`*_spv_sync` tests; a `-D` belongs in the manifest only if it changes the interface | `crates/boyko_shaderdsl/src/lib.rs:1-30`; `crates/boyko_shaderdsl/src/bin/emit_particles.rs:28-45`; `crates/boyko_rhi_vulkan/tests/*_sync.rs` (17 files); `docs/SHADER-VARIANT-MANIFEST.md:1-24` |
| Aether `material` keys: `base, metallic, roughness, reflectance, emissive, flags, textures` — emits a builder fn only; v2 **parks** `material` "pending the shader-policy decision (own campaign)" | `docs/AETHER-LANG-PLAN.md:645-700`; `docs/aether-v2/CONSTRUCTS.md:275-277` |
| The accepted shape of a research + design pair | `docs/animation/ANIMATION-DESIGN-SPACE.md:1-27`, `:578-693` |

Two premises of the brief are therefore **false of the code**: "particles — blending today?" — yes,
two classes, sorted alpha and unsorted additive, into LDR `lit`; "MSDF text — already blended?" —
yes, premultiplied, but onto the swapchain *after* the composite and every AA pass, so it is not in
the scene's transparency order at all and cannot be depth-tested there.

## 1. Systems surveyed

Unreal Engine 5 (Nanite, translucency lighting, Substrate, experimental OIT, TSR) · Unity HDRP /
URP · Godot 4 · Bevy (`Transparent3d`, OIT) · Filament · three.js (the glTF reference) · The Forge /
Confetti (the only visibility-buffer engine with an in-VB transparency example) · id Tech 7 (Doom
Eternal frame study) · Activision (Kohler 2016, Drobot 2025) · CD Projekt (Cyberpunk 2077 talk) ·
the papers: Everitt 2001, Bavoil & Myers 2008, Enderton et al. 2010, Jansen & Bavoil 2010, McGuire &
Enderton 2011, Salvi et al. 2011, McGuire & Bavoil 2013, Salvi & Vaidyanathan 2014, McGuire & Mara
2016, Wyman & McGuire 2017, Münstermann et al. 2018, Vasilakis et al. 2020, Jakubowski 2024 ·
the Khronos glTF material extensions · nvpro-samples and the Khronos Vulkan samples (API cost
ground truth for a raw-FFI backend).

## 2. Where transparents leave a visibility buffer — every engine's answer

| Engine | The rule | Source |
|---|---|---|
| **UE5 Nanite** | "Nanite supports materials that have their Blend Mode set to **Opaque and Masked**"; an unsupported blend mode gets **the default material and an Output Log warning**; Nanite "runs in its own rendering pass that completely bypasses traditional draw calls" | [D] dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine |
| **UE5 authoring consequence** | translucent submeshes are **split off the asset** so the opaque part stays Nanite | [B] philkuzmicz.com/post/preparing-meshes-for-nanite-translucency |
| **Confetti Triangle VB (2018)** | "For transparent objects, we still have to use traditional Forward+ by sorting draw calls back-to-front before we execute them"; even **alpha-masked** geometry carries a 1-bit flag in the packed VB index and "requires a dedicated code path for each with its own ExecuteIndirect" | [B] diaryofagraphicsprogrammer.blogspot.com/2018/03/triangle-visibility-buffer.html |
| **The Forge Unit Test 15a** | *Visibility Buffer OIT*: "a per-pixel linked list of **triangle IDs** is holding layers of transparency. This occupies less memory and is more efficient than storing per-pixel information" — the ID-not-colour economy of a VB applied to an A-buffer; the node struct was not opened (**UNVERIFIED**) | [S] github.com/ConfettiFX/The-Forge README + `Examples_3/Visibility_Buffer` |
| **id Tech 7** | depth prepass (opaque only) → HZB → light/decal cull → opaque forward → particle sim → sky/volumetric froxels → **transparency pass (forward), "after the opaque geometry and when the light scattering data is available"** → UI → post; refraction = a smoothness-selected mip of a downsampled scene colour, no half-res transparency pass | [B] simoncoenen.com/blog/programming/graphics/DoomEternalStudy |
| **Unity HDRP** | an enumerated transparent stage: transparent depth prepass → volumetrics/fog/clouds → SSR → **transparent pre-refraction** → water lighting → **colour pyramid pre-refraction** → transparents → **low-resolution transparents** → combine → transparent postpass → transparent UI | [D] docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/rendering-execution-order.html |
| **Filament** | ONE colour pass **split at the first refractive draw by a sort-key range** (`getFirstRefractionCommand`), `generateMipmapSSR` on the opaque output, the refractive tail into the same colour+depth targets with `clearFlags = NONE` | [S] github.com/google/filament `filament/src/RendererUtils.cpp` (`refractionPass`) |
| **Bevy** | `Transparent3d` is a **sorted** phase (`FloatOrd(distance)`, ascending = back-to-front, **batching disallowed**); `Opaque3d`/`AlphaMask3d` are binned and batched | [S] github.com/bevyengine/bevy `crates/bevy_core_pipeline/src/core_3d/mod.rs` |

**The invariant across all seven:** opaque leaves through the deferred/VB path; transparents
rejoin as a **separate forward pass that consumes the opaque frame** (depth, lit colour, light lists,
shadow maps, volumetrics) and **never writes the geometry buffer**. The only counter-example (The
Forge 15a) builds a *second* per-pixel structure; nobody puts a blended fragment into the single
`(instance, primitive)` per pixel a VB is. The tree's own `vb_raster.fs.hlsl` makes the same
statement from the other side: one id, no discard, no `SV_Depth` [T].

## 3. Ordering — the spectrum, per engine

| Level | Who ships it | What it fixes / what it cannot |
|---|---|---|
| Per-object back-to-front (distance / AABB centre / sort priority) | Bevy (distance), Godot (AABB centre), UE (Translucency Sort Priority, default 0, positive = in front) [D] | fixes nothing inside an object; Godot "may exhibit sorting issues when transparent surfaces overlap" [D docs.godotengine.org/en/stable/tutorials/3d/standard_material_3d.html] |
| Transparent **depth prepass** (front layer writes depth) | HDRP (per-material prepass/postpass), Godot "Depth Pre-Pass" mode | improves intra-object order at one layer; **breaks batching in Forward+** ([S] godotengine/godot#103954) and sorts wrongly with additive blend ([S] #49389) |
| Two-sided sorted draw (back faces then front faces) | classic convex-glass idiom; Filament `doubleSided` + `transparency: twoPassesTwoSides` [D google.github.io/filament/Materials.md.html] | two draws per instance, zero memory; convex shells only |
| Per-pixel OIT (§4) | UE 5.1+ experimental per-pixel sorted OIT, DX12-only at introduction ([D] docs; still experimental through 5.4); Bevy PPLL; CoD AVBOIT 2025 | correct to capacity; each has a byte budget and an overflow policy |
| Sorting vs batching, stated by both sides | Bevy: sorted phase ⇒ no batching [S]; Kohler (Activision 2016): sorting "breaks apart material batches and sometimes even meshes" [B research.activision.com ATVI-TR-16-02] | the draw-level trade the design must price |

The tree's particle recorder is the honest in-tree version: "Neither order is 'correct' — two draws
cannot interleave two classes by depth, which is the standard two-bucket compromise this partition
buys" (`particles.rs:772-778`) [T].

## 4. Order-independent transparency — the families, with API requirements against THIS RHI

### 4.1 The algorithms

1. **Depth peeling** (Everitt 2001) — N geometry passes → N exact layers; **dual** peeling (Bavoil &
   Myers 2008) peels front and back with a **min-max depth buffer**, N layers in N/2+1 passes,
   **requires MIN/MAX blend ops** [D my.eng.utah.edu/~cs5610/handouts/DualDepthPeeling.pdf]. Khronos'
   sample exposes layers 0..7; overflow "skips backmost layers" without flicker [D
   docs.vulkan.org/samples/latest/samples/api/oit_depth_peeling/README.html].
2. **Per-pixel linked lists / A-buffer** — head-pointer image + append buffer,
   `InterlockedExchange`, `IncrementCounter`, `[earlydepthstencil]` to keep early-Z alive; a
   compute resolve sorts and blends. Khronos caps at `SORTED_FRAGMENT_MAX_COUNT = 16` and degrades
   past it [D docs.vulkan.org/…/oit_linked_lists/README.html]. Bevy: `sorted_fragment_max_count: 8`,
   `fragments_per_pixel_average: 4.0`, **no MSAA**, Blend/Premultiplied/Add only, manual depth test
   in the gather "because early-z fails in too many cases" [S bevy `oit/mod.rs`, PR #14876].
3. **Weighted Blended OIT** (McGuire & Bavoil, JCGT 2(2):122-141, 2013) — RT0 `RGBA16F` accum,
   cleared 0, blend `(ONE, ONE)`, writes `float4(c·a, a)·w`; RT1 `R8` revealage, cleared 1, blend
   `(ZERO, ONE_MINUS_SRC_ALPHA)`; resolve `accum.rgb / max(accum.a, 1e-5)` composited with
   `(ONE_MINUS_SRC_ALPHA, SRC_ALPHA)`; practical weight
   `max(min(1, max3(c)·a), a) · clamp(0.03 / (1e-5 + (z/200)^4), 1e-2, 3e3)`; assumes "~1-100
   surfaces, alpha 0.2-0.9", >8 bits per channel [D jcgt.org/published/0002/02/09; [B]
   casual-effects.blogspot.com/2014/03]. Failure modes: opacity → 1, large lighting-intensity
   differences between overlapping surfaces, depth-boost discontinuities at edges [B
   therealmjp.github.io/posts/weighted-blended-oit]. **nvpro measures 20 B/px constant** [S
   github.com/nvpro-samples/vk_order_independent_transparency].
4. **Moment-Based OIT** (Münstermann, Krumpen, Klein, Peters — PACMCGIT 1(1):7, 2018) — works on
   **log-transmittance (absorbance)**, so the depth-dependent function **accumulates additively**:
   pass 1 accumulates `b0 = Σ absorbance` and power moments `b_i = Σ absorbance · z^i` over all
   transparent fragments; pass 2 reconstructs each fragment's transmittance from the moments and
   accumulates its weighted colour **additively**; a composite applies `exp(-b0)`. 4/6/8 power
   moments or up to 4 trigonometric moments; **non-linearly quantised moments compress 32 → 16 bit
   "with no observable loss in quality"** [D momentsingraphics.de/I3D2018.html; the PDF exceeded the
   fetch limit — algorithm statement from the paper page and abstract].
   **[D-arith] Consequence for this RHI, missed by the lens:** every target MBOIT blends into —
   `b0`, the moment vector, the colour accumulator — uses the SAME `(ONE, ONE, ADD)` state, which is
   exactly what `Option<BlendState>` replicated onto all MRT slots produces. MBOIT therefore needs
   **neither `independentBlend` nor any new `BlendFactor`**; it needs two geometry passes and one
   fullscreen composite. **One caveat on the 16-bit figure (second pass):** the measured 3.5 ms
   configuration is "16-bit/channel" moments [B interplayoflight], which with hardware additive
   blending means `R16G16B16A16_SFLOAT` targets — a *precision* reduction the blend unit performs
   unchanged. The paper's *non-linearly quantised* moments (a transform applied to the stored
   moment vector so it fits UNORM16) cannot be applied to a running hardware sum by a per-fragment
   shader; whether the paper realises NLQM through a change of variable applied per fragment, or
   through a custom (non-hardware) accumulation, was **not confirmed** — the PDF exceeded the fetch
   limit. The fit claim above rests on fp16 SFLOAT additive only; NLQM's compatibility with a
   `(ONE, ONE, ADD)` pipeline is UNVERIFIED.
5. **Multi-Layer Alpha Blending** (Salvi & Vaidyanathan, I3D 2014) — single pass, **bounded**
   k-entry per-pixel array, adjacent-layer merge on overflow; "not fully OIT as the order the
   fragments arrive may make a difference"; needs ROV / interlock or a 64-bit atomic loop [D
   dl.acm.org/doi/10.1145/2556700.2556705; nvpro].
6. **Adaptive Transparency / Intel AOIT** (Salvi et al. HPG 2011) — ROV-based, documented
   DirectX 11/12 only [D The Forge README]; Vulkan equivalent = `VK_EXT_fragment_shader_interlock`.
7. **Stochastic transparency** (Enderton, Sintorn, Shirley, Luebke, I3D 2010 / TVCG 17(8)) — a
   random sub-pixel sample subset of size ∝ α; correct **in expectation**, one pass, fixed memory,
   noise; needs MSAA coverage control [D luebke.us/publications/StochasticTransparency_I3D2010.pdf].
8. **Hashed alpha testing** (Wyman & McGuire, I3D 2017) — the alpha-test threshold is a **hash of
   object-space position** (anisotropically scaled to pixel footprint), giving stable noise and
   fixing "geometry disappears with distance"; "when combined with temporal anti-aliasing (TAA),
   hashed alpha testing allows TAA to integrate transparency over time, providing true transparency
   values" [D cwyman.org/papers/i3d17_hashedAlpha.pdf]. Shipped as Godot "Alpha Hash" (recommended
   for hair) [D] and UE "Dither Temporal AA" [B cesium.com/blog/2022/10/20]. **Stays on the
   opaque/VB rail** — masked is Nanite-legal [D].
9. **Phenomenological transparency** (McGuire & Mara, I3D 2016) — coloured transmission, translucent
   coloured shadows, caustics, partial coverage, diffusion, refraction, "using order-independent
   draw calls and low bandwidth" [D research.nvidia.com/publication/2016-02_…].
10. **VB-native OIT** (The Forge 15a) — PPLL of triangle IDs; resolve re-runs the material per
    layer [S README; node struct UNVERIFIED].
11. **Exact software OIT** (LucidRaster, Jakubowski, arXiv:2405.13364, 2024) — "only about 3× slower
    than hardware alpha blending", best at high triangle density / depth complexity [D
    arxiv.org/abs/2405.13364].
12. **Adaptive Voxel-Based OIT** (Drobot, SIGGRAPH Advances 2025) — built for Call of Duty "after
    existing algorithms failed their accuracy + performance bar" [B research.activision.com; the
    5.8 MB deck was **not extractable** this session — **UNVERIFIED** internals].
13. **Cyberpunk 2077** (Sikachev et al., SIGGRAPH 2021 Talks) — decoupled particle lighting,
    distortion, "parallel slab" [B dl.acm.org/doi/10.1145/3450623.3464629; body **UNVERIFIED**].

### 4.2 Fit against the RHI at `ed0bed45` — the table that decides rung 7

| Scheme | Geometry passes | Blend states needed | Needs `independentBlend` | Needs FS atomics / ROV | Bounded memory | Fits today |
|---|---|---|---|---|---|---|
| Sorted alpha (per object) | 1 | `PREMULTIPLIED_ALPHA` (exists) | no | no | 0 B | **yes** |
| Hashed alpha (opaque rail) | 1 | none (`discard`) | no | no | 0 B | **yes** (a `discard` variant + UV export) |
| Additive (particles) | 1 | `ADDITIVE` (exists) | no | no | 0 B | **shipped** |
| WBOIT | 1 | `(ONE,ONE)` + `(ZERO,1−SRC_A)` on two targets | **yes** | no | 9-20 B/px | **no** — VUID-VkPipelineColorBlendStateCreateInfo-pAttachments-00605 requires identical `pAttachments` without it [D khronos.org registry] |
| **MBOIT** (4 power moments, fp16) | **2** + composite | **`ADDITIVE` on every target** | **no** [D-arith] | no | 12-20 B/px | **yes** with two new targets and an RGBA16F accumulator |
| MLAB k=2..8 | 1 | custom (UAV) | — | **interlock / int64** (not enabled, not queried) | 16-64 B/px | no |
| PPLL | 1 + resolve | custom (UAV) | — | atomics in FS; eDSL cannot author, template can | unbounded (capped) | no — Principle 5 |
| Depth peeling / dual | N or N/2+1 | dual needs `MIN`/`MAX` ops (`BlendOp::{Add}` only) | no | no | N layers | no — geometry passes over a VB scene |
| Stochastic (MSAA) | 1 | none | no | no | 0 B | no — `SAMPLE_COUNT_1`, `alpha_to_coverage = FALSE` literals |

### 4.3 Published costs (one measurement set — 1080p, RTX 3080 laptop, Kostas Anagnostou 2022) [B]

| Scheme | ms | Memory | Note |
|---|---|---|---|
| hardware alpha blend (sorted) | **1.37** | 0 | the baseline every OIT is measured against |
| PPLL 8 nodes × 12 B | 4.20 build + 1.32 resolve | ~200 MB | "misses some transparent surfaces… some flickering" |
| ROV custom blend | 3.9 | — | vendor-gated |
| MLAB 2 / 4 / 8 nodes (8-B node) | 4.4 / 6.8 / 12 | ~35 / 68 / 134 MB | merge order-dependent |
| MBOIT 4 / 8 moments fp32 | 8.1 / 13.1 | — | |
| **MBOIT 4 / 8 moments fp16** | **3.5 / 6.2** | **~20 / ~37 MB** | the NLQM result: precision before resolution |
| MBOIT 4 / 8 moments at 960×560 | 2.4 / 4.1 | 5.4 MB | resolution reduction buys less than precision reduction |

Sources: interplayoflight.wordpress.com/2022/06/25 (part 1), 2022/07/02 (part 2), 2022/07/10
(endgame). The author's own verdict is "case by case", no universal winner [B]. **Treat every
figure as relative**: one scene, one rig, and cost scales with depth complexity × resolution by
construction. Cross-check of the memory column against arithmetic at 1080p (2,073,600 px)
[D-arith]: MBOIT-4 fp16 at 10 B/px = 20.7 MB ✔; MLAB-2 at 16 B/px = 33.2 MB ✔; PPLL at 100 B/px =
207 MB ✔ — the published bytes-per-pixel and the published megabytes agree, so the table is
internally consistent and can be scaled to 1440p by 1.778×.

## 5. The material model — glTF is the specification, and it is small

All five Khronos extensions were opened as their `README.md` on `github.com/KhronosGroup/glTF`
(`extensions/2.0/Khronos/<name>/README.md`) [D]:

- **`KHR_materials_transmission`** — `transmissionFactor` (default 0), `transmissionTexture.r`;
  the BSDF: `dielectric_brdf = fresnel_mix(ior = 1.5, base = mix(diffuse_brdf(baseColor),
  specular_btdf(α = roughness²) · baseColor, transmission), layer = specular_brdf(α = roughness²))`;
  Trowbridge-Reitz BTDF, transmission half-vector sampling; scope "infinitely-thin materials with no
  refraction, scattering, or dispersion". **`alphaMode` SHOULD stay `OPAQUE` — "alpha-as-coverage
  is NOT for physically-based transparency."** Alpha = does the surface exist; transmission = does
  light pass through an existing surface.
- **`KHR_materials_volume`** — `thicknessFactor` (default 0, mesh space), `thicknessTexture.g`,
  `attenuationDistance` (default +∞), `attenuationColor` (default 1); Beer-Lambert
  **`T(x) = c^(x/d)`**, `σ_t = −log(c)/d`; `thicknessFactor ≠ 0` switches thin → volumetric and
  **requires a manifold (closed) mesh**.
- **`KHR_materials_ior`** — default 1.5; **`f0 = ((ior−1)/(ior+1))²`**; `ior = 0` = Fresnel 1.0
  compat mode.
- **`KHR_materials_specular`** — `f0 = min(0.04 · specularColor, 1) · specular`, `f90 = specular`.
- **`KHR_materials_dispersion`** — `dispersion = 20 / Abbe` (default 0), Cauchy equation; real-time
  approximation = **three IORs**, base = green, `halfSpread = (ior − 1) · 0.025 · dispersion`;
  requires `KHR_materials_volume`.

Reference implementations opened [S]:

- **three.js** `transmission_pars_fragment.glsl.js`: `getVolumeTransmissionRay` = `refract(−v, n,
  1/ior) · thickness · modelScale` (thickness in **mesh space**);
  **`applyIorToRoughness(r, ior) = r · clamp(2·ior − 2, 0, 1)`**; sample LOD =
  `log2(size.x) · applyIorToRoughness(...)`; `volumeAttenuation = exp(−log(c)/d · dist)`;
  `(1 − F) · transmittance · transmitted` with `EnvironmentBRDF`; dispersion = three samples.
- **Filament** `surface_light_indirect.fs`: `refractionSolidSphere` / `refractionSolidBox`
  (`ray.direction = r` — the exit ray is the **view ray**: the parallel-exit approximation in code)
  / `refractionThinSphere`; `Ft *= 1.0 − E; Ft *= diffuseColor` (E = the specular DFG term — the
  transmitted lobe is the energy the reflection lobe did not take); `T = saturate(exp(−absorption ·
  d))`; screen-space LOD `max(0, (2·log2(perceptualRoughness) + refractionLodOffset) ·
  invLog2sqrt5)`; thin-film series `E *= 1 + transmission · (1 − E.g)/(1 + E.g)`. `Materials.md`:
  `refractionMode ∈ {none, cubemap, screenspace}`, `refractionType ∈ {solid, thin}`; documented
  limits — screen-space assumes parallel exit, cubemap assumes rays "emerge from the centre of the
  object"; **dispersion is not used with `thin`** [D].
- **Unity HDRP** refraction models: **Sphere** (documented artefact: the scene can appear
  **upside-down** because exit rays cross), **Box** (exit approximated by a parallel plane), **Thin**;
  a thickness map [D github.com/Unity-Technologies/Graphics …/refraction-models.md].
- **UE5**: Thin Translucent shading model (one pass, air→glass→air) [D]; Substrate
  `TranslucentColoredTransmittance / TranslucentGreyTransmittance / ColoredTransmittanceOnly`, rough
  refraction with blur from roughness × thickness, opt-in project setting [D].

## 6. Refraction — the same three stages everywhere

Opaque pass → snapshot the scene colour → **roughness-blurred mip chain** → refractive draws sample
it at a roughness-derived LOD. Filament (sort-key split of one pass), three.js (transmission RT +
mips), HDRP (colour pyramid), UE (scene colour), Doom Eternal (smoothness-selected mip), Godot
(back-buffer copy + `filter_linear_mipmap` + `textureLod(screen_texture, uv, blur)`) [S][D][B].

The ordering paradox, stated by HDRP [D Custom-Pass-buffers-pyramids.md]: "transparents behind a
refractive object need to be part of the color pyramid … but transparents in front of refractive
objects need to not be part of the pyramid to avoid leaking"; their fix is a **second transparent
sub-pass** (pre-refraction), a **depth-buffer copy**, and a per-material "Sort with Refractive" flag
which force-enables transparent motion vectors [B discussions.unity.com … secondary]. Godot's failure
of this class: a transparent material does not render when a refractive one is behind it ([S]
godotengine/godot#60232); its lazy allocation of the refraction textures **stutters on first use**
([S] #101554) — Principle 5 already forbids that shape here.

## 7. Lighting, shadows, reflections for transparents — a second lighting implementation

- **UE** — two translucency lighting modes: **cascaded volume textures** around the frustum
  (`r.TranslucencyLightingVolumeDim` default 64, inner 1500, outer 5000) and **Surface
  ForwardShading** — "the most expensive translucency lighting method as each light's contribution
  is computed per-pixel"; SSR on translucency requires the per-pixel mode; diffuse GI from the
  Indirect Lighting Cache is **one sample at the object bounds centre**; self-shadow via **Fourier
  Opacity Maps**; **coloured translucent shadows need a Static light (Lightmass); Lumen does not
  support translucent shadow colour**; Lumen reflections on translucency render **only the
  frontmost layer** (`r.Lumen.TranslucencyReflections.FrontLayer.Enable`), others demoted to the
  radiance cache by a depth threshold [D lit-translucency page; using-colored-translucent-shadows
  page; indxzero cvar wiki].
- **Doom Eternal / HDRP** — reuse the clustered/froxel lists and volumetrics built for opaque [B][D].
- **Godot** — alpha-blended materials "don't cast shadows", "don't appear in any reflections", get
  no SSR or sharp SDFGI reflections [D standard_material_3d]; SSR on transparents is a standing
  proposal (#7274) [S].
- **Papers** — Fourier Opacity Mapping (Jansen & Bavoil, I3D 2010): Fourier-series transmittance
  along light rays, artefact-free for smooth opacity (smoke, hair) [D
  dl.acm.org/doi/10.1145/1730804.1730831]; Colored Stochastic Shadow Maps (McGuire & Enderton, I3D
  2011) [D research.nvidia.com …/CSSM.pdf].
- **Distance fields** — UE's mesh distance fields are opaque/masked-shaped (two-sided DF generation
  for masked foliage, shadows "never fully opaque") [D]. **No engine was found that places a
  transparent occluder into an SDF/brick structure with a transmittance channel** — a transmissive
  SDF shadow here would be original design, not a port.

## 8. TAA — the hard constraint, three shipped contracts

- The physical limit: **one motion vector per pixel**; a blended surface writes no depth, so history
  reprojection has nothing to key on. Bevy: "TAA also does not work well with alpha-blended meshes,
  as it requires depth writing to determine motion"; particles must write MVs or render **after** TAA
  [D docs.rs/bevy_anti_alias/…/TemporalAntiAliasing]. URP: "does not support motion vectors for
  transparent materials" [D]. HDRP: per-material opt-in "Transparent Writes Motion Vectors" [D].
- **Exclude** — UE reserves a stencil bit "Temporal AA mask for translucent object" (Responsive AA)
  [B ikrima.dev — secondary]; Epic's TSR FAQ: "TSR does not need translucent materials to use the
  Responsive AA setting", which under TAA "prevented TAA from losing as much detail with VFX" [D].
- **Write MVs where opaque enough / add a per-pixel accumulation factor / render after TAA** — the
  last "is not recommended… It can jitter at the edges because it's compared against a jittered
  depth buffer" [B elopezr.com/temporal-aa-and-the-quest-for-the-holy-trail]. UE's after-motion-blur
  translucency pass **disables the depth test** for exactly this reason [D
  MaterialTranslucencyPass].
- The tree already chose *before TAA* for particles and already carries the sort ⇔ MV exclusion as
  a live assert [T §0].

## 9. Particles, low-resolution tiers, text

- **Two-bucket compromise** — shipped here and named [T]; Volition's Agents of Mayhem modified
  WBOIT to serve "additive as well as non-additive alpha" in one accumulation [B GDC 2018 abstract;
  the PDF exceeded the fetch limit — weight function **UNVERIFIED**].
- **Low-resolution transparents** — HDRP half-res tier + upsample + "Low Res Transparency Min
  Threshold" [D HDRP-Asset.md]; Doom Eternal uses the colour mip chain instead of a half-res pass
  [B]; Kohler 2016 moves particle layers to compute-rasterised "Compute Sprites" [B].
- **Text / UI** — every engine composites UI after post; the tree does likewise [T]. World-space
  text occluded by scene depth is a scene-transparency question, not a UI one.

## 10. Pitfalls register (each with the evidence and the tree fact it lands on)

| # | Pitfall | Evidence | Lands on |
|---|---|---|---|
| P1 | Assuming the VB can carry a blended fragment "later" | Nanite refuses the blend mode [D]; Confetti falls back to Forward+ [B]; one id per pixel, no discard [T `vb_raster.fs.hlsl`] | the exit must be at the **gather**, not in a shader |
| P2 | Treating cutout as a special case of blend | Nanite: Masked is first-class; Confetti: a flag bit + its own indirect path [B] | masked needs its own bucket **and** its own raster/caster pipelines on every path |
| P3 | Sorting once and calling transparency solved | Godot's docs concede intra-object failure [D]; the tree's own two-bucket admission [T] | the sort key's resolution and the two-sided draw are the v1 mitigations; OIT is a rung |
| P4 | WBOIT's weight is content-calibrated and fails silently | "~1-100 surfaces, alpha 0.2-0.9" [D]; MJP's failure list [B] | a demo scene is green; shipped content is not |
| P5 | Bounded OIT hides its overflow | PPLL caps at 16 and degrades [D]; MLAB merges silently [D]; the measured PPLL "misses some transparent surfaces" [B] | the "green from emptiness" class this repo catalogues — an overflow **counter** a gate reads, not a clamp |
| P6 | Two-target OIT on a single-blend-state RHI | VUID-…-pAttachments-00605 [D]; `from_fn` replication [T] | WBOIT blocked; MBOIT not (all additive) |
| P7 | TAA as an ordering problem when it is a motion-vector problem | Bevy/URP/HDRP/UE all state it [D]; camera-only reprojection [T] | transparents inherit the background's motion → trail |
| P8 | Coupling sorting and motion vectors without noticing | R10 live assert [T]; Godot precedent | any sorted transparent set has the same hazard |
| P9 | Straight alpha under mip generation | NVIDIA "To Pre or Not To Pre" [D developer.nvidia.com/content/alpha-blending-pre-or-not-pre]; the UI's own reason [T `enums.rs:887-889`] | albedo of blend materials must be premultiplied **before** the T2 blit chain |
| P10 | Compositing physically-based transmission into display-referred 8-bit | `lit` is post-OETF [T]; PARTICLES-PLAN F2 [T]; WBOIT needs >8 bpc [D] | Beer-Lambert/Fresnel in LDR is a labelled approximation; HDR `lit` is a separate decision |
| P11 | Moving `lit` to HDR silently breaks the additive commutativity proof | `enums.rs:938-944` [T] | the proof must be rewritten with the HDR move |
| P12 | Refraction cannot see what has not been drawn | HDRP's paradox [D]; Godot #60232 [S] | define **which** scene colour the chain holds |
| P13 | Lazy allocation of the refraction chain | Godot #101554 [S] | Principle 5: preallocate at boot |
| P14 | The approximating shape leaks | Unity upside-down sphere [D]; Filament centre-emergence [D] | thin-only v1 avoids every shape choice |
| P15 | Thickness has no physical source | `KHR_materials_volume` manifold requirement [D]; the GLB loader/meshlet build make no manifoldness claim (UNVERIFIED) | volume is a bake refusal or a warn — owner call |
| P16 | Confusing coverage with transmission | glTF: `alphaMode` stays `OPAQUE` [D] | a transmissive material is routed by `transmission > 0`, not by alpha |
| P17 | Forgetting the Deferred depth horizon | "Deferred particles disappear at 64 units" [T] | a transparent mesh on Deferred inherits it; the constant moves at two sites |
| P18 | Losing early-Z by writing `SV_Depth` or appending to a UAV | the Deferred particle cost [T]; Bevy's manual depth test [S] | the reverse-Z paths keep early-Z; Deferred pays per fragment |
| P19 | Transparents fall out of every screen-space effect | Godot's list [D]; Bevy OIT "no reflections / no depth write" [S] | an explicit **participation matrix** (SSAO, DDGI, SDF shadows, HZB, TAA) |
| P20 | Only the front layer gets good reflections | UE Lumen front-layer [D] | promise front-layer at most |
| P21 | Translucent shadow colour is the first casualty | UE: Static light only, no Lumen [D] | grey first; colour needs one new `BlendFactor` |
| P22 | A transparent pass per render path | four paths, boot-frozen [T]; the particle recorder is path-agnostic [T] | one recorder, per-path targets/compare op |
| P23 | A declared consumer without its producer | the W4 hole [T `render_path_config.rs:96-118`]; the cull is VB-only by code and test [T `render_path_config.rs:1224`, `:3385-3406`] | a translucent pass must **read the same three-term `use_clusters` gate** every lit producer reads (flat-table fallback), never assume a cull; the first design draft's "arm the cull on every path" was itself a W4-class error, caught by the critique (§14) |
| P24 | Declaration order ≠ intended order | "declaration order is execution order, and no barrier separates …" [T `graph_bridge.rs:1651`]; `compile()` derives barriers from the declared access sequence | insert in all three declarators and recorders |
| P25 | The variant explosion escaping the manifest | `SHADER-VARIANT-MANIFEST.md:10-13` [T] | count the axes before the first `-D` |
| P26 | Hand-writing what the eDSL can author | `emit_particles.rs:34-40` [T] | leaf bodies in the eDSL; skeletons as generator templates |
| P27 | Reaching for depth peeling in a VB engine | N/2+1 geometry passes; `MIN/MAX` absent [T] | rejected outright |
| P28 | Choosing a technique from a demo-scene benchmark | "case by case" [B]; one rig, one scene | in-engine re-measure before the OIT rung is armed |
| P29 | Downsampling the wrong axis | fp32→fp16 = 2.3×; half-res = 1.45× [B] | precision before resolution |
| P30 | A material datum with no reader mistaken for a live feature | `base_color.w` has zero shader readers [T] | the cutoff field becomes live only with the masked variant |
| P31 | A "responsive" mask that does not exist | zero `responsive` hits [T] | the mask is a new target and a TAA-plan change, not a flag |
| P32 | Two authorities for the route (component vs material) | `MATERIAL_FLAG_TEXTURED` is re-derived at one upload boundary [T `material.rs:112-121`] | one authority derives the marker; a debug gate checks the other |

## 11. The ladder as dependency facts (the design chooses the rungs)

- **L0 — the seam**: a forward transparent pass at the `particle_draw` slot on every path,
  reading the path's depth as a read-only attachment (D7's table verbatim), `lit` as a blended colour
  attachment (`PREMULTIPLIED_ALPHA` exists), froxel lists + `shadow_apply` + `ddgi_probe_sample`
  (all exist as includes). Needs: a gather that **excludes** the instances from the opaque batches
  and a second scratch (the `CsmCasterScratch` shape); the three-term `use_clusters` gate with the
  flat fallback in the FS — no cull is armed by the pass (P23, corrected); an entry in all three
  declarators (P24).
- **L1 — sorting**: a per-instance key lane + a stable CPU sort in the gather (the particle key
  leaf's `f32` instantiation), draw runs by (key, mesh); the two-sided sorted draw.
- **L2 — masked/hashed**: a `-D MASKED` raster variant on each path's raster and caster pipelines,
  UV + object-position export from the VS, the hash leaf; `base_color.w` gains a reader.
- **L3 — sampled depth**: D13's push descriptor (soft particles, depth-fade, refraction depth
  test) — a shared prerequisite of L4 and of `-D SOFT`.
- **L4 — scene-colour chain**: `add_image_mipped` copy of `lit` + a compute downsample chain
  (the HZB build shape with an average kernel); preallocated (P13).
- **L5 — thin transmission** (`KHR_materials_transmission` math) over L0 + L4 + the material
  extension table.
- **L6 — TAA mask**: a coverage target accumulated by the same premultiplied state; a TAA-plan
  change.
- **L7 — OIT**: MBOIT (the only family the RHI expresses today, §4.2); two targets + accumulator;
  an overflow observable (P5).
- **L8 — volume / Beer-Lambert / thickness** (manifold contract, P15) and **dispersion** (3×
  samples; not with thin, per Filament).
- **L9 — translucent shadows**: a transmittance lane per shadow source; grey with today's
  factors, colour with one new `BlendFactor`; SDF shadows stay opaque-only (§7).
- **L10 — reflections on transparents**: front-layer SSR when an SSR pass exists; until then the
  opaque path's analytic sky/EnvBRDF term.
- **L11 — HDR scene colour**: the coupled decision (TAA pre-tonemap, the additive proof, every
  golden) — required by nothing above for correctness of the *seam*, required for the *physics* of
  L5/L8 to mean what they say (P10).

## 12. Sources (by topic)

**VB / engines.** Epic Nanite doc · Epic lit-translucency, using-transparency, colored-translucent-
shadows, Substrate overview, `MaterialTranslucencyPass` API, TSR FAQ · Confetti Triangle VB blog ·
The Forge README + `Examples_3/Visibility_Buffer` · Doom Eternal frame study (simoncoenen.com) ·
HDRP rendering-execution-order, Custom-Pass-buffers-pyramids, HDRP-Asset.md, refraction-models.md ·
URP motion-vectors · Godot `standard_material_3d`, issues #60232, #101554, #103954, #49389, proposal
#7274 · Bevy `core_3d/mod.rs`, `oit/mod.rs`, PRs #14876, #21831, `TemporalAntiAliasing` docs ·
Filament `RendererUtils.cpp`, `surface_light_indirect.fs`, `Materials.md.html` · three.js
`transmission_pars_fragment.glsl.js` · Activision ATVI-TR-16-02 and AVBOIT 2025 (abstracts) ·
Cyberpunk 2077 SIGGRAPH 2021 (abstract).

**Papers.** Everitt 2001 · Bavoil & Myers 2008 · Enderton et al. 2010 · Jansen & Bavoil 2010 ·
McGuire & Enderton 2011 · Salvi et al. 2011 · McGuire & Bavoil 2013 (+ casual-effects blog, MJP,
LearnOpenGL) · Salvi & Vaidyanathan 2014 · Wyman 2016 (SLAB) · McGuire & Mara 2016 · Wyman & McGuire
2017 · Münstermann et al. 2018 · Moment Transparency HPG 2018 · Vasilakis et al. 2020 STAR (PDF not
opened — **UNVERIFIED**) · Burns & Hunt 2013 · Jakubowski 2024.

**API.** Khronos `VkPipelineColorBlendStateCreateInfo` (VUID 00605) · nvpro
`vk_order_independent_transparency` (bytes per pixel, extension list) · Khronos samples
`oit_linked_lists`, `oit_depth_peeling` · glTF `KHR_materials_{transmission,volume,ior,specular,
dispersion}` · NVIDIA "Alpha Blending: To Pre or Not To Pre".

## 13. Verification notes — corrections to the lenses, and what was NOT confirmed

- **Commit.** The lenses cite `128233be`; HEAD at the time of this record is **`ed0bed45`** (the
  animation-docs commit on top of it). Every line here was re-opened at `ed0bed45`.
- **Sort-key site.** A lens cited `compute.rs:5972` under `rhi_impl/`; there is no such file. The
  sort modules are registered at `crates/boyko_rhi_vulkan/src/compute.rs:5825-5854` and the key's
  bin count is a generator input at `crates/boyko_shaderdsl/src/bin/emit_particles.rs:164-168`.
- **Capability-query idiom.** Not `device.rs:2809+`; the `supports_*` functions are at
  `crates/boyko_rhi_vulkan/src/device.rs:2816` (dynamic rendering), `:2849`, `:2879`, `:2969` (ray
  query), each a `VkPhysicalDeviceFeatures2` chain.
- **`BindGroupEntry`** lives in `crates/boyko_rhi/src/device.rs:352`; `SampledImageAtGeneral` at
  `:401` (a lens placed it in `descriptor.rs`).
- **`MESH_DEPTH_T_MAX`** is at `gbuffer_mrt.fs.hlsl:114` (the particle FS comment says `:113`);
  its host mirror is `crates/boyko_rhi_vulkan/src/compute.rs:3250`, not `gbuffer_depth.rs`.
- **Second-pass corrections (the anchors the first pass had wrong or drifted):**
  - the Deferred-depth row of §0 cited `gbuffer_depth.rs:1-16` for the 64-unit encode; that module
    documents `T_MAX = 10.0` (`GBUFFER_T_MAX`, `:50`) — the marcher/ortho normalizer — and its
    opening doc still says the perspective fragment divides by `T_MAX`, which
    `gbuffer_mrt.fs.hlsl:99-114` no longer does (it divides by `MESH_DEPTH_T_MAX = 64.0` under
    perspective and writes `position.z` under ortho). **A stale doc comment in the tree**, worth
    its own one-line repair; not touched by this record;
  - "declaration order is execution order" is `graph_bridge.rs:1651`, not `:2601-2606` (that span
    is the Forward declarator's cross-frame `cascade`/`atlas` seed, audit B-003);
  - the VB froxel-cull "default off" row is `docs/FEATURE_MAP.md:127`, not `:124` (`:124` is the
    VB pass-chain row; `:125` is the pre-light union with the `ssr` bit);
  - `PARTICLES-PLAN.md`'s inverted-key statement is the LANDED blockquote at `:527` (the design
    cited `:525`, the D10 opening paragraph); the `deferred_path`-predicate pipeline pick is
    `:1517`, the `-D DEPTH_LINEAR` VS+FS deviation `:1515`, the plumbing list `:1509`;
  - `blend: Some` has three hits outside `boyko_rhi/`, one of which is `boyko_demo`'s wgpu
    backend (`crates/boyko_demo/src/render/mod.rs:205`) — the "three pipelines" count in §0 is
    UI + the two particle classes, on this RHI.
- **MBOIT and `independentBlend`.** The oit-and-sorting lens said both WBOIT and MBOIT need it;
  only WBOIT does (§4.1 item 4, §4.2). This changes the rung-7 choice.
- **`lit`'s alpha channel** — what the opaque producers store there was **not** read; the design
  does not repurpose it.
- **Nanite's OIT DX12-only restriction** — search-derived; Epic's OIT page returned no body.
- **The Forge 15a node struct**, **Volition's modified weight**, **AVBOIT internals**, **Cyberpunk's
  parallel slab**, **the Vasilakis STAR taxonomy** — abstracts only; all **UNVERIFIED**.
- **Mesh manifoldness** in the GLB loader / meshlet build — not checked.
- **Soft particles** — `-D SOFT` is designed, not landed; no depth-fade exists (P2 item 4) [T].
- **CSM cascade resolution** — not read; translucent-shadow memory is stated per cascade texel.
- No measurement of any kind was taken in this session.

## 14. Critique log — pass 1 (2026-09-10)

The `architecture-critic`'s first pass over this record and its companion design. Findings that
land on this record are listed; the design's own log (`TRANSPARENCY-DESIGN-SPACE.md` §15) carries
the other fifteen. Every `file:line` was re-opened by the architect on `feat/multi-paradigm-render`
(working tree of 2026-09-10).

| # | Finding (short) | Action |
|---|---|---|
| NB11 | §0's "under VB the froxel cull is default off" is true but incomplete in the direction the design misread: on Deferred and plain Forward the cull is structurally absent (placeholder grid / no pass) and on ForwardPlus its dims are pinned to 0 by `sync_cluster_light_gate`; one sentence here would have stopped D8's premise | **Fixed.** The §0 row now says so, with `render_path_config.rs:1224` / `:3385-3406`, `light.rs:1011-1015`, `graph_bridge.rs:2745-2750`, `deferred_pbr.hlsl:1246-1249`, `forward_opaque.fs.hlsl:308-390` and the per-frame `clusters_enabled` word (`light_table.hlsli:341`, `light_policy.rs:190-193`). P23 and L0 are corrected in the same direction — the record itself had carried "a translucent pass under VB must arm the froxel cull", which is the sentence the design then generalised to every path |
| NB11 (second half) | The `gbuffer_depth.rs:1-16` stale-doc observation is correct and worth its one-line repair | **Confirmed, still unrepaired.** `gbuffer_depth.rs:7-8` says the perspective fragment divides by `T_MAX = 10.0`; `gbuffer_mrt.fs.hlsl:114` divides by `MESH_DEPTH_T_MAX = 64.0`. The architect edits no code; the repair belongs to whichever pass next touches that module (§13) |
| B2 (the values, not the record's own claim) | The `VkBlendFactor` values the design proposed were wrong | **The record's §0 row is correct** (`Zero=0, One=1, SrcAlpha=6, OneMinusSrcAlpha=7`); the design's growth was the error. For the record: `vulkan_core.h:2401-2412` of the installed SDK 1.4.350.0 gives `SRC_COLOR = 2, ONE_MINUS_SRC_COLOR = 3, DST_COLOR = 4, ONE_MINUS_DST_COLOR = 5, DST_ALPHA = 8, ONE_MINUS_DST_ALPHA = 9` [D] |
| NB2 (carried) | `R32_SFLOAT` blend support may not be mandatory | **UNVERIFIED after three fetches** of the spec's formats chapter (none returned the required-support tables); the design probes it at boot (TK-13) so nothing rests on the answer |
| B3 (the range) | The particle key's range is `[0.125, 4096]` (15 octaves), 4.15 % per 8-bit bin | **Confirmed** at `emit_particles.rs:184-192` and recorded here so the next design pass does not re-derive a `far/near` figure the leaf does not use |
