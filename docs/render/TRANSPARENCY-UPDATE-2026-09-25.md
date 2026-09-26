# Transparency — the 2026-09-25 update (a delta over the 09-10 pair)

> Status: architect's delta, 2026-09-25, written against trunk **`6394bc5e`** (worktree
> `D:/wt/docs`, branch `u/research-0925`, identical to the trunk). It amends, and does not replace,
> the 09-10 pair: the survey [`TRANSPARENCY-RESEARCH.md`](TRANSPARENCY-RESEARCH.md) and the design
> [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md), both written at `ed0bed45`.
> Anything this document does not mention **stands as the 09-10 pair wrote it**. §1 restates what
> they decided, §2 lists what still holds, §3–§5 describe what changed and why, and §7 lists the
> decisions this pass takes.
>
> **Provenance.** Tags follow the 09-10 pair:
> - **[S]** source code read.
> - **[D]** official docs, spec or paper.
> - **[B]** blog, talk or third-party writing.
> - **[T]** a file in this tree, re-opened **by content at `6394bc5e`** by this pass.
> - **[D-arith]** arithmetic on tagged facts.
> - **estimate** a number with no source behind it.
>
> **Web access.** The session's web-search budget was exhausted before this pass ran, so every
> external source was reached by opening a known URL. §5.1 names the sources this pass opened
> itself, including the ones revision 2 added. §5.2 lists what is carried from the 09-25 research
> lens without re-opening.
>
> Nothing was timed and no `cargo` command was run. Every millisecond figure is one of two things:
> a published figure quoted with its rig, or a labelled estimate.
>
> This directory is not in `GATED_DOCS` (`tests/internal_docs_anchors.rs:349`), so no anchor
> below is machine-checked. Each `path:line` is "true at `6394bc5e`". Sibling documents dated the
> same day, which are still being revised, are cited by decision ID and section, not by line.
>
> **Revision 2 (2026-09-25).** This revision answers critique pass 1 (1 critical, 10 important,
> 8 optional remarks, 6 open questions). All 19 remarks were confirmed against the tree and
> adopted. The biggest changes:
> - D-U7 no longer premultiplies stored texels. Albedo stays straight alpha with a load-time
>   dilation, and premultiplication happens at the shader output.
> - D-U3 is decided per path.
> - D-U12 defers fog on transparents to the volumetrics design.
> - The masked caster gains a second depth family.
> - V9 is closed as a technical fork.
>
> §15 is the review log.

## 0. The delta in one screen

**The design survives.** Everything below is additive to it.
- No transparency code has landed since 09-10. Every tree fact the pair rests on still holds, at
  drifted line numbers (§2).
- The exit at the gather, the seam at `particle_draw`, the three-term `use_clusters` gate and the
  cold `MaterialXGpu` table are unchanged.

**Six changes on the tree touch the design:**
1. **The mesh gather is now a chained two-query split.** The exit filter must sit on both queries.
   A translucent whose material is still streaming needs a policy (§3.1, D-U1).
2. **A glb decoder landed, and it drops `alphaMode`, `alphaCutoff`, `doubleSided` and every
   `KHR_materials_*` extension** (`crates/boyko_render/src/loaders/glb.rs:945-964`). Imported
   content therefore cannot route to `Translucent` or `Masked` at all today. No engine code turns a
   `GlbScene` into entities either: two examples do it by hand (§3.2, D-U2).
3. **Device features are tables, and a SPIR-V capability census gates them.** `discard` lowers to
   the now-required `DemoteToHelperInvocation`, so R2's masked FS needs no new device row. An FS
   storage write needs `fragmentStoresAndAtomics`, which no capability declares, so the census
   cannot see it (§3.3).
4. **The eDSL gained a `Cf` unsigned-integer vocabulary with real host arms, plus a bit-exact PCG
   hash** (`crates/boyko_shaderdsl/src/particle.rs:236`). This reshapes the sort key and the
   hashed-alpha leaf (§3.4, D-U4, D-U5).
5. **The caster gather has no material dimension**
   (`crates/boyko_render/src/csm_caster.rs:251-253`). Masked casters, which the 09-10 R2 priced
   as "+1 caster variant", need their own material-carrying query and a masked variant of **two**
   depth-shader families: cascades and spots share one, and point lights use another (§3.5, D-U6).
6. **The reflections design adopted HDR `lit` (its R4a) and a `lit_prev` copy.** Transparency's
   R11 is that same decision. R4's scene-colour chain can share the copy (§3.6, D-U8).

**One hybrid gap the pair never stated.** SDF surfaces never reach the depth buffer that the
transparent slot tests against, on any path (§4). A translucent pane or a smoke particle behind an
SDF rock draws **over** the rock. A new rung, R0b, fixes it, and the mechanism differs per path
because early-Z survives on some paths and not others:
- **Deferred.** Transparents already write `SV_Depth` and have no early-Z. The transparent FS
  compares its own ray distance against the `gViewT` lane the composite writes at every pixel.
  That costs one 4-byte load per shaded transparent fragment and nothing when nothing transparent
  draws.
- **Forward, ForwardPlus and VB.** Transparents keep early-Z. A full-screen depth merge puts SDF
  surfaces into the hardware depth, at ≈ 0.13–0.18 ms at 1440p on the owner's GPU (a lower-bound
  estimate). It is declared only on frames that draw a transparent, and never on `hzb_dump` frames.

**One sibling conflict resolved.** The same-day volumetrics design already owns fog on transparents.
It fogs translucent meshes per fragment with no new axis and particles per vertex, so this delta's
first-revision `-D FOG` rows are withdrawn (§3.9, D-U12).

**New primary evidence (§5.1):**
- **Hashed alpha has a published cost.** Wyman & McGuire measure it at +0.02 to +0.34 ms over a
  plain alpha test at 1080p on a GTX 1080, across ten scenes.
- **The paper's own sine hash is the one JCGT 2020 plots as "obviously bad".** Its `pcg3d` is "a
  good default choice". This design uses pcg3d.
- **The hash is built from existing ops.** With `pcg3d`, exponent-bit tricks and a lattice fed as
  `asuint(floor(x))`, the hashed threshold needs **one** new eDSL op (`floor`), not four. The
  signed-integer family is avoided, because its host arms are `unreachable!` (§3.4).
- **The MBOIT 3.5 ms figure is 16 bit per moment, accumulated "with additive blending".** The
  post does not name the format, so "fp16" is an inference (§12). The 09-10 fit claim holds.
- **No same-rig WBOIT timing exists** in the opened set.
- **Per-platform device coverage (gpuinfo, filtered).** `multiDrawIndirect` is on **99.63 %** of
  Windows reports and **98.16 %** of Linux reports. The 82.68 % the lens quoted is the
  all-platform figure, which includes mobile. This closes V9 as a technical fork (D-U15).

**OIT (§6.3, D-U10).**
- **MBOIT stays R7's family.** WBOIT still cannot be expressed today, because it needs a
  per-attachment blend slice and `independentBlend` (§8.3). It becomes a *frozen challenger* with a
  measured reopen criterion: its whole structural saving is MBOIT's unlit stage 1.
- **The k-buffer is rejected.** It needs 36 B/px, which is 2× MBOIT-4 fp16, and it is exact only
  to 4 layers, while the content that needs OIT (smoke, explosions) is exactly the content with
  many more layers than 4.

**Cutout (§6.2, D-U5, D-U7).**
- R2 stays a masked raster variant.
- Hashed alpha with the paper's LOD fade-in keeps the **expected** coverage, which equals the mean
  alpha that box mips already preserve, so no coverage-preserving mip builder is built.
- **The premultiply step 09-10 prescribed does not survive this tree's texture pipeline** (§3.7).
  - Done in encoded space, white at `a = 0.5` reads 0.214 instead of 0.5.
  - Done in linear light at mip 0 only, the gamma-space blit chain brings the same error back at
    every coverage edge of every coarser mip: −57 % at 50 % coverage, −80 % at 25 %.
- **D-U7 therefore stores straight alpha.** A load-time dilation fills fully transparent texels
  with their neighbours' colour, which is Unity's `alphaIsTransparency` step. The shader
  premultiplies at output. The one per-use form left is the dilated copy. It serves only the
  base-colour slots of non-opaque materials, so an image that is also used elsewhere is uploaded
  twice (D-U7).

## 1. What the 09-10 pair decided (the baseline this delta amends)

| # | 09-10 decision (short) | Status after this delta |
|---|---|---|
| D1 | The exit is at the gather (`Without<Translucent>`), never in a shader | **holds; amended by D-U1** (two queries, stale-material policy) |
| D2 | A cold `MaterialXGpu` extension table; the 48-B `MaterialGpu` stays frozen | holds (`Material` is still `{ gpu, textures }`, `crates/boyko_render/src/material.rs:195-200` [T]) |
| D3 | The route is derived, markers are structural, one authority plus a debug check | **holds; amended by D-U2** (the authority is the authored description; one engine-owned glb spawn path derives it) |
| D4 | Cutout is a masked raster variant on the opaque rail, bucketed `(mesh_id << 1) \| masked` | **holds; amended by D-U5, D-U6, D-U14** (the hash leaf; masked casters in two depth families; masked instances under `hwrt`) |
| D5 | Premultiplied OVER sorted, additive unsorted, multiply behind TK-1; the bake premultiplies before mips | **OVER / additive / multiply hold; the stored premultiply is withdrawn by D-U7** (straight alpha + load-time dilation; premultiplied at the shader output) |
| D6 | v1 sort is a CPU two-pass counting sort over a 16-bit depth key; draw runs keyed `(mesh_id, two_sided, blend_class)` | holds |
| D7 | The sort key is a new eDSL log-depth leaf that the particle key is re-emitted from (TK-7) | **amended by D-U4** (a CPU-only IEEE-bits key; TK-7 leaves R1's critical path) |
| D8 | The translucent FS reads the three-term `use_clusters` gate and arms nothing | holds (the cull is still VB-only, `crates/boyko_render/src/render_path_config.rs:1252`, test `:3459` [T]) |
| D9 | Draw before TAA without motion vectors; a coverage mask under the same blend state | holds; **R6's strength is amended by D-U9** |
| D10 | Refraction uses an opaque colour mip chain, opaque + sky only | **holds; amended by D-U8** (share the reflections `lit_prev` copy; ring depth follows the armed consumers) |
| D11 | MBOIT is the OIT rung | **holds, and D-U10 adds a second argument**: the premise "the only family the RHI can express" still holds (WBOIT's per-attachment blend is absent, §8.3); D-U10 adds the quality and structure case that would still decide it once the RHI grows |
| D12 | UI / MSDF text stays outside the frame graph | holds |
| D13 | Grey translucent shadows first, colour behind one enum growth | holds; memory now computable (§3.8) |
| D14 | Translucents are frustum-culled only in v1 | holds |
| D15 | No depth peeling, PPLL, MLAB or stochastic | holds; **k-buffer and AVBOIT-class added to the list, WBOIT reclassified** (D-U10) |
| D16 | Thin-only transmission before volume; box/thin shape only | holds |
| D17 | SDF shadows and SDFDDGI never see a translucent | holds; **extended by D-U11** (SDF primitives are opaque-only) and **D-U3** (SDF surfaces must occlude translucents) |
| D18 | Variant space bounded to 8 + 12 `.spv` at the top of the ladder | holds for the two FS families (no new axis: the Deferred SDF compare and fog are runtime-gated bindings); §8.4 adds the caster and merge rows this delta creates |

The ladder R1 (sorted) → R2 (masked) → R3 (sampled depth) → R4 (colour chain) → R5 (thin
transmission) → R6 (TAA coverage mask) → R7 (MBOIT) → R8 (volume) → R9 (translucent shadows) →
R10 (reflections on transparents) → R11 (HDR) stands. §9 adds two rungs in front (R0a, R0b) and
amends R1, R2, R4, R6 and R7.

## 2. What still holds at `6394bc5e` — re-verified, with today's anchors

Every row was re-opened by content at `6394bc5e` [T]. The third column is where the fact lives
today; line drift from `ed0bed45` is recorded so a later pass does not re-chase it.

| Fact the design rests on | 09-10 anchor | At `6394bc5e` |
|---|---|---|
| `BlendFactor` is `{Zero, One, SrcAlpha, OneMinusSrcAlpha}`; `BlendOp` is `{Add}` | `enums.rs:836-847`, `:862-867` | `crates/boyko_rhi/src/enums.rs:838` (factor enum), `:864` (op enum); presets `:913`, `:926`, `:949` unchanged |
| One blend state replicated onto every colour attachment | `rhi_impl/device.rs:1882-1917` | `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:1900-1904` (the "ALL color attachments" comment), `:1921` (`from_fn`) |
| `VK_SAMPLE_COUNT_1_BIT`, `alpha_to_coverage_enable: VK_FALSE` literals | `:1845-1855` | `:1865`, `:1869` |
| `independentBlend` exists in the FFI struct and is never set | `ffi.rs:2829` | `crates/boyko_rhi_vulkan/src/ffi.rs:2874`; also `multi_draw_indirect` `:2880` and `fragment_stores_and_atomics` `:2897`, **none set anywhere** under `crates/boyko_rhi_vulkan/src` |
| No push descriptor, no interlock, no int64 atomics | three negative greps | three negative greps, re-run [T] |
| `lit` is `R8G8B8A8_UNORM` | `targets.rs:123-127` | `crates/boyko_rhi_vulkan/src/present/targets.rs:123-127` (doc), built through `create_gbuffer_image` (`:2912-2921`) with `GBUFFER_FORMAT = R8G8B8A8Unorm` (`:2279`) |
| No shader reads `base_color.w` | grep, zero hits | grep over `crates/*/shaders`, zero hits [T] |
| No `Translucent`, `AlphaMasked` or `MaterialXGpu` symbol | — | zero hits [T] |
| The `particle_draw` slot exists in all three declarators | `graph_bridge.rs:2447-2462`, `:2983-2991`, `:6170-6185` | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:2501` (Deferred), `:3030` (Forward family), `:6224` (VB); `declare_particle_draw` `:585`, its `add_pass` `:598`; declarators `:1288`, `:2631`, `:4120` |
| The froxel cull is VB-only | `render_path_config.rs:1224`, `:3385-3406` | `crates/boyko_render/src/render_path_config.rs:1252`; test `froxel_light_cull_is_vb_only` `:3459` |
| The VB raster VS is position-only | `vb_raster.vs.hlsl:169-177` | `crates/boyko_rhi_vulkan/shaders/vb_raster.vs.hlsl:173-174` |
| The Deferred depth horizon `MESH_DEPTH_T_MAX = 64.0` | `gbuffer_mrt.fs.hlsl:114` | `crates/boyko_rhi_vulkan/shaders/gbuffer_mrt.fs.hlsl:113` |
| `VbInstanceRow.flags` bit 0 = occlusion, bits 1..31 reserved | `instance_model.rs:238-248` | `crates/boyko_render/src/instance_model.rs:244` (struct), `:252-259` (flags doc) |
| The particle recorder's two-bucket admission | `particles.rs:772-778` | `crates/boyko_rhi_vulkan/src/present/passes/particles.rs:772-773` |
| R10's sort ⇔ motion-vector exclusion | `particle_config.rs:233-236` | `crates/boyko_render/src/particle_config.rs:233` |
| The particle sort key is a hand-written HLSL string over a fixed `[0.125, 4096]` range | `emit_particles.rs:216-241` | `crates/boyko_shaderdsl/src/bin/emit_particles.rs:221` (`SORT_KEY_FN`), `:183` (`SORT_LOG_NEAR = -3`), `:192` (`SORT_LOG_SPAN = 15`) |
| `-D SOFT` designed, not landed | — | zero `SOFT` occurrences in `emit_particles.rs`; the committed particle draw modules are the base and `_dlin` pairs [T] |
| `add_image_mipped`, `vkCmdBlitImage`, 3D images, dispatch-indirect, draw-indexed-indirect exist | — | `crates/boyko_rhi_vulkan/src/framegraph/graph.rs:392`; `crates/boyko_rhi_vulkan/src/brick_atlas.rs:79` (a `VK_IMAGE_TYPE_3D` image); `crates/boyko_rhi_vulkan/src/device.rs:599` (`cmd_dispatch_indirect`), `:673` (`cmd_draw_indexed_indirect`) |
| No `vkCmdDrawIndexedIndirectCount`, no async compute | — | zero hits; queue selection takes ONE family with `GRAPHICS \| COMPUTE` (`device.rs:2846`) [T] |

## 3. What changed in the tree, and what each change does to the design

### 3.1 The mesh gather is a chained two-query split (asset-streaming prereq (d))

**Fact [T].** `gather_mesh_draws` (`crates/boyko_render/src/mesh_draw.rs:1300`) takes two
queries:
- `q_ok`, filtered `(Enabled<RenderEnabled>, Disabled<RenderStale>, Disabled<MaterialStale>)`
  (`:1315`);
- `q_mat_stale`, identical data under `Enabled<MaterialStale>` (`:1319-1328`).

Every pass walks `q_ok.chain(q_mat_stale)` (`:1198-1206`): the affine scatter (its chain at
`:1388`), the textured-payload scatter (`:1425`) and, under `hwrt`, the prev-ring scatter (`:1634`,
in the hwrt variant at `:1518`). A stale row is drawn with the pinned default material, whose fallback colour is
`[0.8, 0.8, 0.8, 1.0]`, alpha 1 (`:1355-1356`). `resolve_material_id` (`:1460`) additionally maps a
`reserve()`d slot on its spawn frame to id 0, before `validate_asset_refs` (`asset_refcount.rs:522`)
has marked anything.

**Correction to the 09-25 research lens (its P33).** The lens wrote that a filter on one query but
not the other "renumbers the ring". It cannot. The filter is part of the `Query` type, every pass
iterates the same two `Query` params, and so a row is dropped from every pass at once. The
`:1307-1312` warning is about something else: a **data** term (`Option<&OcclusionCulling>`)
written as a filter.

The real hazard is **leakage**. With `Without<Translucent>` on `q_ok` only, a translucent whose
material goes stale enters the opaque ring through `q_mat_stale` as a default-material **opaque**
instance: it writes `vb_id`, becomes an HZB occluder, and (under `hwrt`) becomes a TLAS instance.

**Consequence → D-U1.**
- The exit filter goes on **both** queries, in **both** `cfg` variants (`:1300`, `:1518`), and on
  the caster query.
- The translucent gather is **one** query under `Disabled<MaterialStale>`. There is no stale twin.
- A row whose guarded resolution fell back (`raw ≠ 0 ∧ id == 0`, the spawn-frame case) is skipped
  and counted.
- Why skip rather than substitute? Substituting the default would push an alpha-1 grey silhouette
  through the premultiplied pipeline. A translucent occludes nothing, casts nothing in R1 and writes
  no depth, so skipping it leaves no depth, HZB or shadow hole. The opaque gather substitutes
  precisely to avoid such holes, which is why the two policies differ.

### 3.2 A glb decoder landed, and it discards the route

**Fact [T].** `GlbMaterial` (`crates/boyko_render/src/loaders/glb.rs:945-964`) is "a glTF
material, reduced to the channels this engine's PBR shading consumes":
- it keeps base colour factor, metallic, roughness, emissive and five image indices;
- `decode_materials` (`:1169-1214`) reads nothing else.

`alphaMode`, `alphaCutoff`, `doubleSided`, `KHR_materials_transmission`, `_ior`, `_volume` and
`_specular` are dropped. A glTF `BLEND` material imports as **opaque**, carrying a
`base_color_factor[3] < 1` that no shader reads (§2).

No `.glb`/`.gltf` asset is committed (`git ls-files` returns zero), so changing the import moves no
golden.

**Fact [T]: no engine code spawns a `GlbScene`.** Two **examples** turn one into entities by
hand:
- `crates/boyko_app/examples/playground.rs` builds each part's `Material` inline (`:1578-1608`) and
  spawns a `MeshBundle` with `ShadowCaster`;
- `crates/boyko_app/examples/_hud_probe.rs` repeats the same code, down to a duplicated
  `glb_image_color_space` (`:1648`).

Images are uploaded **once per image index** (`playground.rs:1537-1547`) and shared by every
material that names them (`:1590`). Their colour space is decided per image by "any material
names it" as base colour or emissive (`:1642-1647`).

**Fact [T]: vertex alpha survives import.** The decoder keeps a 4-component `COLOR_0`'s alpha
(`glb.rs:884-886`). glTF multiplies base colour by `COLOR_0`, so a material with no base-colour
texture can still vary in alpha per vertex.

**Consequence → D-U2.**
- The route's authority is the **authored description at spawn**: `GlbMaterial` for imports, the
  Gaia bake for authored scenes, the `spawn_mesh` helper for code. It is **not** the streamed
  `Assets<Material>` row, whose content is unknown until `fill`.
- **One engine-owned glb spawn path** derives the route and inserts `Translucent` / `AlphaMasked`.
  It lives in `boyko_render`, next to the decoder, `MeshBundle` (`crates/boyko_render/src/bundles.rs:43`) and the texture
  upload. The two examples call it instead of carrying their own copies. It also owns the image
  decisions (colour space, and D-U7's dilated copy), because those depend on the same
  per-material route. Without it, R0a + R1 would still spawn imported `BLEND` content opaque unless
  every example were edited separately.
- 09-10's G3 debug check (marker ⇔ `Material::route()`) runs over **Loaded** rows only.
- The importer gains the dropped fields (TK-14, rung R0a). glTF is the specification here: "alpha
  as coverage is NOT for physically-based transparency" (09-10 RESEARCH §5), so
  `transmissionFactor > 0` routes `Translucent` through `route()`, not through `alphaMode`.
- **The masked and translucent FS read alpha as `factor.a × texture.a × COLOR_0.a`**, the glTF
  product. 09-10's "`base_color.w` × albedo alpha" omitted the vertex term.
- **A `MASK` material always routes `Masked`.** The first revision collapsed a textureless `MASK`
  to `Opaque` or refused it, on the premise that its alpha is uniform. The premise is false
  whenever a primitive carries a 4-component `COLOR_0`, so that rule is withdrawn. A material
  whose alpha is uniform and below its cutoff simply discards everything, which is correct and
  rare. No optimisation is kept for it.

### 3.3 Device features are tables, and a capability census gates them

**Fact [T].** `REQUIRED_CORE` (`crates/boyko_rhi_vulkan/src/device.rs:2934`) is
`{samplerAnisotropy, geometryShader}` and `REQUIRED_V13` (`:2955`) is `{dynamicRendering,
shaderDemoteToHelperInvocation}`. The device refuses the whole boot by name when a row is
unsupported (the `RequiredFeatureUnsupported` variant in the same file).

`crates/boyko_rhi_vulkan/src/spirv_capability_census.rs` walks every committed `.spv` and requires
each `OpCapability` to have a licensing row. Its `CAP_DEMOTE_TO_HELPER_INVOCATION` row names
"DXC's lowering of `discard` under `-fspv-target-env=vulkan1.3`".

**Consequences:**
- **R2 needs no new device row.** The masked FS's `discard` lowers to
  `OpDemoteToHelperInvocation`, which is already licensed on every boot.
- **An optional capability must not become a `REQUIRED_*` row.** A required row refuses boots on
  which the rung is disarmed, which breaks "a subsystem that is not used costs ZERO". The census's
  own vocabulary has the right shape: a `Requirement::Conditional` row plus a `DeviceCaps` probe.
  - **The precedent is the `RayQueryKHR` row**, gated by `DeviceEnables::enable_ray_query`
    (`spirv_capability_census.rs:142-149`), which disarms `hwrt` and still boots.
  - `bindless_capable` is **not** a precedent. Its gate refuses the boot
    (`BootError::BindlessUnsupported`, `:111-114`). The first revision cited it wrongly.
  - That is how `independentBlend` (WBOIT), `fragmentStoresAndAtomics` (k-buffer) or
    `multiDrawIndirect` (the MDI rung, D-U15) land when un-frozen (D-U13).
- **`fragmentStoresAndAtomics` is invisible to the census.** A fragment-stage storage write declares
  no dedicated capability, so a k-buffer shader enabled without the feature would pass the census
  and fail only on a validated boot. This is recorded as P48 and is part of why the k-buffer is
  frozen (D-U10).

### 3.4 The eDSL grew an unsigned-integer vocabulary and a bit-exact PCG hash

**Fact [T].** `FieldScalar` still has no `floor`, `frac`, `log2`, `exp2` or `sin`
(`crates/boyko_shaderdsl/src/scalar.rs:56-172`). But the control-flow trait `Cf`
(`crates/boyko_shaderdsl/src/cf.rs:85`) carries a `Uint` vocabulary, and its `EvalCf` host impl
starts at `:1381`:
- **Ops with real host arms:** `uadd` (wrapping), `umul` (wrapping), `shr_u` (`:931`), `ushl`
  (`:1100`), `uxor` (`:1105`), `uor`, `asuint` (`:1117`, `f32::to_bits`), `asfloat`,
  `float_to_uint`.
- **The PCG body:** `particle_rng_body` (`crates/boyko_shaderdsl/src/particle.rs:236`) is the
  32-bit PCG hash, bit-exact host ↔ device.
- **Emit-only ops.** On `EvalCf` these host arms are `unreachable!`: every one exists for
  `m2_brick_cubic_hit_body`, which never runs on the host. A new leaf that runs on the host must
  not use them until each gets a host body (P46). The first revision listed only `usub` and
  `umin`. The full integer-relevant set is:
  - the signed `Int` ops `smax`, `uint_from_int`, `slt`, `sadd`, `float_from_int`
    (`cf.rs:1971-1989`), `sint_eq` (`:2008-2010`) and `temp_int`;
  - `usub` (`:2000-2002`) and `umin` (`:2004-2006`);
  - `select_uint` (`:2021-2023`).
  `int_from_uint` and `float_from_uint` do have host arms.
- **`float_to_uint` is host-saturating.** It is `f as u32` (`cf.rs:1545-1550`), whose own comment
  limits it to non-negative inputs. HLSL's `OpConvertFToU` of a negative value is not the same
  operation, so `float_to_uint` is unusable for object-space lattice coordinates, which are
  negative on half of every object.

**Consequences → D-U4, D-U5:**
- The mesh sort key needs **no** op growth (§7, D-U4).
- The hashed-alpha lattice is fed as **`asuint(floor(scale · obj))`**, per component, into
  `pcg3d`. `asuint` is a host-real `f32::to_bits` (`cf.rs:2192`), and it is injective on the
  integral floats `floor` returns. The one growth is therefore **`FieldScalar::floor`**. The
  signed route, float → `floor` → `int` → `uint`, would need `floor`, a new signed cast and host
  bodies for `uint_from_int` and `float_from_int`: at least three items. It is the priced
  fallback if G20's uniformity check rejects float-bit inputs (§12).
- The `-0.0` corner: `floor` returns `-0.0` only for an input of exactly `-0.0`, whose `asuint`
  differs from `+0.0`'s. The cell hashes to a different, equally uniform and equally stable
  value, so nothing is lost.

### 3.5 The caster gather has no material dimension

**Fact [T].** `gather_shadow_casters` (`crates/boyko_render/src/csm_caster.rs:223-227`) queries
`(&MeshHandle, &InstanceModelCol, Option<&OcclusionCulling>)` under `(Enabled<RenderEnabled>,
Disabled<RenderStale>, With<ShadowCaster>)`. It "has no material dimension … a constant default
payload feeds the shared gather core's material lane inertly" (`:251-253`).

**Consequence → D-U6.** A masked caster needs its albedo alpha, so it needs a `MaterialHandle`,
the stale split and the textured payload. The 09-10 R2 priced "+1 caster variant" and missed the
gather change. Folding material lanes into the one caster scratch would put a material resolution
on every opaque caster row. A **separate** masked caster query and scratch keeps the opaque caster
path byte-identical and at zero cost when no `AlphaMasked` entity exists.

**Fact [T]: there are two caster depth-shader families, not one.**
- **Cascades and spot lights** share an **empty** fragment stage (`csm_depth.fs.hlsl:1-11`, "no
  `SV_Target`, no `SV_Depth`"). The spot atlas pass renders "the SAME caster batches" as the
  cascades (`present/passes/gbuffer.rs:2289-2298`).
- **Point lights** use `punctual_depth.fs.hlsl`, which writes the linear radial distance to
  `SV_Depth` (`:3`, `main` at `:38`). Its VS forwards a world position the cascade VS does not.
- Neither VS reads a `uv`. Both `VsIn` declare position, with colour and normal present only for
  `VertexAttribute` parity (`csm_depth.vs.hlsl:62-64`, `punctual_depth.vs.hlsl:52-54`), and the
  shared attribute array stops at location 2.

So a masked caster needs a masked variant of **both** pairs, and each masked VS needs a `uv`
source: either a `VertexAttribute` growth at location 3, or the geometry-table pull 09-10 chose
for the VB VS. That is 4 `.spv` in 2 manifest rows, not 1 (§8.4). A gate that checks only the
CSM would pass while spot and point shadows ship as full cards. G22 covers all three (§9).

**Fact [T]: every `hwrt` shadow query forces opacity.** The ray queries in `deferred_pbr.hlsl`
(`:1113-1114`, `:1148-1149`) and `vb_shadow_vis.comp.hlsl` (`:224-225`) are declared with
`RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_FORCE_OPAQUE` (plus
`SKIP_PROCEDURAL_PRIMITIVES`). A masked instance packed into
the TLAS therefore casts a full-card RT shadow, whatever its BLAS geometry flags say. 09-10 left
this UNVERIFIED (its §14). D-U14 decides it.

### 3.6 The reflections design owns HDR `lit` and a `lit_prev` copy

**Fact [T]** in [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md):
- Its R4a re-types `lit` to **B10G11R11** when `lit_hdr_format_ok`, else `R16G16B16A16_SFLOAT`,
  behind a boot probe (`:196`).
- It places a `lit_prev` copy "after the last opaque producer; the transparency campaign's
  `scene_color_copy` is the same copy — one pass, two consumers" (`:397-398`). `lit_prev` is a
  ring ×2 because the trace reads the **previous** frame (`:201`).

**Consequences → D-U8:**
- 09-10's R11 (HDR scene colour) **is** reflections R4a. There is one decision, one owner ballot,
  one set of re-blessed goldens.
- "One pass, two consumers" is literally true only if the refraction chain's level 0 **is**
  `lit_prev[fi]` and the chain image holds levels 1..k.
- **The ×2 ring belongs to reflections alone.** Refraction reads the **same** frame's copy.
  09-10's refraction chain was a single mipped image, and a mipped graph image is "by
  construction the non-ringed, cross-frame kind" (`framegraph/graph.rs:396`). So the level-0
  image's ring depth must follow the armed consumer set: 2 when a previous-frame reader
  (reflections) is armed, 1 otherwise. Charging the second slot to a refraction-only boot costs
  +14.7 MB at 1440p for a subsystem that is off. The first revision did exactly that; D-U8 now
  derives the depth instead.
- **HDR `lit` must also blend.** Reflections' RK-14 (`lit_hdr_format_ok`) probes B10G11R11 for
  `STORAGE_IMAGE` and `COLOR_ATTACHMENT` only. Every particle draw already blends into `lit` on all
  four paths today (`declare_particle_draw`'s `lit` access "a BLEND is a read-modify-write",
  `graph_bridge.rs:585`), and every rung here adds more. The probe therefore needs a third bit,
  `COLOR_ATTACHMENT_BLEND`, for the arm it picks. This is filed with the reflections design, which
  owns RK-14 (§12, P53).

### 3.7 The texture pipeline filters in gamma space, and premultiplication must respect it

**Fact [T].** Albedo is uploaded as `R8G8B8A8_UNORM` with a mutable sRGB view. The mip chain is a
LINEAR blit on the UNORM image, so "mip filtering happens in gamma space"
(`crates/boyko_render/src/texture.rs:29-35`; the upload is `upload_mipped_pixels`, `:171`). No CPU
mip builder exists.

**What 09-10's TK-8/TK-9 missed, at mip 0.** Premultiplying the **encoded** value (`c_enc · a`)
and then decoding through the sRGB view gives `decode(c_enc · a)`, which is not `c_lin · a`. For
white at `a = 0.5`, the sRGB EOTF gives `((0.5 + 0.055) / 1.055)^2.4 = 0.214` where 0.5 is
correct: the texel reads **57 % too dark** [D-arith].

**What the first revision of this delta missed, at every coarser mip.** It prescribed
`encode(decode(c) · a)` over mip 0 and kept the blit chain. But the chain averages **encoded**
values, and after any premultiplication every coverage edge pairs a colour with 0 [D-arith]:
- A 2×2 block, half opaque white and half transparent, averages to 0.5 encoded. That decodes to
  **0.214** premultiplied, where 0.5 is correct. Divided by the mip's alpha 0.5 (the first
  revision's masked FS), it reads **0.428 instead of 1.0: −57 %**.
- At 25 % coverage it averages to 0.25 encoded, decodes to **0.051** where 0.25 is correct, and
  divides to **0.204: −80 %**.

Opaque albedo pays the gamma-space mip error only between neighbours of very different colour,
which is rare. A premultiplied chain pays it at **every** alpha edge, because 0 is always the most
different neighbour. Foliage and fences are the content R2 exists for, and hashed alpha reads
coarse mips by design, so they would darken with distance on every path, and blended edges would
fringe dark. The first revision's G21 read mip 0 only, so it would have stayed green.

**Two ways out, priced.**

| Option | Result at a coverage edge, mip ≥ 1 | New machinery | Shared images (W2) |
|---|---|---|---|
| (i) premultiplied chain built in **linear light** on the CPU | exact up to 8-bit quantisation | a CPU mip builder (decode → ×a → box → encode, 4/3 N texels) **and** a pre-mipped upload path. Neither exists: `upload_mipped_pixels` takes mip 0 and blits (`texture.rs:171`) | an image named by both an opaque and a non-opaque material needs two uploads, and its opaque use must not see the premultiplied form |
| **(ii) straight alpha + load-time dilation; premultiply at the shader output** | colour is the neighbour's colour; alpha is the exact linear mean, since sRGB never encodes alpha. So `decode(c_f) · a_f` equals the linear premultiplied mean wherever colour is locally constant across the edge: **0.5 at 50 %, 0.25 at 25 %, no darkening**. Elsewhere the residual is the gamma-space error the tree already accepts for opaque albedo, plus a colour × alpha covariance term that is second-order in the colour difference | one CPU pass over mip 0 (a push-pull fill of `α = 0` texels, O(4/3 N)) before the existing `upload_mipped_pixels` | only the dilated copy is per-use (D-U7) |

(ii) is taken (D-U7).
- **What it removes.** The paper's objection to straight alpha is that "colors bled from
  transparent texels and introduced arbitrarily colored halos" (§5.1). Dilation removes exactly
  that cause: a transparent texel carries its neighbour's colour.
- **It is shipped practice.** Unity's importer does the same step: "dilate the color channels of
  visible texels into fully transparent areas … prevents filtering artifacts from forming on their
  edges" [D, §14 H].
- **(i) is frozen, not rejected.** It reopens if G21's two-colour edge fixture (§9) shows an error
  above the same fixture's all-opaque gamma error plus one 8-bit code. The CPU mip builder it needs
  is the same one the frozen coverage-preserving mips would need (§6.2), so a reopen of either
  shares the cost.

### 3.8 Shadow-map dimensions are confirmed

`CSM_SHADOW_DIM = 2048` (`crates/boyko_app/src/gpu_scene/mod.rs:623`), `MAX_CASCADES = 4`
(`crates/boyko_render/src/csm_config.rs:77`); the punctual atlas is `SHADOW_DIM = 512` ×
`M_SLOTS = 16` (`crates/boyko_render/src/shadow_atlas.rs:60`, `:56`) [T].

R9's transmittance lanes therefore cost [D-arith]:
- **Grey `R8`:** 4 × 2048² = **16.8 MB** for the CSM plus **4.2 MB** for the atlas.
- **Colour `RGBA8`:** **67.1 MB** plus **16.8 MB**.

These are resolution-independent. The 09-10 open item "CSM resolution not read" is closed.

### 3.9 The volumetrics design owns fog on transparents, and its grid is its own

[`OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md) phase E-FOG (`:433-435`) planned a
**160×90×64** froxel volume and stated that "the froxel XY×Z IS the cluster grid". The light-cull
grid in the tree is **16×9×24** (`crates/boyko_render/src/light.rs:49-53`) [T]. The first revision
reasoned from that E-FOG text. **It is superseded.** The same-day
[`VOLUMETRICS-DESIGN-SPACE.md`](VOLUMETRICS-DESIGN-SPACE.md) (cited by decision ID, since it is
still under revision) decides:
- **C1.** A fog-owned grid, 160×90×64 by default and 160×90×128 on High, independent of
  resolution. Lights reach it through a second instance of the unchanged cluster-cull kernel over
  a fog-only light table at 16×9×32. "Fog grid == cluster grid" is withdrawn.
- **D6.** Fog composes in one `fog_apply` pass on the HDR `lit`, and requires R4a / R11.
- **D17.** The single scene-colour / `lit_prev` copy runs **after** `fog_apply`, so refraction
  sees a fogged background.
- **D18 and §4.9.** Fog on transparents:
  - **Translucent meshes** fetch fog **per fragment** in `forward_translucent.fs`, through
    bindings that are always bound (a placeholder when unarmed) and gated by a runtime uniform.
    There is **no new axis**, so this design's D18 bound stands.
  - **Particles** fetch **per vertex, at the particle centre**, in `particle_draw.vs` behind
    `-D PARTICLE_FOG`: +2 `.spv` on top of `DEPTH_LINEAR`.
  - The composite rule is `c·T(d) + S(d)·α` for alpha and `c·T` for additive.

**Consequences.**
- **Fog on transparents has one owner, the volumetrics design.** D-U12 is rewritten to adopt
  VOLUMETRICS D17/D18 and withdraws the first revision's `-D FOG` rows on
  `forward_translucent.fs` and `particle_draw.fs`. Both documents were written the same day and
  disagreed. Implementing both would have fogged particles twice, once per vertex in the VS and
  again per pixel in the FS, which squares the transmittance.
- **UE's per-vertex artefact (P41) is still honoured.** It is a large-triangle artefact, and the
  meshes that have large triangles are fogged per fragment. A particle is a small quad evaluated at
  its centre, which is the same granularity as its per-particle lighting (PARTICLES D11).
- **The seam gains `fog_apply`** in front of the `lit_prev` copy (§8.2).
- **An AVBOIT-class transmittance grid** (§6.3) would be sized against the fog-owned grid, not the
  light-cull grid. It stays not-a-candidate (UNVERIFIED internals).

## 4. The hybrid gap: SDF surfaces never reach the seam's depth (new)

### 4.1 Facts, per path

| Path | What the SDF march writes | Depth the transparent slot tests | Evidence [T] |
|---|---|---|---|
| Deferred | `gViewT` (`R32_SFLOAT`, the surface `t`, valid under `mask == 1`) + the G-buffer | `depth` (ResId 3) — mesh raster only | `targets.rs:128-130`; `docs/PARTICLES-PLAN.md:1541`: "SDF surfaces do not write depth on this path … billboards are not occluded by SDF geometry there" |
| Forward / ForwardPlus | `lit` only; **reads** `forward_depth` when the mesh leg exists | `forward_depth` — mesh raster only | `graph_bridge.rs:2990` (`sdf_forward_march`, writes `lit`, reads depth); the VIEWT variant is forbidden here by `debug_assert` (`:2984-2988`) |
| VisibilityBuffer | `lit`, and `viewt` only when `path_sdf_forward_writes_viewt()` (TAA-armed SDF legs); **reads** `vb_depth` | `vb_depth` — mesh raster only | `graph_bridge.rs:6098` (pass), `:6106-6114` (the conditional `viewt` write) |

On every path with an SDF leg, a translucent mesh or an alpha/additive particle **behind** an SDF
surface passes the depth test and draws over it. PARTICLES-PLAN records this for one case (Deferred
without a mesh raster). It is in fact the general case, for particles today and for every 09-10
rung tomorrow.

### 4.2 Options, with numbers, per path

Traffic below is per pixel of the full frame. Time is that traffic at **336 GB/s**, the RTX 3060
Laptop's spec bandwidth (§6.1). It is a **lower bound, an estimate**.

**The deciding fact differs per path: whether transparents keep early-Z** [T].
- **Deferred: no early-Z.** Particles take `-D DEPTH_LINEAR`, which writes `SV_Depth`, so "the
  billboards therefore pay full shading before the depth reject on Deferred"
  (`crates/boyko_rhi_vulkan/shaders/particle_draw.fs.hlsl:36-42`). Translucent meshes need the
  same linear encode, because the marcher matrix pins projective `z` to 1.0 on that path.
- **Forward, ForwardPlus, VB: early-Z kept.** The three reverse-Z paths take the base compile.

The first revision argued (a) for every path from "0 per-fragment cost, early-Z". That is false
on Deferred, where (a) would pay its fixed cost with none of the benefit.

| Option | What it does | Fixed cost | Per-transparent-fragment cost | RHI growth | Verdict |
|---|---|---|---|---|---|
| **(a) `sdf_depth_merge`** — a full-screen raster pass writing `SV_Depth` from the SDF `t` lane, depth-tested with the path's compare op | puts SDF surfaces into the hardware depth once | read `t` 4 B + depth test/write 8 B = **12 B/px** where the `t` lane already exists (VB with TAA): 24.9 / 44.2 / 99.5 MB ⇒ **≈ 0.07 / 0.13 / 0.30 ms**; **16 B/px** where the marcher must newly write `t` (Forward; VB without TAA): 33.2 / 59.0 / 132.7 MB ⇒ **≈ 0.10 / 0.18 / 0.40 ms** at 1080p / 1440p / 4K [D-arith, estimate] | **0 where early-Z holds**: hidden fragments are rejected before shading | **none**: a depth-only pipeline with a null colour-blend state already exists (`rhi_impl/device.rs:1934-1939`) | **taken on Forward, ForwardPlus, VB** |
| **(b′) fused compare in the transparent FS** | the FS computes its own ray distance, loads the `t` lane at its pixel and `discard`s when behind | **0**; a frame with no transparent draw runs nothing | one 4-B load per shaded transparent fragment; DRAM ≤ 4 B per px of **covered** area while the lane stays in L2 across layers (estimate), ≤ 4 B × depth complexity without reuse | **none**: the `t` lane is an R32F **colour storage image** (`present/targets.rs:128-133`), bindable through an ordinary descriptor set. The first revision's "R3's push descriptor first" was wrong | **taken on Deferred** |
| (c) the marcher writes the depth image directly | compute store into the depth image | — | — | storage-image usage on a depth format; this pass claims no portable guarantee for it and found no use of it in the tree | rejected — (a) and (b′) reach the same result with shapes that already exist |

**Why (b′) wins on Deferred, with numbers.** (a) there costs a fixed 12 B/px (≈ 0.13 ms at 1440p),
because `gViewT` already exists, and it buys no shading saving. (b′) costs 4 B per covered pixel
with L2 reuse, which is at most a third of (a) even at full-screen coverage. Without reuse it
still wins while the mean depth complexity over the whole screen stays below 3. It costs nothing
on a frame with nothing transparent, and it touches neither HZB nor the dump. On Forward and VB,
(b′) would shade every hidden fragment (a `discard` still runs the rest of the quad as helper
lanes). Smoke behind SDF terrain is the worst case there, so (a) keeps its argument on those paths.

**The Deferred compare is complete** [T]. The composite writes `gViewT` at **every** pixel: the
SDF `t` where the SDF won, `t_mesh` where the mesh won, and `1.0e30` over background
(`crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl:1902`). `t_mesh = md · MESH_DEPTH_T_MAX`,
and `md` is the `DEPTH_LINEAR` encode `length(eye_rel) / 64`, so on the perspective arm
`length(eye_rel) ≥ gViewT[px]` is one test against mesh and SDF occluders together, and it always
passes over background.
- **Where it lives.** It goes into the Deferred-only `-D DEPTH_LINEAR` builds of `particle_draw.fs`
  (and later `forward_translucent.fs`).
- **Binding.** `gViewT` is always bound and gated by a runtime uniform, armed iff the Deferred boot
  has an SDF leg. This is the always-bound placeholder pattern VOLUMETRICS D18 also uses. It adds
  **no axis**, so D18's count stands, and the `_dlin` row's interface grows by one binding.
- **Ortho.** The arm's compare quantity is open (§12).

**The discriminator, for (a).** Where no mesh surface exists, the merge simply writes. Where one
does, `t` on the SDF legs is the **composite** surface, so the merge writes
`project(t · (1 + 2⁻¹⁶))`:
- for an SDF-won pixel the biased depth is still nearer than the mesh depth, so it passes the test
  and writes;
- for a mesh-won pixel the biased depth is farther than the stored mesh depth, so it fails and
  writes nothing.

The loss is SDF surfaces within 0.0015 % relative distance of a mesh surface. That is 1.5 × 10⁻⁵
of the depth, finer than the 16-bit sort key's 0.0122 % minimum bin (§7, D-U4), so no transparent
ordering can resolve it [D-arith].

**Placement of (a).**
- **VB.** The merge sits after `sdf_forward_march` and before `translucent_draw`/`particle_draw`.
  That is after the HZB block, which the frame declares in exactly one of two alternative slots
  (split, `graph_bridge.rs:5056`; unsplit, `:5635`). So **this frame's HZB is byte-identical** and
  occlusion culling does not change. The first revision called these "both HZB builds"; there is
  one build, in one of two positions.
- **Forward.** The rung extends the declarator with the `viewt` write that the `debug_assert` at
  `:2984-2988` demands first.

**The `hzb_dump` diagnostic (P45, decided).**
- The dump reads `vb_depth` as its source and is "DECLARED LAST in the whole graph" by a stated
  invariant (`graph_bridge.rs:6289-6294`). It therefore cannot be declared before the merge; the
  first revision's second option is closed.
- **The merge is not declared on frames where `scene.hzb_dump.is_some()`.** The graph is
  re-declared every frame (`declare_frame_graph`, called by `render_gbuffer_frame`), so this
  omission is per frame. Transparents behind SDF draw wrongly on dump frames only, which are
  diagnostic frames.

**Arming (a) follows presence, not capability.** Because the graph is re-declared every frame, the
merge is declared on a frame only when all of these hold:
1. the path has an SDF leg;
2. the frame is not a dump frame;
3. the frame draws a transparent:
   - **Translucent meshes:** the frame's translucent gather produced at least one instance. That is
     an exact CPU count.
   - **Particles:** the particle draw arms on `particle_res`, the capability, because the live
     particle count is GPU-side. The merge follows a CPU **presence bound** instead: live emitter
     entities, extended after the last despawn by the longest authored particle lifetime. The
     particles design owns that bound. Until it supplies one, the merge follows `particle_res`, and
     an SDF app with the particles plugin on and zero emitters pays the fixed cost above.

Deferred's (b′) needs no arming: a frame with no transparent draw runs no transparent FS.

**Hierarchical-Z and depth compression after the merge.** A pass that writes `SV_Depth` can change
what the hardware's hierarchical-Z and depth compression do for later draws that test the same
buffer. The bandwidth model above does not include this, and this pass found no source to price
it.
- **The gate measures it.** R0b's perf gate takes the merge zone **and** the downstream transparent
  zones, armed versus disarmed, on a fixture where the merge occludes nothing. Any side effect then
  shows up as a delta instead of hiding inside the draws (§9).
- **A knob if the delta is non-zero.** Put the merge's full-screen triangle at the far plane and
  declare conservative depth, since its output is never farther. The effect on this GPU is
  UNVERIFIED.

**Golden impact.** Particle goldens that place billboards behind SDF surfaces change, on every path
with an SDF leg. This is a correction, not a regression, and it needs the owner's re-bless
(ballot V17).

## 5. Research delta — evidence new since 09-10, with provenance

### 5.1 Opened by this pass (primaries unless tagged)

- **Wyman & McGuire, "Hashed Alpha Testing", I3D 2017** [D]
  (https://cwyman.org/papers/i3d17_hashedAlpha.pdf, text extracted from the PDF).
  - **Cost, Table 1: "alpha tests at 1920×1080 on a GeForce GTX 1080".** Traditional → hashed →
    stochastic, per scene:

    | Scene | Traditional | Hashed | Stochastic |
    |---|---|---|---|
    | Single fence | 0.06 ms | 0.08 ms | 0.20 ms |
    | Bishop Pine | 0.22 ms | 0.30 ms | 0.75 ms |
    | European Beech | 0.39 ms | 0.50 ms | 1.69 ms |
    | UE3 FoliageMap | 2.52 ms | 2.86 ms | 11.42 ms |
    | San Miguel | 5.19 ms | 5.28 ms | 7.30 ms |

    Across all ten scenes hashed costs **+0.02 to +0.34 ms, +2 % to +36 %**. The authors "did not
    optimize performance".
  - **Hash.** The paper uses `fract(1.0e4 * sin(17.0*x + 0.1*y) * (0.1 + abs(sin(13.0*y + x))))`
    and states "our hash function is less important than its properties".
  - **Algorithm (Listing 1).**
    1. `pixScale = 1/(g_HashScale · max(|ddx(obj)|, |ddy(obj)|))`.
    2. Two lattice scales `exp2(floor(log2 pixScale))` and `exp2(ceil(…))`.
    3. `hash3D(floor(scale · obj))` at each scale, lerped by `fract(log2 pixScale)`.
    4. The lerp goes through a CDF with `a = min(lerp, 1 − lerp)` so the threshold stays uniform.
    5. Clamp to `[1e-6, 1]`.
  - **Under TAA** "using noise below pixel scale (e.g., 0.3–0.5) allows for temporal averaging".
  - **Fade-in.** Noise fades in by LOD as `ατ = 0.5 + δ·b(lod)`, with `b` quadratic up to `n` and
    "n = 6 worked well". Anisotropy scales the LOD input.
  - **Premultiplied alpha.** "We encourage using premultiplied alpha, which mipmaps correctly"; with
    straight alpha, "colors bled from transparent texels and introduced arbitrarily colored halos".
  - **Mip coverage.** Coverage loss comes from "reduced alpha variance" in box-filtered mips, among
    other causes. Castaño's per-mip thresholds "differ between textures and even within a mip
    level".
- **Jarzynski & Olano, "Hash Functions for GPU Rendering", JCGT 9(3), 2020** [D]
  (https://jcgt.org/published/0009/03/02/paper.pdf).
  - Figure 1 calls the LCG and **trig** hashes "obviously bad, with visible banding, linear
    artifacts, and repeated patterns".
  - "pcg3d and pcg4d fall on the Pareto Frontier and are a good default choice for
    multidimensional high-quality hash functions".
  - `pcg3d` is multiply-add, xor and shift only:
    `v = v*1664525u + 1013904223u; v.x += v.y*v.z; …; v ^= v >> 16u; …`.
- **interplayoflight, "Order independent transparency: endgame" (2022-07-10)** [B]
  (https://interplayoflight.wordpress.com/2022/07/10/order-independent-transparency-endgame/).
  - Hardware blend 1.37 ms at 1080p.
  - MBOIT 4 / 8 moments at 32 bit: 8.1 / 13.1 ms. At 16 bit: **3.5 / 6.2 ms**. At 16 bit
    half-resolution: 2.4 / 4.1 ms.
  - Memory 10 B/px (~20 MB) and 18 B/px (~37 MB).
  - "I accumulated the moments with additive blending".
  - Conclusion: decide "on a case by case basis".
  - Part 1 (2022-06-25) names the rig, a "Laptop RTX 3080" at 1080p, and gives PPLL 4.20 + 1.32 ms,
    ~200 MB. **Neither post measures WBOIT.**
- **Wikipedia, "GeForce 30 series"** [B, vendor spec reproduced]
  (https://en.wikipedia.org/wiki/GeForce_30_series).
  - RTX 3060 Laptop: 3840 CUDA cores, 30 SM, 192-bit, 336 GB/s, TGP 60–115 W.
  - RTX 3080 Laptop: 6144 cores, 48 SM, 256-bit, 448 GB/s, TGP 80–150 W.
  - Desktop RTX 3080: 8704 cores, 760 GB/s.
- **The Vulkan spec's formats chapter** (https://docs.vulkan.org/spec/latest/chapters/formats.html)
  and gpuinfo's optimal-tiling list were opened again. Neither returned the required-support table
  or the float-format rows, so **`R32_SFLOAT` blend support stays UNVERIFIED** (fourth attempt);
  TK-13's boot probe still covers it.

Opened by revision 2:
- **gpuinfo, core 1.0 feature coverage, filtered by platform** [D-data]
  (https://vulkan.gpuinfo.org/listfeaturescore10.php?platform=windows and `?platform=linux`).
  These count driver **reports**, not installed base.

  | Feature | Windows | Linux |
  |---|---|---|
  | `multiDrawIndirect` | **99.63 %** | **98.16 %** |
  | `geometryShader` (already `REQUIRED_CORE`) | 99.27 % | 96.31 % |
  | `independentBlend` | 100 % | 100 % |
  | `fragmentStoresAndAtomics` | 100 % | 99.77 % |
  | `samplerAnisotropy` (already `REQUIRED_CORE`) | 100 % | 100 % |

  The lens's `multiDrawIndirect` 82.68 % was the all-platform figure, which includes mobile.
- **Unity, `TextureImporter.alphaIsTransparency`** [D]
  (https://docs.unity3d.com/ScriptReference/TextureImporter-alphaIsTransparency.html): "dilate the
  color channels of visible texels into fully transparent areas. This effectively adds padding
  around transparent areas that prevents filtering artifacts from forming on their edges". The page
  adds that it leaves the colour of invisible texels undefined.
- **Two more attempts at the mandatory-format table failed.**
  - The raw `Vulkan-Docs` `chapters/formats.adoc` was returned truncated before the tables.
  - gpuinfo's per-format `COLOR_ATTACHMENT_BLEND` listing for `B10G11R11_UFLOAT_PACK32` loads its
    rows client-side and returned none.

  So whether B10G11R11 and RGBA16F guarantee blending is **UNVERIFIED**; see §12 and P53.

### 5.2 Carried from the 09-25 research lens (not re-opened by this pass)

These are cited as the lens tagged them; each URL is in §14.
- **Masked materials in virtual-geometry rasterizers:**
  - Nanite supports only Opaque and Masked [D].
  - Nanite Foliage avoids masks because masking "introduces a lot of overdraw" [D].
  - Bevy 0.19.1 `MeshletMesh` is opaque-only [D].
- **Alpha-to-coverage.** Bevy: without MSAA it "is identical to Mask with a value of 0.5" [D].
- **Coverage-preserving mips:** Castaño (NVTT) [B, author], DirectXTex
  `ScaleMipMapsAlphaForCoverage` [D], Unity `mipMapsPreserveCoverage` [D], Yuksel alpha
  distribution [D].
- **MBOIT's half-memory quantised form "can use rasterizer ordered views"** [D, author postscript].
- **k-buffers:**
  - nvpro Loop32: `8·K+4` B/px, two draws, no extension [S].
  - Diligent layered OIT: `K·4+4` B/px, atomic min, `earlydepthstencil` crucial [D].
- **Feature coverage on gpuinfo, all platforms:** `independentBlend` 100 %,
  `fragmentStoresAndAtomics` 99.96 %, `multiDrawIndirect` 82.68 % [D-data]. These are superseded
  for this engine's targets by the per-platform figures revision 2 opened (§5.1).
- **Upscaler reactive mask.** FSR2: write alpha to the reactive mask, "clamping the maximum reactive
  value to around 0.9" [D, vendor guidance].
- **UE:**
  - Coloured translucent shadows work with Static lights only [D].
  - The translucency lighting volume uses `r.TranslucencyLightingVolumeDim` 64 [D].
  - Fog on translucency is per vertex by default [D].
- **HDRP:** transparent execution order [D]; coloured shadows only through ray tracing [D].
- **Unity Entities Graphics:** `DepthSorted_Tag` is a structural sort tag [D].
- **Doom:**
  - Doom 2016 builds a refraction blur chain and lights particles in an atlas [B].
  - Doom Eternal draws transparents after its 160×90×64 scattering volume [B].
- **Newer OIT work:**
  - Wavelet OIT: 1.0–1.6 ms at 1440p on a desktop RTX 3080 in one implementation [B].
  - AVBOIT (Drobot 2025): internals **UNVERIFIED**; the deck could not be decoded [D abstract].

### 5.3 What shipped engines do (the delta rows)

| Engine | Cutout in a VB / meshlet raster | Blend | OIT | Coloured translucent shadows | Source |
|---|---|---|---|---|---|
| UE5 | Masked through the programmable raster; Nanite Foliage moves foliage to geometry | sorted per object, priority | experimental per-pixel sorted | Static lights only; not Lumen, not HWRT | §14 [4] [5] [9] |
| Unity HDRP | — (no VB) | enumerated transparent stages, depth prepass/postpass | — | ray-traced only | [13] [14] |
| Unity Entities Graphics | — | `DepthSorted_Tag` component opts an entity into sorting | — | — | [16] |
| Bevy 0.19 | meshlets **opaque only** | sorted `Transparent3d`; A2C = Mask 0.5 without MSAA | fixed-layer buffer, manual depth test | — | [18] [19] [20] |
| Godot 4 | — | sorted by AABB centre; Alpha Hash (casts shadows), Alpha Scissor, A2C | — | — | [17] |
| The Forge / Confetti | a flag bit + dedicated indirect path | forward, sorted draw calls | VB linked list of triangle IDs | — | 09-10 RESEARCH §2 |
| id Tech (Doom Eternal) | — | forward after the scattering volume | — | — | [41] |

**The pattern that matters for this engine:**
- Every engine that ships a visibility buffer or meshlet raster treats masked as the expensive
  case: Nanite's own foliage guidance, Bevy's exclusion.
- **Grey translucent shadows for dynamic lights are shipped practice.** UE casts them through
  Fourier Opacity Maps ([`VFX-RESEARCH.md`](VFX-RESEARCH.md) §4.4, carried there).
- **Coloured translucent shadows for dynamic lights are not.** No engine ships them on the raster
  path: UE limits coloured ones to Static lights, and HDRP to ray tracing.
- So the 09-10 R9's **grey** lane is within shipped practice, and only its **colour** growth
  (TK-1's enum growth) is beyond it. The first revision said all of R9 was beyond practice, which
  mixed the two. Both halves stay late in the ladder, where they already sit.

### 5.4 Corrections to the 09-25 research lens

| Lens claim | Correction [T] |
|---|---|
| P33: a filter on one of the two queries "renumbers the ring against the prev-ring and TLAS lanes" | Every pass walks the same two `Query` params, so a filter drops the row everywhere. The hazard is **leakage** of a stale-material translucent into the opaque ring (§3.1) |
| "The translucent gather has to copy the split and call `resolve_material_id`" | It takes one query (`Disabled<MaterialStale>`) and skips fallbacks (D-U1) |
| Hashed alpha "needs `floor`/`frac`/`log2`/`exp2` op growth" | one `floor`; the lattice is `asuint(floor(x))`, and the rest is exponent-bit arithmetic over existing `Cf` ops with host arms (D-U5, §3.4) |
| Hashed alpha: "no cost figure was found" | Wyman & McGuire Table 1 (§5.1) |
| "Coverage-preserving mips are a bake step" the design needs | not needed under hashed alpha with fade-in (D-U10's cutout row, §6.2) |
| The eDSL "still has no `log2`, `exp2`, `floor` or `frac`" | true of `FieldScalar`; the `Cf` `Uint` vocabulary and the PCG body the lens did not report change the conclusion (§3.4) |
| Unmentioned | the glb decoder drops the route (§3.2); SDF surfaces never reach the seam's depth (§4); the caster gather has no material dimension (§3.5); sRGB premultiply (§3.7) |

## 6. Options scored against the owner rules

### 6.1 Reference rig and the scaling rule

**Reference GPU:** the owner's RTX 3060 Laptop, 6 GB (`docs/OPTIMIZATION-PLAN-RENDER.md` and
`docs/LIGHTING-PLAN.md` name it, per 09-10 §1.5).

Published OIT figures are on an RTX 3080 Laptop at 1080p. Scaling rule [D-arith on §5.1 specs;
the result is an **estimate**]:
- **Rig:** × **1.33** (bandwidth 448 / 336) to × **1.6** (SM 48 / 30). TGP varies 60–150 W across
  laptops, so the true factor is not measured.
- **Resolution:** × 1.78 for 1440p and × 4.0 for 4K, assuming fill-bound cost.

Pixels: 2 073 600 / 3 686 400 / 8 294 400.

### 6.2 Cutout (foliage, fences, hair cards, chain-link, LOD dissolve)

| Option | Quality | GPU (published) | VRAM | Rule 3 (ECS / zero when off) | Rule 4 (hot path) | Rule 5 | Rule 6 (eDSL / RHI) | Verdict |
|---|---|---|---|---|---|---|---|---|
| Plain alpha test | disappears with distance (paper §2) | baseline in Table 1 | 0 | `AlphaMasked` ZST; masked pipelines built only when a masked entity class is armed | `discard`; no allocation | in-house | `discard` licensed (§3.3) | the near-field half of the fade-in |
| **Hashed + fade-in (Wyman & McGuire)** | keeps expected coverage = mean alpha; stable noise; converges under TAA | **+0.02 to +0.34 ms at 1080p, GTX 1080, over a plain alpha test** [D] | 0 | same | ~50 ALU per masked fragment (estimate: two `pcg3d` calls + CDF) | in-house | one `floor` growth (D-U5) | **taken (R2)** |
| Coverage-preserving mips (Castaño / Yuksel) | fixes thinning for **plain** cutoff only; per-mip thresholds are content-dependent (paper §3) | 0 at runtime | 0 | a load-time CPU builder + a pre-mipped upload path | load time only | in-house (no NVTT/DirectXTex) | none | **frozen**: not needed under hashed; reopen with V16 |
| Alpha-to-coverage | best edges | needs MSAA | 4× MSAA `vb_id` `R32G32_UINT` (`targets.rs:1109`) + `D32` = 48 B/px ⇒ **99.5 / 176.9 / 398.1 MB** before any per-sample resolve [D-arith] | an MSAA VB is a path rebuild | — | — | `SAMPLE_COUNT_1` literal | **rejected** |
| Stochastic alpha test | noisy even under TAA | 1.4–4.5× plain (Table 1) | 0 | — | — | — | — | rejected by the paper's own numbers |

**Why hashed plus fade-in needs no coverage-preserving mips** [D-arith on paper §2 and Listing 1]:
- A box-filtered mip preserves the mean alpha exactly: the blit filters the linear alpha channel,
  and sRGB never encodes alpha.
- A uniform random threshold passes a fragment with probability `α`, so the expected coverage at
  any LOD is the mean alpha.
- The paper's fade-in keeps the plain 0.5 threshold near the camera, where the fine mips have not
  lost variance yet.
- Castaño scaling would *raise* the mean alpha of distant mips, so under hashed alpha distant
  foliage would grow denser. The two techniques conflict on one texture, and hashed is the one the
  ladder needs anyway.

**Per-path threshold policy (P35), decided here.**
- **TAA armed** (Deferred or VB with `taa_on`): hashed, `g_HashScale = 0.5` (the paper's TAA
  range), fade-in `n = 6`.
- **TAA off** (Forward, ForwardPlus, or TAA disarmed): hashed, `g_HashScale = 1.0`, the same fade-in.
  The result is stable, object-anchored stipple at distance instead of thinning.

The policy is one boot-frozen uniform, not a variant: `{mode ∈ {plain, hashed}, g_HashScale}`.
- **Plain is a dynamically uniform branch around the hash**, not a separate `.spv`. It serves two
  purposes. It makes V16's alternative a uniform value. It also gives R2's perf gate (i) a
  same-pipeline A/B of hashed versus a plain alpha test (§9), the comparison the paper's Table 1
  measures.
- **Taste call.** Whether the owner prefers thinning to stipple on the no-TAA paths is ballot V16.

**What Table 1 does and does not price** [D]. Its three columns are all alpha-tested renders: a
traditional test, hashed, and stochastic. It bounds **hashed versus plain** at +2 % to +36 %. It
says nothing about **masked versus opaque**. In this engine's VB raster, masked adds a UV source, a
bindless sample and `discard` to a position-only opaque VS (P38), and no borrowed number covers
that. R2 therefore measures it and pins it (§9).

### 6.3 Blended surfaces and OIT (numbers per the §6.1 rule)

"3060L est." applies the rig factor 1.33–1.6 and the resolution factor to the published 3080-Laptop
figure. It is an **estimate** and valid only for that post's scene; it orders the families, it does
not predict the owner's frame.

| Technique | Published (3080L, 1080p) | 3060L est. 1080p / 1440p / 4K (ms) | VRAM 1080p / 1440p / 4K (MB) | Fits the RHI today | Owner-rule fit | Verdict |
|---|---|---|---|---|---|---|
| Sorted premultiplied + unsorted additive (R1) | 1.37 ms | 1.8–2.2 / 3.2–3.9 / 7.3–8.8 | 0 | yes | fits every rule | **R1** |
| MBOIT-4 fp16, full res | 3.5 ms (2.55× sorted) | 4.7–5.6 / 8.3–10.0 / 18.6–22.4 | 18 B/px (`b0` R16F + moments + accumulator): 37.3 / 66.4 / 149.3 | yes (all additive) | needs `log`/`exp` op growth at R7; bounded memory | **R7** |
| MBOIT-4 fp16, half-res moments | 2.4 ms (1.75× sorted) | 3.2–3.8 / 5.7–6.8 / 12.8–15.4 | 10.5 B/px: 21.8 / 38.7 / 87.1 | yes | same | R7's first knob (unchanged from 09-10) |
| WBOIT | **no same-rig figure** | — | 10 B/px (RGBA16F + R16F revealage): 20.7 / 36.9 / 82.9 | **no**: needs a per-attachment blend slice (absent, §8.3) + `OneMinusSrcColor` (absent) + `independentBlend` (100 % Windows and Linux, §5.1) | a depth-range-tuned weight: "less distinction between layers close together … tuned once for the desired depth range" [B author, via the 09-25 lens, source [34]]; this engine's sort contract spans 15 octaves | **frozen challenger** (D-U10) |
| k-buffer Loop32, K = 4 | no figure | — | 36 B/px: 74.6 / 132.7 / 298.6 | `fragmentStoresAndAtomics` (100 % Windows, 99.77 % Linux), invisible to the census (P48) | exact to 4 layers; the OIT content has D ≫ 4 | **rejected** |
| MLAB-2 | 4.4 ms | 5.9–7.0 / 10.4–12.5 / 23.4–28.2 | ~35 MB at 1080p | interlock absent | vendor-gated | rejected (09-10) |
| PPLL | 5.52 ms | 7.3–8.8 / 13.1–15.7 / 29.4–35.3 | ~200 MB at 1080p | atomics + unbounded pool | violates the bounded-memory rule | rejected (09-10) |
| Wavelet OIT | 1.0–1.6 ms at **1440p, desktop RTX 3080**, 200 spheres [B] | 1440p: ≈ 2.3–3.6 (× 2.26–2.27 by bandwidth 760/336 and CUDA cores 8704/3840; estimate) | ~44 B/px est. | R9G9B9E5 accumulation route unverified | — | not a candidate until a primary is opened |
| AVBOIT-class froxel transmittance | not extractable | — | grid parameters unknown | 3D images ✔ | would be sized against the volumetrics design's fog-owned grid, not the 16×9×24 cull grid (§3.9) | not a candidate (UNVERIFIED internals) |
| Depth peeling / stochastic (MSAA) | — | — | — | `MIN`/`MAX` ops absent / `SAMPLE_COUNT_1` | — | rejected (09-10) |

### 6.4 Hybrid-specific options

§4.2 decides the SDF-occlusion question with numbers. Four more hybrid facts decide themselves:
- **The refraction chain sees SDF surfaces for free.** The copy runs after `sdf_forward_march` on
  every path, so glass in front of an SDF rock refracts the rock with no extra pass.
- **SDF primitives are opaque-only** (D-U11). The first revision decided this by precedent; owner
  rule 2 asks for numbers, so here is the cost and structure case.
  - **The march itself is not the obstacle.** Re-marching the interior along the refracted ray is
    common ray-marching practice (PLAUSIBLE; the critique's judgement, no source opened). Its cost
    is bounded by one more march over the covered fraction `f` of the screen: at most
    `f × T_march` per extra layer, with the same step budget. `T_march`, the marcher's own GPU zone,
    has **no measured figure in the tree** (this pass searched `docs/` and found none), so the cost
    cannot be stated in ms yet.
  - **The obstacle is ordering.** The marcher resolves one surface per pixel, the same invariant as
    the VB. A translucent SDF layer must composite per pixel between sorted mesh translucents, and
    a full-screen layer drawn at one fixed position in the sorted list cannot interleave with them.
    Only the OIT rung (R7) can absorb such a layer, and even then only as a moment contribution
    from the marcher.
  - **The decision.** D-U11 stands as a bake refusal. It reopens after R7, with `T_march` measured,
    at a cost of `f × T_march` plus one moment write per covered pixel.
- **SDF shadows stay opaque-only** (D17). One march per shadowed fragment cannot carry a
  transmittance integral without a new data structure no precedent defines.
- **The OIT family does not depend on the leg.** MBOIT moments accumulate from the translucent
  meshes and particles; the SDF leg contributes only depth (via R0b) and background colour.

## 7. Decisions taken here (technical forks, each with its deciding number or fact)

| # | Decision | Amends | The number / fact that decides it |
|---|---|---|---|
| **D-U1** | `Without<Translucent>` goes on **both** `q_ok` and `q_mat_stale` in both `cfg` variants, and on the caster query. The translucent gather is **one** query under `Disabled<MaterialStale>`; a guarded-resolution fallback (`raw ≠ 0 ∧ id == 0`) is skipped and counted | D1 | A stale translucent substituted by the default draws an alpha-1 grey silhouette (`mesh_draw.rs:1355-1356`); a translucent occludes nothing, so skipping leaves no depth/HZB/shadow hole (§3.1) |
| **D-U2** | The route's authority is the authored description at spawn. The glb importer carries `alphaMode`, `alphaCutoff`, `doubleSided`, `KHR_materials_transmission/ior/volume/specular` (TK-14). **One engine-owned glb spawn path in `boyko_render`** derives the route, inserts `Translucent` / `AlphaMasked`, and owns the image decisions (colour space, D-U7's dilated copy); the two examples call it. Masked and translucent alpha is `factor.a × texture.a × COLOR_0.a`. A `MASK` material always routes `Masked`. G3 checks Loaded rows only | D3 | the decoder drops all of those fields today (`glb.rs:945-964`, `:1169-1214`); the only spawn sites are two examples with duplicated code (`playground.rs:1578-1608`, `_hud_probe.rs:1648`), so R0a + R1 without a spawn path would still spawn `BLEND` content opaque; `COLOR_0` alpha survives import (`glb.rs:884-886`), which falsifies the first revision's "a textureless `MASK` has uniform alpha"; zero committed glb assets, so zero golden churn (§3.2) |
| **D-U3** | SDF surfaces occlude transparents on every path with an SDF leg, **per path**. **Deferred:** the `-D DEPTH_LINEAR` transparent FS compares `length(eye_rel)` against `gViewT` and `discard`s (an always-bound binding under a runtime uniform, no axis). **Forward / ForwardPlus / VB:** `sdf_depth_merge`, option (a), with the `1 + 2⁻¹⁶` discriminator, declared on a frame iff SDF leg ∧ a transparent draw this frame ∧ not a dump frame, after the HZB block | new | Deferred transparents write `SV_Depth` and have no early-Z (`particle_draw.fs.hlsl:36-42`), so (a)'s fixed 12 B/px (≈ 0.13 ms at 1440p) buys nothing there, while (b′) costs ≤ 4 B per covered px, ≤ ⅓ of it. On the reverse-Z paths early-Z rejects hidden fragments, and (a) costs ≈ 0.10 / 0.18 / 0.40 ms at 1080p / 1440p / 4K where `t` must be written, and ≈ 0.07 / 0.13 / 0.30 ms where it exists (lower bound, estimate), against shading every hidden fragment. Zero RHI growth either way (§4.2) |
| **D-U4** | The mesh sort key is CPU-only plain Rust: `raw = (bits(clamp(d, 0.125, 4096)) − bits(0.125)) >> 11`, then `raw' = clamp_i32(raw + round(sort_bias × 61 440), 0, 61 440)`, then inverted by `^ 0xFFFF`. A **NaN** depth takes the far end (`raw' = 61 440`) and is counted in `saturated_keys`. The particle key is untouched; "one body for both buckets" is withdrawn | D7, TK-7 | 15 octaves × 2²³ mantissa steps >> 11 = **61 440 ≤ 65 535**; 4096 bins per octave ⇒ **0.0122–0.0244 %** relative (vs 0.0159 % uniform for the log key), **1.95 mm at 10 m**; exact and monotone by IEEE-754 ordering, no transcendental. **`sort_bias`** keeps 09-10's meaning ("±1.0 moves an object one full depth-range", 09-10 DESIGN-SPACE §1.2) as ±61 440 key units: exact at octave granularity, within the mantissa-linear deviation (≤ 0.0861 octave) in between. **NaN:** `f32::clamp(NaN, …)` returns NaN, whose bits give `(0x7FC00000 − 0x3E000000) >> 11 = 538 624`, overflowing 16 bits, so the NaN arm is explicit. The buckets never interleave (separate draws), so no invariant needs a shared key. If the MDI rung moves the sort to the GPU, the same key is `asuint` + `uadd` + `shr_u` + `uxor` on `Cf` ops with real host arms (§3.4); its clamps would need `umin`'s host body first (P46) |
| **D-U5** | The hashed threshold keeps Listing 1's structure and changes three parts. (i) `pcg3d` replaces the sine hash. (ii) `floor(log2 x)` / `exp2(floor(…))` come from the exponent bits, and `fract(log2 x)` from the mantissa-linear form `asfloat((bits << 9 >> 9) \| 0x3F800000) − 1`. (iii) The lattice is **`asuint(floor(scale · obj))`** per component into `pcg3d`, so the only op growth is **`FieldScalar::floor`**. The mode is a uniform `{plain, hashed}`: `g_HashScale` 0.5 with TAA, 1.0 without; fade-in `n = 6` | R2 | JCGT 2020: trig "obviously bad", `pcg3d` "a good default choice" [D]. The signed `Int` ops and `select_uint` are `unreachable!` on `EvalCf` (`cf.rs:1971-1989`, `:2008-2010`, `:2021-2023`), and `float_to_uint` saturates on the host but is not the device's conversion for negatives (`cf.rs:1545-1550`). `asuint` is host-real (`cf.rs:2192`) and injective on integral floats. The signed route costs ≥ 3 items (`floor`, a signed cast, host bodies for `uint_from_int` / `float_from_int`), and it is the fallback if G20's χ² rejects float-bit inputs. The mantissa-linear form deviates from `log2(1+m) − m` by at most **0.0861** (at `m = 1/ln 2 − 1`), but it is continuous and monotone, and the CDF step needs only that the same weight is used on both sides, so the threshold stays uniform [D-arith]. Lattice and hash are bit-exact host ↔ device; only the CDF's divisions carry a ULP band |
| **D-U6** | Masked casters get a separate `With<AlphaMasked>` query pair (ok + stale, stale ⇒ default material, matching the main view's substitution) feeding `MaskedCasterScratch`; the opaque caster query gains `Without<AlphaMasked>, Without<Translucent>`. The masked caster pipelines are a **`-D MASKED` pair per depth family**: `csm_depth.{vs,fs}`, which cascades and the spot atlas share, and `punctual_depth.{vs,fs}` for points (radial `SV_Depth` + `discard`). Each masked VS carries `uv`. The caster FS uses the same threshold uniform as the view | R2 | the caster gather has no material dimension (`csm_caster.rs:251-253`); a second scratch keeps the opaque caster path byte-identical and zero-cost without masked entities. Two families exist (`csm_depth.fs.hlsl:1-11`; `gbuffer.rs:2289-2298` "the SAME caster batches"; `punctual_depth.fs.hlsl:3`, `:38`), and neither VS reads a `uv` (§3.5) ⇒ **4 `.spv`, 2 manifest rows**, where 09-10 counted 1 |
| **D-U7** | **No stored premultiplication.** Albedo stays straight alpha. At load, the engine glb spawn path (and the Gaia bake) runs a push-pull **dilation** over mip 0 that fills the RGB of `α = 0` texels from their neighbours, for images that a `Blend` or `Masked` material names as base colour. The existing gamma-space blit chain then builds the mips. The Blend FS premultiplies at output (`c_lin · a`); the masked FS uses the sampled colour as is (the first revision's division by alpha is withdrawn). The dilated copy serves only those base-colour slots: an image also named by an `Opaque` material or by any other slot (emissive included) is uploaded **twice**, original and dilated | TK-8, TK-9, D5 | Encoded-space premultiply reads **0.214 instead of 0.5** (−57 %). Linear-light premultiply at mip 0 followed by the gamma-space blit reads **−57 % at a 50 % coverage edge and −80 % at 25 %** in every mip ≥ 1 (§3.7) [D-arith]. Dilation gives **0.5 and 0.25 exactly** where colour is locally constant across the edge, and elsewhere only the opaque albedo's accepted gamma error plus a second-order colour × alpha term. **Cost:** load-time CPU O(4/3 N), estimate ≈ 10–30 ms per 2048² image single-threaded, parallel over the threadpool, transient scratch only; **0 VRAM**, except +4/3 · W · H · 4 B per image straddling routes (**22.4 MB** at 2048², **5.6 MB** at 1024²), 0 when none does. Shipped practice: Unity's `alphaIsTransparency` [D]. Option (i), a linear-light premultiplied chain, is frozen with a measured reopen criterion (§3.7, G21) |
| **D-U8** | The refraction chain's level 0 is the reflections design's `lit_prev[fi]`; `scene_color_mips` holds levels 1..k; the copy pass exists once, armed by whichever consumer is armed. **`lit_prev`'s ring depth follows the armed set**: 2 when a previous-frame reader (reflections) is armed, 1 when only refraction is. The refraction FS binds level 0 and the chain as two resources in every case, so one shader serves both. A refractive fragment with LOD ∈ (0,1) takes one extra bilinear fetch | D10, R4 | Levels 1..k are `S/3`: **2.8 / 4.9 / 11.1 MB** RGBA8. Refraction only: `S + S/3` = **11.1 / 19.7 / 44.2 MB**, 09-10's own figure. Both armed: `2S + S/3` = **19.4 / 34.4 / 77.4 MB**, which saves `S` (8.3 / 14.7 / 33.2 MB) and one full-res copy against two separate chains. Depth 1 is structurally the kind the tree already uses: a mipped graph image is "by construction the non-ringed, cross-frame kind" (`graph.rs:396`). The first revision's "costs `S` when only refraction is" charged reflections' slot to a boot with reflections off |
| **D-U9** | R6's feedback shrink is `k *= 1 − min(cov, 0.9)`; the uncited `r = 0.75` is withdrawn (09-10 ballot 12 closed) | R6 | FSR2's reactive-mask guidance "clamping the maximum reactive value to around 0.9" [D, vendor] is the only cited form; G8 still tunes it |
| **D-U10** | **MBOIT stays R7's family.** WBOIT is a frozen challenger, not expressible today (per-attachment blend absent, §8.3), reopened if R7's GPU zone shows **stage 1 ≥ one third** of R7's total on the owner fixture. k-buffer, AVBOIT-class, A2C, stochastic and a coverage-preserving mip builder are rejected or frozen as §6.2 / §6.3 state | D11, D15 | WBOIT's structural saving over MBOIT is stage 1 (the unlit moment pass), minus its own revealage target. Only a measured stage-1 share can say whether it is worth a depth-tuned weight on a 15-octave range plus the blend-slice RHI growth. The k-buffer needs 36 vs 18 B/px, is exact only to K = 4, and its niche (few-layer glass) is transmissive and drawn sorted anyway (R5) |
| **D-U11** | A material whose route is not `Opaque` on an SDF primitive is a bake refusal; reopened after R7 with the marcher's zone measured | D17 | The march is not the obstacle: a second march costs ≤ `f × T_march` over the covered fraction `f`, but `T_march` has no measured figure in the tree. Ordering is: a per-pixel SDF layer cannot interleave with sorted mesh translucents except through OIT (§6.4) |
| **D-U12** | **Fog on transparents is owned by the volumetrics design** (its D17, D18, §4.9): translucent meshes per fragment through always-bound bindings under a runtime uniform (no axis); particles per vertex at the centre via `PARTICLE_FOG` on `particle_draw.vs`; `fog_apply` precedes the single `lit_prev` copy. This design adds **no** fog variant row; the first revision's `-D FOG` rows on `forward_translucent.fs` and `particle_draw.fs` are withdrawn | new | Two same-day designs disagreed. Implementing both would fog a particle twice (VS, then FS), which squares its transmittance. UE's per-vertex artefact (P41) is a large-triangle effect, and large triangles are fogged per fragment under VOLUMETRICS D18; a particle is evaluated at its centre, like its lighting (§3.9) |
| **D-U13** | A feature an optional rung needs lands as a census `Conditional` row + a `DeviceCaps` probe, never as a `REQUIRED_*` row. The precedent is `RayQueryKHR` | new | a `REQUIRED_*` row refuses the whole boot by name (§3.3), so a disarmed rung would still cost a device. `RayQueryKHR`'s row is gated by `enable_ray_query` and still boots (`spirv_capability_census.rs:142-149`); `bindless_capable` refuses the boot (`:111-114`) and is not a precedent |
| **D-U14** | **Under `hwrt`, R2 is complete only with its RT half, R2-rt.** R2-rt packs masked instances with `VK_GEOMETRY_OPAQUE_BIT_KHR` cleared on their BLAS geometry. It also replaces `RAY_FLAG_FORCE_OPAQUE` at the three shadow query sites with a `Proceed()` loop that runs the plain cutoff on non-opaque candidates. Until R2-rt merges, masked instances are **not packed** into the TLAS, and each omission is counted | R2, 09-10 §14 | Every shadow query forces opacity (`deferred_pbr.hlsl:1113-1114`, `:1148-1149`; `vb_shadow_vis.comp.hlsl:224-225`), and every BLAS geometry is built opaque (`accel.rs:220`, `:237`). Packing masked instances today would cast full-card RT shadows. "Not packed" keeps the invariant those three sites rely on, "every packed instance is opaque", and is the rule R1 already applies to translucents. The interim gap (no RT shadow from cutout geometry on a feature-gated build) is visible and counted, where a full card would be wrong and silent. R2-rt's per-candidate cost (a UV pull + one bindless sample) is unmeasured; G23 reports it. The plain cutoff is used because a shadow ray has no pixel footprint to scale a hash lattice |
| **D-U15** | The MDI rung's `multiDrawIndirect` lands as a `Conditional` census row + `DeviceCaps` probe; devices without it keep R1's CPU-sorted path. **V9 is closed as a technical fork**, not an owner ballot | V9 | gpuinfo, filtered by platform: **99.63 %** of Windows reports and **98.16 %** of Linux reports have it, not the all-platform 82.68 % the ballot quoted (§5.1). At most 0.37 % / 1.84 % of reports lack it, which is fewer than lack the already-required `geometryShader` (0.73 % / 3.69 %), so a `Required` row would refuse little. But R1's path ships first and stays as the path below the instance line, so `Conditional` costs **no new code** and keeps D-U13's zero cost for a disarmed rung. Required buys nothing Conditional lacks |

## 8. How it lands: ECS, render graph, RHI, eDSL (the delta only)

### 8.1 ECS

| Kind | Item | Owner crate | Zero when unused |
|---|---|---|---|
| Component (ZST) | `Translucent`, `AlphaMasked` (unchanged from 09-10 §1.1) | `boyko_render` | no entity carries them ⇒ `Without<…>` excludes nothing, `With<…>` matches no archetype |
| Resource | `TranslucentRenderScratch` (09-10 §1.3; the sort-key lane now holds the D-U4 key) | `boyko_render` | inserted only when `translucent_wanted` |
| Resource | **`MaskedCasterScratch`** (new, D-U6) — the `CsmCasterScratch` newtype shape; feeds both masked caster families | `boyko_render` | inserted only when a masked caster class is armed |
| Resource | skip counters: `translucent_stale_skipped`, `translucent_fallback_skipped`, `saturated_keys` (G9 shape; NaN keys included), `masked_tlas_unpacked` (D-U14 interim, `hwrt` only) | `boyko_render` | counters in the scratch, not a side store |
| System | `gather_translucents` — one query, `Disabled<MaterialStale>`, `.after_set(AssetValidateSet)` through an `add_*` helper (the `add_gather_mesh_draws` shape) | `boyko_render` | registered only when armed |
| System | `gather_masked_casters` — the ok + stale pair, `.after_set(AssetValidateSet)` | `boyko_render` | same |
| System (debug) | G3's marker ⇔ `route()` check over Loaded rows | `boyko_render` | `debug_assertions` only |
| Importer | `GlbMaterial` gains `alpha_mode`, `alpha_cutoff`, `double_sided`, and the four `KHR` factors (TK-14) | `boyko_render::loaders::glb` | parse-time only |
| **Spawner** (new, D-U2) | the engine-owned glb spawn path: meshes, materials, images (colour space; D-U7's dilated copy), route markers, `ShadowCaster`; `playground.rs` and `_hud_probe.rs` call it instead of their copies | `boyko_render` (beside the decoder, `MeshBundle` and the texture upload) | load-time only; a scene with no glb calls nothing |
| Load step | `dilate_transparent_texels` — a push-pull fill of `α = 0` texels over mip 0, before `upload_mipped_pixels` (D-U7) | `boyko_render::texture` | runs only for images a non-opaque material names as base colour; function-local transient scratch |

Hot path: the gather, the key and the sort touch `ScratchColumn` lanes and stack counters only (09-10
§1.3). No `Vec`, no lock, no `dyn`. The spawner and the dilation are load-time.

### 8.2 Render graph: the seam, amended

```
… last opaque lit producer → [sdf_forward_march]
→ [R0b; Forward, VB; SDF leg ∧ a transparent draw this frame ∧ ¬dump frame]
      sdf_depth_merge (depth-only, SV_Depth, the path's compare op)
→ [volumetrics, when armed] fog_apply                         (VOLUMETRICS D6 / D17)
→ [R4] lit_prev copy (shared with reflections) → scene_color_mips[1..k]
→ translucent_draw / oit_* / additive_draw / particle_draw     (09-10 §2.3; on Deferred the
      `DEPTH_LINEAR` FS builds carry R0b's gViewT compare)
→ [Deferred, VB] taa_resolve → present_sample → …              (unchanged)
```

- **Ordering constraints this design owns:** merge → transparent draws; copy → transparent draws.
  `fog_apply` → copy is VOLUMETRICS D17. The relative order of the merge and `fog_apply` (depth
  versus `lit`) is VOLUMETRICS' to pin if `fog_apply` reads depth.
- **Declarators.** The merge lands in `declare_forward_graph` (`graph_bridge.rs:2631`) and
  `declare_vb_graph` (`:4120`) ahead of their `declare_particle_draw` calls (`:3030`, `:6224`).
  Deferred (`declare_deferred_graph`, `:1288`) gains no pass: its R0b is the `gViewT` binding on
  the particle draw (`:2501`) and, later, the translucent draw.
- **Forward.** R0b also adds the marcher's `viewt` write, the one the `:2984-2988` assert requires
  first.
- **Stream pins.** G6 re-measures the barrier stream per path (09-10), now also with R0b declared
  and not declared. The pass set varies per frame, as `hzb_dump` already makes it.

### 8.3 RHI gaps, per rung (checked in the code at `6394bc5e`)

| Capability | Today [T] | Needed by |
|---|---|---|
| Depth-only raster pipeline, `SV_Depth` output | ✔ (`rhi_impl/device.rs:1934-1939`; `gbuffer_mrt` writes `SV_Depth`) | R0b (Forward, VB) |
| An R32F storage-image **load** in a fragment shader, format declared in the shader | ✔ no new feature: `fragmentStoresAndAtomics` covers stores and atomics only (the spec's feature text, not re-opened by this pass); `gViewT` is already an R32F storage ring every Deferred resolve loads (`targets.rs:128-133`) | R0b (Deferred) |
| `PREMULTIPLIED_ALPHA` / `ADDITIVE` presets | ✔ (`enums.rs:913`, `:949`) | R1 |
| `vkCmdDrawIndexedIndirect` / `vkCmdDispatchIndirect` | ✔ (`device.rs:673`, `:599`) | the MDI rung; particles |
| `multiDrawIndirect` feature | field exists, **never enabled** (`ffi.rs:2880`); 99.63 % Windows / 98.16 % Linux reports (§5.1) | the MDI rung, as a `Conditional` row (D-U15) |
| `vkCmdDrawIndexedIndirectCount` | **absent** | the MDI rung |
| `discard` in the FS | ✔ licensed (`DemoteToHelperInvocation` required) | R2 |
| Non-opaque BLAS geometry + a candidate loop in `RayQuery` | BLAS geometry is built `VK_GEOMETRY_OPAQUE_BIT_KHR` (`accel.rs:220`, `:237`); every shadow query is `FORCE_OPAQUE` (§3.5); `rayQuery` itself is already a `Conditional` row | R2-rt (`hwrt` only, D-U14) |
| Push descriptor / read-only sampled depth | **absent** | R3 (TK-2) |
| Mipped image + blit | ✔ (`graph.rs:392`) | R4 |
| A non-ringed, cross-frame image | ✔ (`add_image_mipped`'s kind, `graph.rs:396`) | R4 refraction-only (D-U8) |
| `SrcColor`/`OneMinusSrcColor`/`DstColor`/`OneMinusDstColor` | **absent** (TK-1, values `2,3,4,5`) | R9 colour, `Multiply` |
| Per-attachment blend slice + `independentBlend` | **absent** (`from_fn` replication, `rhi_impl/device.rs:1921`) | only WBOIT (frozen) |
| `fragmentStoresAndAtomics` | field exists, never enabled (`ffi.rs:2897`) | only the k-buffer (rejected) |
| MSAA / A2C | **absent** (`:1865`, `:1869`) | only A2C (rejected) |
| Async compute | **absent** (one `GRAPHICS \| COMPUTE` family, `device.rs:2846`) | nothing on this ladder |
| `R32_SFLOAT` blend | UNVERIFIED; TK-13 probe | R7 `b0` |
| `COLOR_ATTACHMENT_BLEND` on the HDR `lit` format (B10G11R11, else RGBA16F) | **not probed**: RK-14 probes `STORAGE_IMAGE` + `COLOR_ATTACHMENT` only; the mandatory-table status is UNVERIFIED (§5.1) | every blended draw into `lit` once R11 = reflections R4a lands, particles included (P53) |

### 8.4 eDSL surface and manifest rows (delta)

| Leaf / op | Instantiations | Gate |
|---|---|---|
| **`FieldScalar::floor`** (new op; IEEE-exact on both sides) — the **only** op growth for R2 | `f32::floor` / HLSL `floor` | host == device on a grid including negatives and ±0 |
| `pcg3d_body` (new; `umul`/`uadd`/`uxor`/`shr_u` over `Cf::Uint`, all host-real, the `particle_rng_body` shape) | Eval + Emit | bit-exact vs a GPU readback |
| `hashed_alpha_threshold(obj, max_deriv, hash_scale, fade)` (replaces 09-10's `hashed_alpha_threshold(obj_pos, pixel_scale)`); the lattice is `asuint(floor(·))` | Eval + Emit; `max_deriv` and the LOD come from the skeleton (`ddx`/`ddy`/`CalculateLevelOfDetail` are quad ops the eDSL does not author); the `{plain, hashed}` branch is a skeleton-level uniform branch | lattice + hash bit-exact; threshold within 4 ULP; uniformity (G20) |
| `sdf_t_to_depth(t, projection constants, bias)` (new) | Eval + Emit, the reverse-Z encodings the Forward and VB rasters use | host == device; the bias discriminator on a two-surface fixture |
| the Deferred compare `length(eye_rel) ≥ gViewT` | no new leaf: the `DEPTH_LINEAR` numerator already exists in the particle FS; one compare line | G18 on Deferred |
| `dilate_transparent_texels` (new, host-only) | Rust only (load step); not an eDSL leaf, since nothing runs on the device | G21 |
| `log_depth_unit` / `sort_key` (09-10 TK-7) | **no longer on R1's path**; the mesh key is D-U4's plain Rust | — |

Manifest rows ([`SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md)):
- `sdf_depth_merge.{vs,fs}` — one row, reverse-Z only. Deferred takes no merge, so no
  `DEPTH_LINEAR` build exists.
- **`-D MASKED` `csm_depth.{vs,fs}`** — one row, 2 `.spv`, shared by cascades and the spot atlas.
- **`-D MASKED` `punctual_depth.{vs,fs}`** — one row, 2 `.spv`, point lights.
- The Deferred-only `DEPTH_LINEAR` builds of `particle_draw.fs` (and later `forward_translucent.fs`)
  gain one binding (`gViewT`). That changes an existing row's interface column, not its `.spv`
  count.
- **No fog rows here.** VOLUMETRICS owns `PARTICLE_FOG` on `particle_draw.vs`, and translucents
  take no fog axis (D-U12).

The masked VB / G-buffer / forward rows are 09-10's. The hashed policy is a uniform, not a variant.
D18's 8 + 12 bound for the two FS families stands.

## 9. The amended rung plan, with gates

Each rung carries three gates:
- a **red-first** gate that fails on today's tree for the reason the rung fixes;
- a **golden** gate: byte-identity of every default-config golden unless the rung's own re-bless is
  listed;
- a **perf** gate: a `GpuZoneRecorder` zone (`crates/boyko_rhi_vulkan/src/present/gpu_zone.rs:998`)
  or a CPU µs counter with its budget.

**Budgets.** A budget is one of three kinds, and each is labelled in the table:
- this document's **estimate**, replaced by the first measurement;
- a **borrowed** bound, used only where the source measured the same comparison;
- **report-then-pin**: the first run reports the number, and later runs regress against it, with a
  tolerance set from at least 5 repeated runs' spread rather than guessed.

A borrowed bound whose baseline differs from the gate's comparison is not used, because it could
only be red for no defect, or be replaced by its first run and then never fail (P57).

| Rung | Adds | Red-first | Golden | Perf |
|---|---|---|---|---|
| **R0a** (new) | TK-14 + the engine-owned glb spawn path (D-U2) | **G17**, asserted on **spawned archetypes**, not parsed fields, through the engine path. The glb fixture has `OPAQUE` / `MASK(0.3)` / `BLEND` / `doubleSided` / `transmissionFactor` materials, a **textureless `MASK`** primitive with a 4-component `COLOR_0`, and **one image shared** by an `OPAQUE` and a `BLEND` material. The assertions: each route spawns its marker; the vertex-alpha `MASK` spawns `AlphaMasked`; the shared image has two uploads (original + dilated). Red today: no engine path exists, and the examples spawn every part unmarked | no committed glb ⇒ no re-bless; the two examples' opaque output unchanged | load-time only; dilation ms per image reported |
| **R0b** (new) | D-U3, per path | **G18 (particles)**: on every SDF-carrying path (Deferred through the compare, Forward and VB through the merge), an alpha particle 1 unit behind an SDF sphere is **visible today** (red) and hidden after; a control in front stays drawn. **Structural checks:** on a frame with no transparent draw, and on a VB frame with `hzb_dump` set, `sdf_depth_merge` is absent from the declared pass list. The translucent-pane case moved to R1 (G18b): before R1 no translucent class exists, so a pane is an opaque mesh the composite already hides, and that half could not be red here | HZB byte-identical with the merge declared vs not; default goldens unchanged (no transparent draw ⇒ nothing declared or run); particle goldens behind SDF re-bless (V17) | **Forward / VB** (estimate): merge zone **+ Δ of the downstream transparent zones**, declared vs not, on a fixture where the merge occludes nothing, ≤ **0.3 ms** at 1440p on the 3060L (bandwidth floor 0.13–0.18 ms); a non-zero Δ is reported separately as the hierarchical-Z / compression effect. **Deferred** (estimate): Δ of the particle zone with the compare's uniform on vs off at full-screen coverage ≤ **0.1 ms** at 1440p (bandwidth floor 3.69 M × 4 B = 14.7 MB ⇒ 0.044 ms) |
| **R1** (amended) | 09-10 R1 + D-U1 + D-U4 + **D-U7** (the first rung that samples non-opaque albedo) | **G16**: a translucent with a stale material yields zero `vb_id` / G-buffer texels (red with the filter on `q_ok` only) and zero translucent draws, skip counter = 1. **G18b** (from R0b): a translucent pane 1 unit behind an SDF sphere, R0b armed, is hidden on every SDF path; the same scene with R0b force-disarmed by a test hook draws the pane over the sphere (the red control). **G19**: the D-U4 key is monotone over **every** `f32` in `[0.125, 4096]` (exhaustive, 125.8 M values); NaN, ±∞, negatives and subnormals all give keys ≤ 61 440, NaN takes the far end and counts in `saturated_keys`; `sort_bias = ±1.0` moves a mid-range key by exactly ±61 440 before the clamp. **G4 re-derived**: panes at 1.000 m and 1.014 m key to bins 12 288 and 12 345 at shift 11 (57 apart, green) and **tie at bin 48** under a shift-19 control (8-bit, red by construction) [D-arith]. **G21** (D-U7), through the real upload, blit chain and sRGB view, sampled at **LOD 1** — see the note below the table | G1 (09-10) | CPU gather + key + sort ≤ **50 µs** at 1 024 translucent instances (estimate); GPU zone reported |
| **R2** (amended) | 09-10 R2 + D-U5 + D-U6 | **G20**: `pcg3d` and the `asuint(floor(·))` lattice bit-exact on a host-vs-readback grid **including negative coordinates and `−0.0`**; threshold ≤ 4 ULP; χ² uniformity at lerp ∈ {0, 0.25, 0.5} on float-bit lattice inputs (a red here selects the priced signed-route fallback, §3.4). **G22**, in **each** of a cascade, a spot-atlas layer and a point cube face: a fence card's shadow keeps its kept-texel fraction within 5 %; the same card through the opaque caster pipeline casts the full card (red, in all three); the stale control casts the full card. **G12** (09-10) with fade-in on | default goldens unchanged (no masked entity in them) | **(i) borrowed:** hashed ≤ **1.4×** plain on the same masked batch, same pipeline, mode uniform toggled. This is Table 1's own comparison (both sides alpha-tested; its worst case is +36 %). **(ii) report-then-pin:** masked vs opaque, the same geometry with and without `AlphaMasked`. No borrowed number exists for it (§6.2, P38); the merge note records the ratio, and later runs regress against it |
| **R2-rt** (new; `hwrt` builds; completes R2 there) | D-U14 | **G23**: a fence card's `hwrt` sun shadow keeps its kept-texel fraction within 5 % of G22's cascade result; with `FORCE_OPAQUE` restored it casts the full card (red); before R2-rt the counter `masked_tlas_unpacked` equals the masked instance count (the interim is counted, not silent) | `hwrt` goldens with masked content (none today) | report-then-pin: `shadow_vis` zone Δ with vs without masked instances packed |
| R3 | 09-10 (TK-2 push descriptor) | 09-10 | 09-10 | 09-10 |
| **R4** (amended) | chain levels 1..k from the shared `lit_prev[fi]`; ring depth from the armed set (D-U8) | 09-10's refraction fixture, plus: the chain's level 1 equals a host 2×2 average of `lit_prev[fi]`; a refraction-only boot declares `lit_prev` with ring depth **1** and a reflections boot with **2** (a structural check on the declared resource) | 09-10 | chain zone ≤ **0.1 ms** at 1440p RGBA8 (estimate; bandwidth `5/3 · S` = 24.6 MB ⇒ 0.073 ms) |
| R5 | 09-10 | 09-10 | 09-10 | 09-10 |
| **R6** (amended) | `k *= 1 − min(cov, 0.9)` (D-U9) | G8 (09-10) with the `min(·, 0.9)` form; the `cov = 0` control equals TAA-without-mask | 09-10 | 09-10 |
| **R7** (re-argued) | MBOIT-4 16-bit (09-10) + a **stage-1 share** readout from two zones | G10 (09-10), which also pins the 16-bit moment format (the published figure's "fp16" is an inference, §12) | 09-10 | default-on only if R7's zone ≤ **2.0×** the sorted pass on the owner's smoke fixture at 1440p (published: 2.55× full-res, 1.75× half-res moments, so the prediction is that half-res ships); the stage-1 share decides WBOIT's reopen (D-U10) |
| R8 | 09-10 | 09-10 | 09-10 | 09-10 |
| R9 | 09-10; numbers per §3.8 | G13 (09-10) | 09-10 | +16.8 MB CSM + 4.2 MB atlas (grey) |
| R10 | 09-10; SSR comes from the reflections ladder (its R5) | 09-10 | 09-10 | 09-10 |
| R11 | = reflections R4a (one decision), **plus `COLOR_ATTACHMENT_BLEND` in RK-14's probe** (P53) | reflections design; plus: a particle blended into `lit` on the probed arm matches the RGBA8 golden within the format's quantisation | reflections design | reflections design |

**G21 in full** (D-U7). The fixture is a 2×2 texture: column 0 opaque white, column 1 transparent
black before dilation. Every tolerance is one 8-bit code, because the blit's rounding of 127.5 is
implementation-defined.
- **Masked, 50 % coverage.** Colour **1.0**, alpha **0.5**.
- **Blend.** The premultiplied output is **0.5**.
- **25 % coverage** (one opaque texel). Colour 1.0, alpha 0.25.
- **Red control 1, the undilated straight form.** It reads colour 0.212–0.216.
- **Red control 2, the first revision's mip-0 premultiply.** It reads 0.214 premultiplied, and
  0.428 after that revision's division.
- **Reopen check for option (i).** A two-colour edge (red | green opaque beside transparent) is
  compared with a host linear-light premultiplied reference. It must stay within the same texture's
  all-opaque gamma error plus one code, or D-U7's option (i) reopens.

**Order:** R0a → R0b → R1 → R2 (+ R2-rt on `hwrt` builds) → R3 → R4 → R5 → R6 → R7 → R9 → R8 →
R10, with R11 timed by the owner jointly with reflections R4a.
- R0a comes first because it is the only rung that makes imported content reach any other rung.
- R0b comes before R1 because R1's G2 ("`lit` differs only inside the instance's bounds") would
  otherwise pass on an SDF-leg boot with a visibly wrong image. That is the "green from emptiness"
  class, and a gate should not be able to go green that way. R0b's own red-first gate uses
  particles, the one transparent class that exists before R1.

**Memory at the top of the ladder, 1440p** [D-arith]:

| Item | MB |
|---|---|
| R7 (R16F `b0`) | 66.4 |
| `scene_color_mips` RGBA8 (levels 1..k) | 4.9 |
| `lit_prev` level 0, refraction only (ring depth 1, D-U8) | 14.7 |
| `taa_cov` × 2 frames in flight | 7.4 |
| R9 grey | 21.0 |
| **Total** | **≈ 114 MB ≈ 1.8 % of 6 GB** |

At 4K the same list is ≈ 231 MB ≈ 3.6 %. The first revision's 129 / 264 MB charged reflections'
second ring slot to a refraction-only boot. Memory still decides no rung. The D-U7 dilated copies
are content-dependent (+22.4 MB per straddling 2048² image) and are not in this list.

## 10. Pitfalls — the 09-25 register, corrected and extended

| # | Pitfall | Evidence | Lands on |
|---|---|---|---|
| P33 (corrected) | A stale-material translucent **leaks** into the opaque ring when the exit filter is on `q_ok` only | §3.1 [T] | D-U1, G16 |
| P34 (decided) | What a stale translucent draws as | §3.1 | D-U1: nothing, counted |
| P35 (decided) | Hashed noise on paths without TAA | paper §5.4 [D] | §6.2 policy; ballot V16 |
| P36 | "A2C works without MSAA" is false | Bevy [D] | A2C rejected |
| P37 (re-scoped) | Blit mips cannot preserve coverage **for plain cutoff** | paper §2 [D]; `texture.rs:29-35` [T] | hashed with fade-in makes it moot (§6.2) |
| P38 | Masked in a VB / meshlet raster is the expensive case | Nanite Foliage, Bevy [D] | R2 perf gate (ii): measured masked vs opaque, then pinned |
| P39 | MBOIT's half-memory quantised form needs ROVs | author postscript [D] | R7 stays SFLOAT16 |
| P40 | A reactive mask at 1.0 over-rejects | FSR2 [D] | D-U9 cap 0.9 |
| P41 | Per-vertex fog on large translucent triangles | UE [D] | VOLUMETRICS D18 fogs meshes per fragment (D-U12) |
| **P42** | SDF surfaces are absent from the seam's depth on every path | §4.1 [T] | R0b (per path), G18, G18b |
| **P43** | The glb importer silently routes every material `Opaque`, and no engine code spawns a `GlbScene` | §3.2 [T] | R0a, G17 |
| **P44** | Premultiplying sRGB-encoded values darkens by 57 % at `a = 0.5` | §3.7 [D-arith] | D-U7, G21 |
| **P45** (decided) | `hzb_dump` reads `vb_depth` after the slot, is declared last by invariant, and would dump the merged depth | `graph_bridge.rs:6289-6294` [T] | the merge is not declared on dump frames (§4.2) |
| **P46** (extended) | On `EvalCf`, `smax`, `uint_from_int`, `slt`, `sadd`, `float_from_int`, `sint_eq`, `temp_int`, `usub`, `umin` and `select_uint` are `unreachable!` | `cf.rs:1971-1989`, `:2000-2010`, `:2021-2023` [T] | D-U4, D-U5 avoid them; a leaf that needs one adds its host body first |
| **P47** | Masked casters need a material dimension the caster gather does not have | `csm_caster.rs:251-253` [T] | D-U6, G22 |
| **P48** | `fragmentStoresAndAtomics` is invisible to the SPIR-V capability census | §3.3 [T] | k-buffer frozen; any FS-store rung must gate on a validated boot |
| **P49** | The sine hash is low-quality and not bit-reproducible across host and device | JCGT 2020 Fig. 1 [D] | D-U5 (`pcg3d`) |
| **P50** | "Required feature" used for an optional rung refuses boots where the rung is off | §3.3 [T] | D-U13, D-U15 |
| **P51** | A premultiplied mip chain filtered in gamma space darkens **every** coverage edge: −57 % at 50 %, −80 % at 25 % | §3.7 [D-arith]; `texture.rs:27-35` [T] | D-U7 (straight + dilation), G21 at LOD 1 |
| **P52** | Images are shared across materials whose routes differ, so a per-image stored form serves one route wrongly | `playground.rs:1537-1547`, `:1590`, `:1642-1647` [T] | D-U7: the dilated copy serves non-opaque base-colour slots only; G17 |
| **P53** | HDR `lit` must blend, and RK-14 probes storage + attachment only | `REFLECTIONS-DESIGN-SPACE.md` RK-14 [T]; particles blend into `lit` today (`declare_particle_draw`) | a third probe bit, filed with reflections R4a; R11 row |
| **P54** | Two same-day designs each owned fog on transparents: a particle fogged in the VS and again in the FS squares its transmittance | VOLUMETRICS D18 [T] | D-U12 adopts VOLUMETRICS |
| **P55** | Every `hwrt` shadow query is `FORCE_OPAQUE`, so a packed masked instance casts a full card | §3.5 [T] | D-U14, R2-rt, G23 |
| **P56** | `float_to_uint` saturates on the host but not on the device for negatives | `cf.rs:1545-1550` [T] | D-U5's lattice uses `asuint(floor(·))` |
| **P57** | A perf budget borrowed from a table whose baseline differs from the gate's comparison (masked vs opaque priced from hashed vs plain) | paper Table 1 [D] | R2 perf gates (i) and (ii) |
| **P58** | A red-first gate half that is green before its rung because its subject class does not exist yet (a "translucent pane" before R1) | §9 R0b | G18b moved to R1 with a disarm control |

## 11. Owner VALUE questions (the ballot, updated)

09-10 ballots 1–11 and 13–15 carry unchanged. Ballot 1 is now **joint with reflections R4a** (one
HDR decision). Ballot 12 (`r = 0.75`) is closed by D-U9 as a technical fork. **The first revision's
V9 is withdrawn**: it asked the owner a technical question framed by the wrong device population,
and D-U15 decides it with per-platform numbers. New ballots:

- **V16. Distant cutout on paths without TAA** (Forward, ForwardPlus, or TAA disarmed).
  - **Default:** stable, object-anchored stipple (hashed at `g_HashScale = 1.0`, fade-in from LOD 0
    to 6).
  - **Alternative:** plain cutoff (the mode uniform's `plain` value), where foliage thins with
    distance. Mitigating the thinning needs coverage-preserving mips, which need a load-time CPU
    mip builder and a pre-mipped upload path, and conflict with hashed alpha on the same texture
    (§6.2).
  - This is a visual preference, not a performance fork: both cost zero at runtime.
- **V17. Approve the re-bless R0b causes.** Particle and transparent goldens whose billboards sit
  behind SDF surfaces change from "drawn over the SDF" (wrong) to "hidden" (right), on every path
  with an SDF leg. The set is enumerated by running G18's fixture list against the pinned goldens
  before the rung merges.

## 12. Unverified / open (delta)

- Every 3060-Laptop millisecond in §6.3 is an estimate built on one blog's scene, a spec-sheet ratio
  and a fill-bound assumption. None decides a rung; the gates in §9 replace each with a zone.
- `R32_SFLOAT` colour-attachment blend support in the mandatory format set is **UNVERIFIED** after
  four fetches; TK-13's probe covers it.
- **Whether B10G11R11 and RGBA16F guarantee `COLOR_ATTACHMENT_BLEND`** is UNVERIFIED. Revision 2
  made two more attempts: the spec table did not serve, and gpuinfo's per-format listing loads
  client-side. Until one is read, RK-14 must probe the blend bit on whichever arm it picks, and
  the degrade chain needs a last arm that keeps today's RGBA8 `lit` if neither HDR format blends
  (P53).
- **The MBOIT "fp16" format is an inference.** The post says "16bit/moment" and "additive
  blending" but names no format. A float format is the reading consistent with P39 (the quantised
  UNORM form needs ROVs) and with signed odd moments. G10 decides it.
- The claim that Vulkan does not require SPIR-V division to be correctly rounded comes from the
  spec's precision table, which this pass did **not** re-open. G20 therefore bands only the
  threshold (a CDF with divisions) and demands exactness only of the lattice and hash, which use no
  division.
- **`pcg3d` on float-bit lattice inputs.** JCGT's quality measurements feed integer coordinates.
  `asuint(floor(x))` feeds bit patterns whose high bits are structured by the exponent. G20's χ²
  decides; the signed route (≥ 3 items, §3.4) is the priced fallback.
- **The Deferred compare's orthographic arm.** On the perspective arm `gViewT` and
  `length(eye_rel)` are the same quantity (§4.2). Under ortho the `DEPTH_LINEAR` encode writes
  `position.z`, and the compare must use the marcher's parallel-ray parameter instead. That form
  was not read; G18's fixture list includes an ortho case.
- **The particles presence bound** that lets R0b's merge follow live emitters instead of the
  particles capability is owned by the particles design. Until it exists, the merge follows
  `particle_res`.
- **Hierarchical-Z and depth compression** after the merge's `SV_Depth` write are unpriced. R0b's
  perf gate measures the downstream Δ.
- Whether Forward boots allocate the `viewt` ring was not checked; if they do not, R0b allocates it
  under its arm (4 B/px: 8.3 / 14.7 / 33.2 MB).
- The projection form `sdf_t_to_depth` must invert (reverse-Z constants per path) was not read.
  The leaf takes the constants as inputs, and its oracle pins them against the raster's own depth
  on a mesh-only fixture.
- **R2-rt's cost** (a UV pull + one bindless sample per non-opaque candidate) and **`T_march`**
  (D-U11's reopen input) have no measured figure.
- AVBOIT's structure, Wavelet OIT's accumulation format, Frostbite's transparency choices and the
  hashed-alpha anisotropy term's cost on this engine's content are all still open.
- WBOIT has no published timing in any source this or the 09-25 pass opened. D-U10's reopen
  criterion is designed so that none is needed.

## 13. Overlap with the relayed request (post-FX / AA, effects, sun rays, volumetrics)

This document covers transparency. Where the relayed topics cross it:
- **Explosions, rain, smoke** are particle transparents.
  - They inherit R0b on every SDF path: smoke behind SDF terrain becomes hidden. This fixes
    PARTICLES-PLAN's recorded Deferred case and its unrecorded Forward / VB siblings.
  - They inherit R6's capped coverage mask, and R7 for dense alpha smoke.
  - Rain streaks are additive: unsorted, with no OIT needed. Under 8-bit `lit` they clip at white,
    and contributions below 1/255 vanish until HDR (R11 = reflections R4a) lands. HDR `lit` must
    then also blend (P53).
- **AA.** Transparents render inside the TAA loop and need the coverage mask (D-U9). SMAA and FXAA
  run after compositing, on 8-bit `lit`. A2C is rejected (no MSAA, §6.2). Hashed cutout converges
  only where TAA runs.
- **Bloom from emissive transparents and particles** needs HDR `lit`: the same single decision.
- **Volumetrics and sun rays.**
  - Fog on transparents belongs to the volumetrics design (D-U12, its D18). Translucent meshes are
    fogged per fragment and particles per vertex at the centre. Both draw after `fog_apply` and
    after the single scene-colour copy (its D17).
  - Fog composition needs HDR `lit` (its D6), so R11 is a prerequisite of volumetrics as well as of
    bloom and additive transparents.
  - The fog grid is the volumetrics design's own (its C1). The E-FOG constraint "fog grid == cluster
    grid" is superseded.
  - God rays via fog self-shadowing see particles only if particle density is injected into the
    volume (the volumetrics design's D16; E-PART, `docs/OPTIMIZATION-PLAN-RENDER.md:108`).
  - MBOIT moments are usable for transparent shadows (the paper's abstract, via the 09-25 lens), a
    later option for particle self-shadowing that R9 does not preclude. Grey translucent shadows
    for dynamic lights are shipped practice (§5.3).

## 14. Sources

Opened by this pass:
- [A] Wyman & McGuire, "Hashed Alpha Testing", I3D 2017 — https://cwyman.org/papers/i3d17_hashedAlpha.pdf [D]
- [B] Jarzynski & Olano, "Hash Functions for GPU Rendering", JCGT 9(3), 2020 — https://jcgt.org/published/0009/03/02/paper.pdf [D]
- [C] interplayoflight, OIT endgame — https://interplayoflight.wordpress.com/2022/07/10/order-independent-transparency-endgame/ [B]
- [D] interplayoflight, OIT part 1 — https://interplayoflight.wordpress.com/2022/06/25/order-independent-transparency-part-1/ [B]
- [E] GeForce 30 series specifications — https://en.wikipedia.org/wiki/GeForce_30_series [B]
- [F] Vulkan formats chapter (required-support table not returned) — https://docs.vulkan.org/spec/latest/chapters/formats.html [D]

Opened by revision 2:
- [G] gpuinfo core 1.0 feature coverage, Windows — https://vulkan.gpuinfo.org/listfeaturescore10.php?platform=windows [D-data]; Linux — https://vulkan.gpuinfo.org/listfeaturescore10.php?platform=linux [D-data]
- [H] Unity, `TextureImporter.alphaIsTransparency` — https://docs.unity3d.com/ScriptReference/TextureImporter-alphaIsTransparency.html [D]
- [I] Khronos `Vulkan-Docs` `chapters/formats.adoc` (returned truncated before the mandatory tables) — https://raw.githubusercontent.com/KhronosGroup/Vulkan-Docs/main/chapters/formats.adoc [D]
- [J] gpuinfo per-format `COLOR_ATTACHMENT_BLEND` listing for `B10G11R11_UFLOAT_PACK32` (rows load client-side; none returned) — https://vulkan.gpuinfo.org/listdevicescoverage.php?optimaltilingformat=B10G11R11_UFLOAT_PACK32&featureflagbit=COLOR_ATTACHMENT_BLEND&platform=windows [D-data]

Carried from the 09-25 research lens (numbering is that lens's):
- [1] https://research.activision.com/publications/2026/adaptive-voxel-based-order-independent-transparency
- [2] https://advances.realtimerendering.com/s2025/content/AVBOIT_SIG2025_MDROBOT-final.pdf
- [4] https://dev.epicgames.com/documentation/unreal-engine/nanite-foliage
- [5] https://dev.epicgames.com/documentation/unreal-engine/nanite-virtualized-geometry-in-unreal-engine
- [9] https://dev.epicgames.com/documentation/unreal-engine/using-colored-translucent-shadows-in-unreal-engine
- [10] https://dev.epicgames.com/documentation/unreal-engine/lit-translucency-in-unreal-engine
- [11] https://dev.epicgames.com/documentation/unreal-engine/volumetric-fog-in-unreal-engine
- [13] https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/rendering-execution-order.html
- [14] https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Ray-Traced-Shadows.html
- [15] https://docs.unity3d.com/ScriptReference/TextureImporterSettings-mipMapsPreserveCoverage.html
- [16] https://docs.unity3d.com/Packages/com.unity.entities.graphics@1.4/api/Unity.Rendering.DepthSorted_Tag.html
- [17] https://docs.godotengine.org/en/stable/tutorials/3d/standard_material_3d.html
- [18] https://docs.rs/bevy/latest/bevy/prelude/enum.AlphaMode.html
- [19] https://docs.rs/bevy/latest/bevy/pbr/experimental/meshlet/struct.MeshletMesh.html
- [20] https://github.com/bevyengine/bevy/pull/14876
- [21] http://www.ludicon.com/castano/blog/articles/computing-alpha-mipmaps/
- [22] https://github.com/microsoft/DirectXTex/wiki/ScaleMipMapsAlphaForCoverage
- [23] http://www.cemyuksel.com/research/alphadistribution/
- [27] https://momentsingraphics.de/MissingTMBOITCode.html
- [30] https://osor.io/OIT
- [34] http://casual-effects.blogspot.com/2015/03/implemented-weighted-blended-order.html
- [35] https://github.com/nvpro-samples/vk_order_independent_transparency
- [36] https://diligentgraphics.github.io/docs/d0/d40/DiligentSamples_Tutorials_Tutorial29_OIT_readme.html
- [37] https://github.com/GPUOpen-Effects/FidelityFX-FSR2/blob/master/README.md
- [39] https://vulkan.gpuinfo.org/listfeaturescore10.php
- [40] http://www.adriancourreges.com/blog/2016/09/09/doom-2016-graphics-study/
- [41] https://simoncoenen.com/blog/programming/graphics/DoomEternalStudy
- The 09-10 pair's own source lists (TRANSPARENCY-RESEARCH.md §12) — not re-opened.

## 15. Review log — critique pass 1 (2026-09-25)

Verdict received: CHANGES_REQUESTED, 1 critical, 10 important, 8 optional, 6 open questions. Every
remark was re-checked against the tree at `6394bc5e`, or against the fetched source, before it was
acted on. **All 19 were confirmed and adopted; none is refuted.** The critique's line numbers into
`VOLUMETRICS-DESIGN-SPACE.md` had already drifted (D17 is now `:771`, D18 `:772`). The substance
held, and that sibling is now cited by decision ID.

| # | Remark | Verified at | Resolution |
|---|---|---|---|
| C1 | D-U7's mip-0 premultiply is undone by the gamma-space blit at every coverage edge (−57 % / −80 %); G21 checks mip 0 only | `texture.rs:27-35`; arithmetic re-derived | **Fixed.** §3.7 prices both exits. D-U7 now stores straight alpha with a load-time dilation (Unity's `alphaIsTransparency` step [H]) and premultiplies at output. The masked FS's division is withdrawn. The linear-light chain is frozen with a measured reopen criterion. G21 samples LOD 1 at 50 % and 25 % coverage, with both old forms as red controls. P51 |
| W1 | VOLUMETRICS D17/D18/C1 contradict D-U12; double fog | VOLUMETRICS C1, D6, D17, D18, §4.9 | **Fixed.** D-U12 adopts VOLUMETRICS as the owner; the `-D FOG` rows are withdrawn; §3.9 and §13 are rewritten; `fog_apply` is added to the §8.2 seam. P54 |
| W2 | Per-image premultiply vs per-material route; shared images | `playground.rs:1537-1547`, `:1590`, `:1642-1647` | **Fixed.** With no stored premultiply, only the dilated copy is per-use. It serves non-opaque base-colour slots only, and an image with any other use is uploaded twice. Priced: +22.4 MB per straddling 2048² image, 0 when none straddles. G17 covers it. P52 |
| W3 | No engine glb spawn site; the textureless `MASK` uniform-alpha rule is false (`COLOR_0` alpha) | `playground.rs:1578-1608`, `_hud_probe.rs:1648`, `glb.rs:884-886` | **Fixed.** One engine-owned spawn path in `boyko_render` (D-U2, §8.1). Alpha is `factor × texture × COLOR_0`. `MASK` always routes `Masked`. G17 asserts spawned archetypes, including a vertex-alpha `MASK`. P43 extended |
| W4 | D-U5 undercounts the eDSL growth; P46 incomplete | `cf.rs:1971-1989`, `:2000-2010`, `:2021-2023`, `:1545-1550`, `:2192` | **Fixed**, via the critique's own `asuint(floor(x))` suggestion. The growth is exactly `floor`; the signed route is priced (≥ 3 items) as the fallback. P46 lists every emit-only arm; P56 covers `float_to_uint` |
| W5 | Two caster depth families; G22 covers one | `csm_depth.fs.hlsl:1-11`, `gbuffer.rs:2289-2298`, `punctual_depth.fs.hlsl:3`, `:38`, both VS `VsIn` | **Fixed.** D-U6 enumerates 2 masked pairs = 4 `.spv`, 2 rows (§8.4). G22 runs in a cascade, a spot layer and a point face |
| W6 | The R2 perf gate borrows a hashed-vs-plain bound for masked-vs-opaque | paper Table 1 (§5.1) | **Fixed.** (i) Hashed vs plain ≤ 1.4×, measured as a same-pipeline A/B through the new mode uniform. (ii) Masked vs opaque is report-then-pin. P57 |
| W7 | G18's pane half cannot be red at R0b | order in §9 | **Fixed.** G18 is particles-only at R0b; the pane case is G18b at R1, with a force-disarm red control. P58 |
| W8 | "0 per-fragment cost, early-Z" is false on Deferred | `particle_draw.fs.hlsl:36-42`; `sdf_gbuffer_composite.hlsl:1902` | **Fixed.** D-U3 is per path. Deferred takes (b′), the fused `gViewT` compare: ≤ 4 B per covered px versus a fixed 12 B/px. The composite's full-frame `gViewT` makes it a complete occluder test. Forward and VB keep (a) |
| W9 | V9 framed by the all-platform 82.68 % | gpuinfo [G], fetched by this pass: 99.63 % Windows / 98.16 % Linux | **Fixed.** V9 is withdrawn and D-U15 decides `Conditional` on technical grounds; §5.1 carries the per-platform table |
| W10 | D-U8 charges reflections' second ring slot to refraction-only boots | `graph.rs:396` | **Fixed.** Ring depth follows the armed set (1 for refraction only), with a structural gate at R4. The memory table drops from 129 to 114 MB at 1440p (264 → 231 MB at 4K) |
| O1 | D-U13 cites the refusing `bindless_capable` gate | `spirv_capability_census.rs:111-114`, `:142-149` | **Adopted.** The precedent is `RayQueryKHR` (§3.3, D-U13) |
| O2 | `sort_bias` dropped; NaN overflows the key | 09-10 DESIGN-SPACE §1.2; `(0x7FC00000 − 0x3E000000) >> 11 = 538 624` | **Adopted.** `sort_bias` is ±61 440 key units; NaN takes the far end and is counted (D-U4, G19) |
| O3 | (b) needs no push descriptor | `targets.rs:128-133` | **Adopted.** Corrected in §4.2, and it matters now that Deferred takes (b′) |
| O4 | Nothing changed for WBOIT | §8.3 | **Adopted.** §0, the D11 row, the §6.3 row and D-U10 say WBOIT is not expressible today; 09-10's premise still holds |
| O5 | G21's exact 0.502 is unreachable | blit rounding | **Adopted.** G21's tolerance is one 8-bit code |
| O6 | D-U11 decided by precedent | tree search for a marcher zone: none | **Adopted as a record.** D-U11 is re-argued: the cost is bounded by `f × T_march` (unmeasured), and ordering against sorted mesh translucents is the obstacle. It reopens after R7 with `T_march` measured |
| O7 | "R9 beyond practice" mixes grey and colour | `VFX-RESEARCH.md` §4.4 | **Adopted.** Grey is shipped practice (UE Fourier opacity maps); only colour is beyond it (§5.3) |
| O8 | "Both HZB builds" | `graph_bridge.rs:5054-5056`, `:5633-5635` | **Adopted.** One HZB block in one of two alternative slots (§4.2) |

**Open questions answered.**
1. **HDR `lit` blend.** Confirmed as a gap: RK-14 probes storage + attachment only, and particles
   already blend into `lit`. Filed as a third probe bit with a last RGBA8 arm, because the
   mandatory table could not be read (P53, §8.3, §12, the R11 row).
2. **R0b arming.** The graph is re-declared per frame (`declare_frame_graph`), so the merge follows
   presence: an exact translucent count, and a particles-owned presence bound that falls back to
   `particle_res` until it exists. Deferred's compare needs no arming (§4.2).
3. **Hierarchical-Z and compression after the merge.** Not priceable from the sources opened. R0b's
   perf gate now measures the downstream transparent zones armed vs disarmed, on an occludes-nothing
   fixture, and a conservative-depth knob is noted as UNVERIFIED (§4.2, §9).
4. **P45.** Decided: the dump is "DECLARED LAST" (`graph_bridge.rs:6289-6294`), so the merge is not
   declared on dump frames (§4.2).
5. **Masked casters under `hwrt`.** Every shadow query is `FORCE_OPAQUE` and every BLAS geometry is
   opaque. D-U14 adds R2-rt (non-opaque geometry + a `Proceed()` candidate loop, gated by G23).
   Until then masked instances are not packed, and the omission is counted.
6. **"fp16" MBOIT.** Recorded as an inference; G10 pins the format (§12, the R7 row).
