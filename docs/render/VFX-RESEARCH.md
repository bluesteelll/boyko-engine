# Visual effects (explosions, rain, weather): the survey

> Status: research, 2026-09-25, trunk `6394bc5e` (read through the `u/research-0925` worktree).
> It feeds [`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md), which is a delta over the particle plan
> ([`../PARTICLES-PLAN.md`](../PARTICLES-PLAN.md), Rev 4) and its research corpus
> ([`../PARTICLES-RESEARCH.md`](../PARTICLES-RESEARCH.md), 2026-08-20). The earlier corpus is
> not repeated here. This survey covers only what an effects layer adds on top of it.
>
> **Evidence tags.** **[M]** measured in this tree (rig named) · **[P]** primary published source
> (paper, talk, vendor documentation, engine source) · **[V]** vendor marketing claim · **[S]**
> secondary source (article, encyclopedia, forum) · **[E]** estimate or derivation, with its
> inputs shown · **[T]** read in the tree at `6394bc5e`. Code is cited as a path plus a symbol.
> This directory is not in `GATED_DOCS` (`tests/internal_docs_anchors.rs`), so no `path:line`
> here is machine-checked. The few written below were re-opened by content.
>
> **Limits of this pass.** The session's web-search budget was exhausted before this document was
> written. Everything below that is not **[T]** was either fetched directly from a known URL during
> this pass (listed in §10 as *fetched*) or carried from the upstream research report of the same
> day (listed as *carried*: the report's own fetch, not re-opened here). The following could not be
> retrieved by either pass:
> - the God of War wind talk's numbers (the GDC Vault page carries only the abstract);
> - Epic's Ribbon and Light renderer reference pages (the page body renders empty);
> - the Counter-Strike 2 smoke page;
> - Frostbite and Decima particle authoring (nothing found).
>
> No `cargo` command was run and nothing was timed for this document.
>
> **Pass 2 (after critique pass 1).** The web-search budget was exhausted again, so four more
> primary or secondary pages were fetched by known URL:
> - Kanter's tile-based rasterization analysis (§2);
> - NVIDIA's texture-atlas whitepaper (§4.1);
> - Unity's Renderer module and its standard particle shaders (§4.1);
> - Lagarde 2a, re-opened for the Remember Me map (§7.2).
>
> The Vulkan specification's SPIR-V precision table could not be read: both the Vulkan and GLSL
> pages were too large for the fetch tool. No claim below rests on it. Where operation precision
> matters, the tree's own rule is cited instead (§1.2, the eDSL row).

---

## 0. The reading in eight lines

1. **The GPU skeleton is already shipped; the effects layer is not.** Particle rungs P0, P1, P1b
   and P2 items 1–3 have landed, and P2 item 4 (soft particles) is designed. There is no colour
   ramp on the device (key 0 only), no particle flipbook, no lit particles, no events, no culling,
   no wind, no weather, no decals, no camera shake, and no lifetime component. §1 has the evidence.
2. **Shipped explosions are flipbooks, not simulations.** Engines use motion-vector-interpolated
   sheets with baked multi-directional lighting (Killzone 2 → Lozar, Star Citizen, Unity six-way).
   Epic scopes real-time 3D gas to "hero effects or cinematics" and offers baking it *into* a
   flipbook [P].
3. **Rain is a particle or layer draw, a top-down occlusion map, and a shading-time wetness
   term.** Remember Me, Wicked Engine, Far Cry 6 and Godot all build the occlusion map. Lagarde's
   porosity model is the wetness term everyone cites [P].
4. **Rain drops need no per-drop memory.** NVIDIA's 2007 sample re-spawns drops that leave a
   camera-following box [P]. Remember Me's layers cost the same "whatever the strength of the
   rain" [P]. Both avoid the emit/die bookkeeping a general particle pool pays.
5. **On the reference GPU, blended layers have two regimes, and the working set decides which
   applies, not the target size.**
   - A layer whose overdrawn region fits on chip (the 3 MB L2, plus NVIDIA's tiled caching since
     Maxwell [S, Kanter]) can approach the ROP peak of 66.6–81.7 Gpx/s.
   - A region that does not fit streams through DRAM at an effective 15.1–42 Gpx/s [S/E] (§2).
   - A compact explosion at 1080p (about 15 % of the screen, 1.24 MB of `RGBA8`) plausibly sits in
     the first regime. At that point the card draw is bound by texture rate, not fill.
   - Full-screen layers (a camera inside the smoke) and the same explosion at 4K (5.0 MB) sit in
     the second.

   Neither regime has been measured here. The upstream report's single 40 Gpx/s figure is one
   point inside the range.
6. **Fill scales down less than pixel count does.** GPU Gems 3 measured a 2.04× gain for a
   73-instruction shader at 4×4 downsampling, but only 1.14× for a 9-instruction shader at 2×2
   [P]. The particle fragment shader in this tree is one modulate plus at most one sample.
7. **`lit` is 8-bit and post-tonemap at `6394bc5e` [T]**, while every surveyed engine composites
   effects in HDR before the tonemap (DOOM: bloom and tonemap run after the particles [P]).
8. **The hybrid scene splits collision by leg.** The SDF leg already collides for free (rung P1).
   The mesh leg has no GPU representation that a particle can query: no mesh-to-SDF bake exists [T].

---

## 1. What the tree holds today [T]

### 1.1 Particle system state

| Item | Status | Where |
|---|---|---|
| Kickoff → emit → sim, all indirect; dead list + dual alive lists | landed (P0) | `crates/boyko_rhi_vulkan/src/present/passes/particles.rs` (four `cmd_dispatch_indirect` call sites; two `cmd_draw_indexed_indirect`) |
| Records: `ParticleSim` 48 B, `ParticleRender` 32 B; 92 B/particle resident, 128 B/particle/frame sim traffic | landed | `crates/boyko_render/src/particle.rs`; plan D2 and §Goal |
| Own 64 Hz clock, no `CoreSchedule::Fixed` coupling | landed | `crates/boyko_render/src/particle_clock.rs`; plan D6/D17 |
| SDF collision with a Lipschitz skip | landed (P1) | `ParticleCollision::{Off, Sdf, SdfStats}` in `crates/boyko_render/src/particle_config.rs` |
| Alpha class + FFX-shaped 8-bit radix sort | landed (P2 items 2–3) | `ParticleSortMode::{None, Radix}` |
| Deferred `-D DEPTH_LINEAR` draw pair (early-Z lost on that leg) | landed (P2 item 1) | `particle_draw_dlin.{vs,fs}.spv` |
| Soft particles (`-D SOFT`, push descriptor, view-space Z on every path) | **designed, not landed** | plan §"P2 item 4"; there is no `push_descriptor` anywhere under `crates/` |
| Colour ramp | **not rendered**: key 0 is copied at spawn and held; `color_times` is read by nothing | `crates/boyko_render/src/particle_effect.rs`, module doc "What P0 actually renders" |
| Authoring | Rust POD `ParticleEffect`, two presets (`spark`, `smoke`) | `ParticleEffectsExt` |
| One-shot spawns | **already exist on emitters**: `ParticleEmitter::burst` is added to the frame's count and zeroed by the tick that reads it | `crates/boyko_render/src/particle.rs` (`ParticleEmitter`) |
| Emit requests | A1 pushes one request row for **every enabled emitter**, including one whose spawn count this frame is 0. The `MAX_EMITTERS` release clamp therefore counts idle emitters | `crates/boyko_render/src/particle_system.rs` (the `emitters.iter_entities_mut()` loop in the A1 tick) |
| Ramp and age lanes | The sim holds `f16 inv_life_total` (`ParticleSim::size0_invlife`), not the total lifetime. Gate #14 pins **zero `OpFDiv`** module-wide in `particle_sim` | `particle.rs`; `crates/boyko_rhi_vulkan/tests/particle_edsl_sync.rs` (`particle_sim_carries_no_float_divide`) |
| `EffectParamsGpu::flags` | "Effect-level feature bits, forwarded verbatim". **No bit is defined**: emit copies the low 16 bits into the particle's `effect_flags`, and the sim reads the whole row per step anyway | `particle_effect.rs`; `particle_emit.comp.hlsl` |
| Limits | `MAX_EMITTERS = MAX_EFFECTS = 256`; `PARTICLE_DEFAULT_CAPACITY = 262_144`; `PARTICLE_SUBSTEP_CEILING = 64` | `particle.rs`, `particle_config.rs` |
| Modes | `ParticleMode::{Off, GpuUnlit}`; P3 adds `GpuLit` | `particle_config.rs` |
| Generator | `emit_particles.rs` owns **8 sources → 12 artifacts**; zero `cull`/`frustum` hits | `crates/boyko_shaderdsl/src/bin/emit_particles.rs` |

**Free lanes the effects layer can use without growing a per-particle record:**

- `ParticleRender.flags` (32 bits). The sim writes `r.flags = p.effect_flags >> 16u`. **Neither
  `particle_draw.vs.hlsl` nor `particle_draw.fs.hlsl` reads it**: the VS consumes `position`,
  `size`, `color_rgba8`, `rot_cs` and `tex_index` only.
- `ParticleRender.tex_index` (32 bits). The bindless table is capped at
  `BINDLESS_TEXTURE_CAPACITY = 4096` (`crates/boyko_rhi_vulkan/src/bindless.rs`), so an index needs
  12 bits.
- `EffectParamsGpu` (128 B per effect, not per particle) carries **five spare words**: `_r0: [u32; 2]`,
  `_r1`, `_r2` and `_r3`.

**The draw cannot reach the effect table today.** The draw's sets carry the render record and a
camera block, not `EffectParamsGpu`. Plan P2-item-4 D16 recorded this as the reason soft-particle
`fade_distance` is global.

### 1.2 Neighbouring machinery an effects layer builds on

| Mechanism | State | Where |
|---|---|---|
| **A sprite-sheet flipbook already exists, in the UI** | `UiSheet::frame_uv(index)` uses row-major cells with a half-texel inset. It divides (`col / cw`). Pinned by GPU goldens that use 16 mutually distinct frames at two tick counts | `crates/boyko_ui/src/sprite.rs`; `crates/boyko_render/tests/ui_flipbook_gpu_golden.rs` |
| Lights | `PointLight { position, color, power, range }`; `MAX_LIGHTS = 1024`; froxel grid `16 × 9 × 24`, `MAX_LIGHTS_PER_CLUSTER = 256` | `crates/boyko_render/src/light.rs` |
| Froxel light lists | Built under VisibilityBuffer only; a flat-table fallback elsewhere (the transparency design's D8) | `TRANSPARENCY-DESIGN-SPACE.md` §13.A D8 |
| Punctual shadow atlas | `SHADOW_DIM = 512` × `M_SLOTS = 16` layers | `crates/boyko_render/src/shadow_atlas.rs` |
| CSM | `MAX_CASCADES = 4`; the caster gather is `CsmCasterScratch` | `csm_config.rs`, `csm_caster.rs` |
| SDF edits | `SdfPrimitive(SdfEdit)`, **boot-static**: gathered once, `MAX_SDF_EDITS` inline (768 B = 16 edits). Dynamic per-frame edits are "a separate campaign" | `crates/boyko_render/src/sdf_edit.rs` module doc |
| Physics | `RigidBodyBundle`, `Collider { shape: ColliderShape::{Sphere, Box}, layer, mask }`, `Sensor`, `Contact { manifold, other }` (optional, queryable) | `crates/boyko_physics/src/components.rs` |
| Soft bodies (a CPU wind consumer) | `SoftBody`: SoA by axis; the solver reads a global `gravity` | `crates/boyko_physics/src/soft/component.rs`, `soft/colored.rs` |
| Events | `#[event]` with lazily-minted ids and participant/parameter substructs | `crates/boyko_macros/src/lib.rs` (`pub fn event`) |
| Time | `Time::{delta_secs, elapsed, relative_speed, is_paused}` | `crates/boyko_ecs/src/ecs/core/time/time.rs` |
| Shader camera block | Lit producers bind the 80 B `CompositePushConstants`; `cam_eye.w` and `cam_up.w` are free. `ViewUniform` is **not** what shading tails bind (correction taken from the sibling dynamic-materials survey, re-checked here) | `crates/boyko_rhi_vulkan/src/compute.rs` (`CompositePushConstants`); `crates/boyko_scene/src/camera.rs` (`ViewUniform`) |
| Blend states | `BlendState::{PREMULTIPLIED_ALPHA, ADDITIVE}`; `BlendFactor::{Zero, One, SrcAlpha, OneMinusSrcAlpha}` only, so there is **no multiply blend** | `crates/boyko_rhi/src/enums.rs` |
| eDSL `Cf` node set | Now includes `sin`, `cos`, `rsqrt`, `vec3_dot`, `vec2_frac`, `vec2_lerp`, `vec2_smoothstep`, `float_to_uint`, `float_from_uint`, and `ushl`/`shr_u`/`uxor`/`uor`/`and_u` | `crates/boyko_shaderdsl/src/cf.rs` |
| eDSL `FieldScalar` (the physics-reachable algebra) | `add sub mul div neg min max clamp01 lerp abs sqrt select` plus compares: **no `floor`, no trig** | `crates/boyko_shaderdsl/src/scalar.rs` |
| eDSL reach into physics | `boyko_physics → boyko_sdf_math → boyko_shaderdsl` is a live edge (the brick decode's `f32` path) | `crates/boyko_physics/Cargo.toml`; `crates/boyko_rhi_vulkan/Cargo.toml` comment on that edge |
| eDSL float parity | Float leaves match their `f32` oracle "modulo FMA contraction", the crate's standing carve-out. Only integer leaves (`particle_rng_body`) are bit-exact by construction. A divide is never part of a bit-exact contract (M7: `OpFDiv` carries 2.5 ULP) | `crates/boyko_shaderdsl/src/particle.rs` module doc; `ParticleSim::rot_cs` doc |
| Texture mips | Mip chains are built by LINEAR blit over the whole image (`texture.rs` module doc); the level count is `TextureDesc::mip_levels` | `crates/boyko_render/src/texture.rs`; `crates/boyko_rhi/src/device.rs` |
| The bindless sampler | One shared immutable sampler: trilinear, anisotropy 16, max LOD unclamped. So a per-texture level cap can only come from the image's own level count | `crates/boyko_rhi_vulkan/src/bindless.rs` |
| Rain-occluder selection | The CSM caster gather is a structural `With<ShadowCaster>` query. Render does not depend on physics; physics marks bodies with `RigidBody` plus the `Simulated`/`Kinematic` bitset tags | `crates/boyko_render/src/csm_caster.rs` module doc; `crates/boyko_physics/src/components.rs` |
| Lit-producer descriptor sets | `deferred_pbr` uses one flat set 0 (bindings 0–23). The VB, Forward and SDF-march producers share a shadow set 1. The VB producers also bind the bindless mesh buffers at set 2 (`vb_geom_fetch.hlsli`) and bindless textures at set 3 (`vb_shade`). So **no set index below 4 is free in every producer**. `deferred_pbr` already declares its HWRT bindings entirely under `#if HWRT` in its own set 0, with a per-variant layout ("the 20th resolve descriptor the HWRT layout adds atop the 19 the software resolve uses") | `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl` (binding 19's two arms), `vb_geom_fetch.hlsli`, `vb_shade.comp.hlsl` |

### 1.3 The RHI, verified in `crates/boyko_rhi_vulkan/src/device.rs`

| Capability | State |
|---|---|
| `vkCmdDispatchIndirect`, `vkCmdDrawIndexedIndirect` | loaded and used |
| `vkCmdDrawIndexedIndirectCount` | **not loaded**. The field doc on `cmd_draw_indexed_indirect` says it "needs `drawIndirectCount` in a `VkPhysicalDeviceVulkan12Features` this device never chains" |
| Push descriptors | **absent** (zero hits); plan D13 decided to enable `VK_KHR_push_descriptor` |
| `independentBlend` | only the FFI field exists; `REQUIRED_CORE` enables just `samplerAnisotropy` and `geometryShader` |
| `textureCompressionBC` | **not enabled**. `boyko_rhi::enums::Format` has 19 variants, none block-compressed (`R8G8B8A8{Unorm,Srgb}`, `B8G8R8A8{Unorm,Srgb}`, `R8{Snorm,Unorm}`, `R8G8Unorm`, `R16Sfloat`, `R16Unorm`, `R16G16{Unorm,Sfloat}`, `R16G16B16A16{Unorm,Sfloat}`, `B10G11R11UfloatPack32`, `R32Sfloat`, `R32G32Uint`, `R32G32B32Sfloat`, `D32Sfloat`) |
| `TextureDimension` | `{D2, D3}` only. **The upstream report's "`D2Array`" does not exist.** Array layers ride `array_layers` on the texture descriptor, but the bindless table is `Texture2D gTextures[]`, so a flipbook is a 2D atlas |
| Queues | one queue family requiring `GRAPHICS \| COMPUTE`; no async compute; no timeline semaphores |
| `shaderInt64` | not required, so there are no 64-bit atomics |
| `vertexPipelineStoresAndAtomics`, `fragmentStoresAndAtomics` | FFI fields only (`ffi.rs`); not in `REQUIRED_CORE`. **A vertex shader cannot write a buffer here** |
| Fragment shading rate (VRS) | zero hits under `crates/` |
| Format queries | `vkGetPhysicalDeviceFormatProperties` is loaded (the Render P1b probe). `Format` carries `R32G32Uint` and `R32Sfloat` |
| Frame graph | `add_image_mipped`, `add_image_seeded` and `add_buffer_seeded` exist; there is no transient aliasing |

### 1.4 `lit` is LDR

`lit` is an `R8G8B8A8` storage ring ("the deferred resolve's OUTPUT", `present/targets.rs`, field
`lit`), written post-tonemap. The consequences are carried from the plan: additive clips at white,
and contributions below 1/255 round away (plan D7 trade-off, Open Question 1). The transparency
design (R11, ballot 1) and the reflections design (R4a) both hold the HDR move as an owner
decision.

### 1.5 Verified absent (grep over `crates/**/*.rs`, false hits inspected)

Decals · wind field · weather · camera shake / trauma · a lifetime or despawn component (the only
`Lifetime` hits are prose about Rust lifetimes) · audio · a top-down occlusion map · mesh SDFs ·
BC formats · per-particle culling · particle flipbooks · a TAA reactive or coverage input (no
`reactive`, `particle` or `responsive` in `taa_resolve.comp.hlsl`) · foliage.

### 1.6 Measured particle costs [M]

The particle plan's gate #17 was measured on an RTX 3060 Laptop on the VB path, with billboard
half-extent `particle_scene::billboard_size()`. Its section is flagged **SUSPECT**, because the
20 warm-up frames were folded into the medians.

| alive | kickoff | emit | SIM | ns/particle (SIM) | DRAW |
|---|---|---|---|---|---|
| 10 240 | 4 960 ns | 3 072 ns | 17.4 µs | 1.700 | 37.9 µs |
| 65 536 | 5 120 ns | 3 200 ns | 72.7 µs | 1.109 | 106.5 µs |
| 102 400 | 4 768 ns | 3 072 ns | 102.4 µs | 1.000 | 158.7 µs |
| 1 048 576 | 5 888 ns | 3 072 ns | 1 110 µs | 1.059 | 1 404 µs |

The live compute budget is **`9 µs + 1.10 × (128 B × N) / 121 GB/s`** (plan §Goal). There is **no
large-billboard fill measurement**: every DRAW row is at a billboard size of a few pixels, so the
rows measure vertex and raster setup (≈ 1.55 ns/particle at 102 400), not blending.

---

## 2. The reference GPU and the fill model

**RTX 3060 Laptop (GA106)** [S, Wikipedia "GeForce 30 series", derived from vendor clocks]:

| Property | Value |
|---|---|
| Configuration | 3840 shaders, 120 TMUs, **48 ROPs**, 30 SMs |
| L2 | **3 MB** |
| Clocks | 817–1387 MHz base, 1283–1703 MHz boost (TGP 60–115 W) |
| Memory | 6 GB, 192-bit, **288–336 GB/s** |
| Peak pixel rate | 66.6–81.7 Gpx/s (48 ROPs × 1.387–1.703 GHz) |
| Peak texture rate | 166.4–204.4 Gtex/s (120 TMUs × the same clocks) |
| FP32 | the source lists 6.9–10.9 TFLOPS; **at the clock pair above it is 10.65–13.08 TFLOPS** (3840 × 2 × 1.387–1.703 GHz) [E] |

The source's FP32 range cannot be reproduced from its own clock columns, which pass 1 did not
notice. The pixel and texture rows follow from 1.387–1.703 GHz. The design's single FLOP-priced
estimate (the SDF strip march) therefore carries the whole span, 6.9–13.08 TFLOPS, and treats the
listed 6.9 as a pessimistic sustained floor.

**The derivation [E]: two regimes, decided by the working set.**
1. A blended `RGBA8` fragment reads and writes 4 B each at the ROP, so 8 B per fragment.
2. **Region off chip.** When the overdrawn region is larger than what stays resident on chip, a
   layer streams through DRAM. Examples are a full-screen layer (8.3 MB at 1080p) or a 4K
   explosion (15 % of 8.29 Mpx × 4 B = 5.0 MB). Both exceed the 3 MB L2. The effective blend rate
   is then bandwidth / 8 B:
   - 121 GB/s (the effective streaming rate measured on this part at gate #17 [M]) gives
     **15.1 Gpx/s**;
   - 336 GB/s (peak) gives **42 Gpx/s**.
3. **Region on chip.** When the overdrawn region fits, successive layers re-hit L2, and the ceiling
   is the ROP peak, **66.6–81.7 Gpx/s**. An example is one compact explosion at 1080p (15 % of
   2.07 Mpx × 4 B = 1.24 MB). NVIDIA GPUs since Maxwell also buffer rasteriser output in on-chip
   tiles "to ensure that the pixel output from rasterization fits within a fixed size on-chip
   buffer or cache" [S, Kanter, RealWorldTech 2016, fetched; measured on GTX 970 and GTX 1070, not
   on Ampere]. Textures sampled by the same cards compete for the same L2, so how close a real
   draw gets to this ceiling is unmeasured.
4. At 1440p the same explosion is 2.2 MB, which is borderline.
5. An `R16G16B16A16_SFLOAT` target doubles the bytes and halves both rates.

**Cost of one screen of blended coverage** (one full-screen layer, which is always the off-chip
regime at these resolutions):

| Resolution | Pixels | Time at 42 Gpx/s | Time at 15.1 Gpx/s |
|---|---|---|---|
| 1080p | 2.07 M | 0.049 ms | 0.137 ms |
| 1440p | 3.69 M | 0.088 ms | 0.244 ms |
| 4K | 8.29 M | 0.198 ms | 0.549 ms |

For an on-chip region, the same fragment count costs 0.025–0.031 ms per screen-equivalent at
1080p, at 66.6–81.7 Gpx/s.

**What this does to the design's numbers.** For compact, textured cards in the on-chip regime,
fill (≥ 0.13 ms for 5.12 screen-layers at 1080p) drops *below* texture sampling (0.29–0.35 ms for
28.16 screen-samples). So the card draw becomes texture-bound, not fill-bound. For full-screen or
4K layers, fill dominates.

A linear model calibrated on single untextured layers cannot tell these cases apart. The design's
first rung therefore measures stacking depth over a fixed region and textured cards separately,
not coverage alone (design FX0).

---

## 3. Authoring models

| Engine | Model | What an effects layer here can take from it |
|---|---|---|
| **UE Niagara** [P] | System → Emitter → Module stack. Namespaced attributes. Spawn and Update stages; **Simulation Stages** are multi-pass GPU iteration and underlie Niagara Fluids. Data interfaces. Effect Types for scalability, component pooling, "fixed bounds will be more performant than dynamic bounds". **GPU emitters get no events**: "Events only work with CPU simulation" | Effect-type budgets; pooling; and the warning that per-effect GPU programs multiply dispatches ("1 particle can cost as 64", carried from `PARTICLES-RESEARCH.md`) |
| **Unity VFX Graph** [P] | Spawn → Initialize → Update → Output contexts. **GPU events**: Trigger Event On Die / Rate / Always, executed "at the end of Update" into another system. Output switches: `Indirect Draw`, `Compute Culling`, `Frustum Culling` in a compute pass, `Use Soft Particle`, `Generate Motion Vectors`, `Exclude From TU And AA`, UV modes `FlipbookBlend` / **`FlipbookMotionBlend`** | GPU-side spawn-on-death; culling in the sim; per-output feature switches |
| **Destiny** (SIGGRAPH 2017) [P, abstract] | Node graphs whose parameters are expressions; an expression→HLSL converter plus a bytecode interpreter that runs on CPU and GPU | The eDSL's one-body/two-instantiations is the zero-overhead form of the same parity idea (`PARTICLES-RESEARCH.md`) |
| **The Last of Us Part II** (SIGGRAPH 2020) [P, abstract] | GPU-driven effects that spawn from world information and attach to animated geometry; wetness and blood on characters | Effects reading world data (depth, SDF, occlusion map) |
| **PopcornFX** [V] | A runtime middleware for custom engines, with a C++ SDK | Rejected on rule 5 (third-party runtime) |
| **Wicked Engine** [P, source] | Effect features are option bits on **one** simulation shader. `WeatherComponent` holds `rain_amount = 0`, `rain_length = 0.04`, `rain_speed = 1`, `rain_scale = 0.005`, `rain_splash_scale = 0.1`, `rain_color = (0.6, 0.8, 1, 0.5)`, `windDirection`, `windRandomness = 5`, `windWaveSize = 1`, `windSpeed = 1`. No snow or wetness fields | The shape of the effect table that D12 already chose: one skeleton, lanes per feature |
| Frostbite, Decima | No primary source on particle authoring was found | — |

---

## 4. Rendering techniques

### 4.1 Sprites, stretched billboards, flipbooks

- **Velocity-stretched billboards** (Wicked `xParticleMotionBlurAmount`) scale the quad along the
  view-space motion vector [P source, carried]. This is the standard spark and rain-streak shape.
- **Motion-vector flipbook interpolation** [P, Lozar, fetched]:
  - Origin: "first developed by Guerrilla Games for Killzone 2".
  - The current frame is warped toward the next by the motion vectors; the next frame is warped
    backward; the two are cross-faded by the frame fraction.
  - The motion-vector texture "must be uncompressed". Lozar paired a 512² motion-vector map with a
    4096² colour sheet, so the motion vectors tolerate heavy downsampling.
  - Gain: playback extended "more than 10 times while keeping the same perceived frame rate".
  - Cost: two colour samples plus two motion-vector samples per fragment.
  - Star Citizen: 32 frames extended "by over 30 times" [S, 80.lv, carried].
  - Unity ships it as `FlipbookMotionBlend` [P].
- **Baked multi-directional lighting ("six-way")** [P, Unity blog, fetched]:
  - Six directional lightmaps packed into two RGBA textures; the first texture's alpha is
    transparency, the second's alpha is optional emissive.
  - "Comparable to a traditional lit sprite" in cost.
  - Probes are evaluated **in the vertex shader** and interpolated.
  - Limits: flat under received shadows; "reflection probes do not affect particles"; lightmaps do
    not account for occlusion by neighbouring particles.
  - Star Citizen bakes five directions plus a normal map and a temperature map coloured by a
    gradient [S, carried]. EmberGen exports these maps [V].
- **Mipmapped atlases leak between cells** [P, NVIDIA SDK whitepaper WP-01387-001, "Improve
  Batching Using Texture Atlases", 2004, fetched; text extracted from the PDF]. Three findings
  apply to flipbook sheets:
  - A 2×2 box filter over a power-of-two atlas "does not pollute mip-maps with neighboring texels,
    if the atlas is a power-of-two texture and contains only power-of-two textures that do not
    unnecessarily cross power-of-two lines". Pollution starts at the level where a cell would have
    to shrink below one texel. So a chain needs only as many levels as the cells have ("a 1kx1k
    atlas containing 16 256x256 textures should only store 8 mip-levels").
  - A separate problem survives even a clean chain: "bilinear and anisotropic filtering of all
    lower mip-maps also access unrelated neighboring texels", because a fixed half-texel inset at
    level 0 is not half a texel at level k.
  - Its two cures are a shader clamp "taking into account which mip-level the texture operation is
    about to access" (which it calls comparatively expensive) and border padding per level (which
    "quickly wastes texture-memory").

  The in-tree `UiSheet` precedent is an unmipped UI sheet, so it never met this problem.
- **Screen-coverage bounds on particles are shipped controls.** Unity's Renderer module has "Min
  Particle Size" and "Max Particle Size", each "expressed as a fraction of viewport size" [P,
  fetched]. Unity's standard particle shaders have **Camera Fading**: "Near fade – the distance
  from the camera where particles are completely faded away; Far fade – the distance from the
  camera where particles start to fade as they get nearer" [P, fetched]. Neither page states a
  cost or a default.

### 4.2 Soft particles and depth

Lorach (NVIDIA 2007) fades sprites by the scene-depth difference [P, carried]. The in-tree design
(plan P2 item 4) has already chosen: a pushed depth descriptor, view-space Z on every path, two FS
variants, and an orthographic sentinel. No new research changes it.

### 4.3 Lighting families for particles

| Family | Where lighting is evaluated | Shipped by | Cost shape |
|---|---|---|---|
| Per particle, at the centre, in the sim | once per particle | the in-tree P3/D11 design | ≈ 100k evaluations vs 5M per-fragment (plan D11) |
| Per vertex | 4 per quad | Unity six-way probes [P] | 4 per card; low frequency across the card |
| Decoupled lighting atlas | fixed-size tiles in a 4k atlas, tile size by distance | DOOM 2016 [P, Courrèges, fetched]: "Regardless of the resolution you're playing at … particle lighting is always computed and stored in these tiny fixed-size tiles" | tile² evaluations per sprite, independent of screen resolution |
| Low-resolution lighting volume | cascaded view-frustum volumes, default 64³; "raising this by a factor of 2 increases the cost … by a factor of 8" | UE lit translucency [P, carried] | a volume per frame |
| Voxelize into the froxel extinction volume | the fog volume itself, with volumetric shadow maps | Frostbite 2015 [P, abstract], UE Volume domain [P, fetched: particles write "Albedo, Emissive, and Extinction"] | shared with the fog |

### 4.4 Particle shadows

- UE casts translucent shadows through Fourier Opacity Maps. They are good for "blobby volumes",
  show "severe ringing" on opaque-ish translucency, and need fixed bounds [P, carried].
- Frostbite uses volumetric shadow maps [P, abstract].
- Six-way bakes the self-shadowing into the texture.
- The in-tree transparency design's R9 (`csm_trans`, grey transmittance) is the caster route this
  engine already plans.

### 4.5 OIT, sorting, TAA

- In-tree decisions stand: FFX radix for the alpha class (landed), MBOIT-4 as the OIT rung
  (transparency R7), and a TAA coverage mask `taa_cov` (transparency R6).
- AMD FSR2: particles are where "writing motion vectors might be prohibitive"; "write alpha to the
  reactive mask" with the maximum clamped "to around 0.9" [P, fetched].

### 4.6 Off-screen (low-resolution) particles [P, GPU Gems 3 ch. 23, fetched]

Measured on a GeForce 8800 GTX at 1600×1200:

| Shader | Full res | Mixed 2×2 | Low 2×2 | Mixed 4×4 | Low 4×4 |
|---|---|---|---|---|---|
| 73 instructions | 25 fps | 44 | 46 | **51** | 61 |
| 41 instructions | 36 fps | 47.6 | 53.5 | — | — |
| 9 instructions | 44 fps | 50 | 57 | — | — |

- Depth is downsampled by taking the **maximum** of the samples ("an expedient hack", which
  shrinks silhouettes and removes halos).
- Edges are found with a Sobel filter, stencil-masked, and re-rendered at full resolution.
- **The gain shrinks as the shader gets cheaper.** This is why §0 item 6 matters: the particle FS
  in this tree is tiny, and the flipbook and six-way features are what would make it heavy.

### 4.6b Variable-rate shading for large cards [P, Microsoft DirectX VRS specification, fetched in pass 2]

- The specification lists "transparencies" among the content where a reduced rate is acceptable.
- Tier 1 sets the rate "on a per-draw-basis"; Tier 2 adds a per-primitive rate and a screen-space
  rate image.
- **What it saves, and what it does not.** Coarse shading runs one fragment-shader invocation per
  2×2 (or larger) block. That cuts the shader's texture samples and ALU. It does **not** cut blend
  work, because the output-merger still blends every covered pixel. So it helps the on-chip,
  texture-bound regime of §2, and does nothing for the off-chip, bandwidth-bound regime.
- The Vulkan form is `VK_KHR_fragment_shading_rate`, which has zero hits in this tree (§1.3).
- No shipped particle measurement was retrieved.

### 4.7 Distortion

- Offsets accumulate in a screen buffer, which then warps a scene-colour copy [P, Froyok,
  carried]. The pitfalls are foreground bleeding, edge clamping and lost layering.
- UE: "only opaque materials can be refracted properly" [P, carried].
- The in-tree transparency design's R4 builds the opaque colour mip chain and R5 builds the
  `-D REFRACT` translucent FS. Those are the scene-colour copy and the refraction sample.

---

## 5. Collision, events, budgets

### 5.1 Collision

| Method | Evidence | Limits |
|---|---|---|
| SDF (in-tree P1) | `PARTICLES-RESEARCH.md` TL;DR 4 | only the SDF leg has a field; `MAX_SDF_EDITS = 16`, boot-static |
| Previous-frame depth buffer | Wicked source [P, fetched]: reprojects with `previous_view_projection`, `surfaceThickness = 1.5`, normal from `cross(p2 − p0, p1 − p0)`, reflects times restitution | off-screen loss (UE [S]); self-collision with opaque particles (Unity [P]); this tree has two depth encodings per path (plan D7/D14) |
| Top-down height field | Godot `GPUParticlesCollisionHeightField3D` [P, fetched]: resolutions 256²…8192², default 1024²; update "when moved" by default, "Always" has "a significant performance cost"; "heightmaps cannot represent overhangs (e.g. indoors or caves)"; follow-camera forces an update whenever the camera moves | overhangs; dynamic objects |
| Hardware ray tracing | UE: "one frame behind", falls back to the global SDF [P, carried] | `hwrt` is an optional feature here |
| Analytic colliders | Godot: up to 32 per frame (`PARTICLES-RESEARCH.md`) | a small table |

### 5.2 Events

- Unity runs GPU events at the end of Update into another system.
- Niagara GPU emitters have none and read other emitters' attributes instead.
- The plan's D3 forbids concurrent push and pop on the dead stack, so a same-pass spawn is
  excluded by construction [T]. An event can only spawn in a later emit.

### 5.3 Culling and budgets

- Wicked frustum-tests a sphere in `simulateCS`, appends to `culledCount` with `InterlockedAdd`,
  and writes a negated squared distance as the sort key [P, fetched].
- Niagara's Effect Types and pooling [P].
- Unity: `capacity ≈ spawn_rate × lifetime` is the field rule (`PARTICLES-RESEARCH.md`). That rule
  is also a readback-free way to predict the alive count on the host.

---

## 6. Explosions in shipped practice

| Component | Practice | Evidence |
|---|---|---|
| Fireball / smoke core | motion-vector flipbook plus six-way (or five-direction + normal + temperature) lighting | Lozar [P], Unity [P], Star Citizen [S] |
| Hero volumetric | real-time 3D gas: density, temperature and velocity per cell; ray-marched; baked shadows by stepping the volume. "2D simulations are more efficient and better suited for games", "3D … higher memory and GPU cost … hero effects or cinematics"; "can also be baked into a flipbook". No default resolution or cost is stated | Niagara Fluids [P, fetched] |
| Baked 3D density playback (between flipbooks and live gas) | Imported OpenVDB sequences as Sparse Volume Textures, ray-marched, with frame playback and looping. The feature is "**Experimental** … use caution when shipping with it", and streaming animated frames "is not recommended for real-time playback". No per-frame cost is stated | UE Heterogeneous Volumes [P, fetched in pass 2] |
| Classic GPU fluid fire | reaction coordinate mapped through an artist 1D texture; obstacles voxelized per slice; shipped in Hellgate: London | GPU Gems 3 ch. 30 [P, carried] |
| Soft volumes that receive light and shadow | particle density injected into froxel fog | UE Volume domain [P], Frostbite [P, abstract] |
| Shockwave / heat | screen-space offsets warping a scene-colour copy | Froyok [P, carried] |
| Scorch marks | clustered-forward decals: "up to 256 lights, 256 decals and 256 cubemaps" per cluster, atlas-stored, applied while meshes render | DOOM [P, Courrèges, fetched] |
| Flash | a short-lived dynamic light. UE warns that fog temporal reprojection makes "muzzle flashes, leave lighting trails"; the workaround is a volumetric scattering intensity of 0 on that light | UE Volumetric Fog [P, fetched] |
| Camera shake | trauma in [0, 1], `+= 0.2 or 0.5` per hit, linear decay, shake = trauma² or trauma³; Perlin noise because it "automagically works with pause and slow-motion"; in 3D, rotational only | Eiserloh GDC 2016 [P, fetched] |
| Debris | rigid bodies (interactive) or mesh particles (cosmetic) | Niagara mesh renderer (`PARTICLES-RESEARCH.md`); no primary cost source found |
| Audio | no primary source retrieved; the engine has no audio subsystem, and the sibling audio survey of the same date (`docs/audio/AUDIO-RESEARCH.md`) covers it | — |

---

## 7. Rain and weather in shipped practice

### 7.1 Streaks

**Remember Me (Lagarde)** [P, fetched]:
- "Four layers of rain" on a camera-linked cylinder or cone, with one pre-motion-blurred drop
  texture per layer.
- The first two layers are low-resolution, with soft depth testing.
- Cost at 1280×720: PS3 **0.40 + 1.29 = 1.69 ms**, X360 **0.34 + 1.38 = 1.72 ms**.
- "Timings are the same whatever the strength of the rain."

**NVIDIA "Rain" (Tariq 2007)** [P, fetched, primary PDF]:
- Drops are animated with Stream Out, expanded by the geometry shader, and shaded from a texture
  array of more than 300 drop appearances indexed by view and light angles (Garg & Nayar 2006).
- The camera position is used "to re-spawn any rain particles that have fallen out of bounds".
- GeForce 8800 GTX at 1280×1024, "simple shader":

| Drops | Animation fps | Extrusion + raster fps |
|---|---|---|
| 200 000 | 574 | 545 |
| 1 000 000 | 257 | 126 |
| 5 000 000 | 67 | 26 |

- Its own trade-off: layered textures are "fast but … can look like they lack depth" and cannot
  "exhibit complex dynamics like stormy wind, or respond to local lighting".

**Wicked Engine** [P, source]: GPU particles driven by the `WeatherComponent` rain fields (§3).

**Far Cry 6** [P, Ubisoft, fetched]: "GPU particles"; no counts or budgets published.

### 7.2 Occlusion under roofs: one top-down map, everywhere

| Engine | Map | Use |
|---|---|---|
| Remember Me [P, fetched; re-opened in pass 2] | "256×256 depth map and a 20m x 20m orthogonal frustum" (7.8 cm cells) placed in front of the camera; PS3 0.32 ms, "mainly dominated by character", so **dynamic casters are in the map**; X360 0.20 ms; the transfer to the CPU costs 0.036 ms on PS3. Outside the map: "If we are out of shadowmap, just kill the pixel". The post does not state an update frequency | splash positions: project, reconstruct NDC Z, unproject |
| Wicked [P, source, fetched] | `rain_blocker_matrix_prev` into a shadow-atlas slot. If `shadow > shadow_pos.z`: `velocity = 0`, the position moves onto the blocker plus half the splash size, `sizeBeginEnd = splashSize`, `life = 0.15` | the drop **becomes its own splash**, on the GPU |
| Far Cry 6 [P, fetched] | "a wetness shadow map, which masked out wetness when objects were occluded"; dynamic objects (weapons, vehicles, characters) use "ray casts to detect rain exposure" | wetness exposure |
| Godot [P, fetched] | a height-field collider, 1024² by default, updated when moved | particle collision |

### 7.3 Splashes, ripples, puddles, wetness

- **Splashes.** Remember Me: under heavy rain PS3 0.33 ms, X360 0.25 ms. Its intensity presets are
  (0.33, 20 splashes), (0.66, 40) and (1.0, 60) [P].
- **Ripples** [P, Lagarde 2b, fetched]: a 256² normal map built from four ring layers with
  `TimeMul = (1.0, 0.85, 0.93, 1.13)` and `TimeAdd = (0.0, 0.2, 0.45, 0.7)`, enabled progressively
  by intensity. PS3 0.14 ms, X360 0.15 ms.
- **Puddles** [P, Lagarde 2b]:
  - a vertex-colour mask (0 means puddle);
  - `AccumulatedWaters.x = min(FloodLevel.x, 1.0 − Heightmap)` for water in cracks;
  - the puddle margin is `saturate((FloodLevel.y − VertexColor.g) / 0.4)`.
- **Far Cry 6 puddles** [P]: a terrain decal whose "gradient served as a pseudo signed distance
  field, allowing puddles to build from the center", plus "procedural simulations for rain and
  wind ripples".
- **Wet surfaces** [P, Lagarde 3b, fetched]:
  - `factor = lerp(1, 0.2, (1 − Metalness) · Porosity)`;
  - `Diffuse *= lerp(1.0, factor, WetLevel)`;
  - `Gloss = lerp(1.0, Gloss, lerp(1, factor, 0.5 · WetLevel))`;
  - the normal blends toward the vertex normal as the water layer thickens;
  - "only few extra instructions (but this still two textures fetch for the blending)", and it
    fits deferred shading.
- **Whole-system budget**: Remember Me's rain was about **2.8 ms on PS3 and 2.86 ms on X360 at
  720p** [P]. This includes camera droplets at 0.32 / 0.54 ms, drawn "in view space … on near plane".

### 7.4 Snow

Batman: Arkham Origins deformable snow [P, GDC 2014 transcript, fetched]:
- An "ankle-high orthogonal frustum"; actors are rendered into a ping-ponged height texture.
- Resolution `Min(512, …)`, noting it "Doesn't need to be high-res".
- Memory **2 MB** on consoles and **2–4 MB** on PC (FP16 vs FP32).
- Cost "< 1.0ms GPU on PS3/360".
- Refill by subtracting a small value per frame; "Only render 2 surfaces/frame".
- The DX11 version adds a minimum height field and projected displacement.

### 7.5 Wind shared by particles, foliage and cloth

**Ghost of Tsushima** [P/S, gamedeveloper.com, fetched]:
- "The main wind vector has a constant direction, but we varied the magnitude … using
  time-varying Perlin noise".
- **Vorticles** are "invisible wind-generating particles" with a position, orientation, radius and
  wind direction, atomically appended into "a single small array".
- They are evaluated by "just running down the whole list and adding up all of the
  contributions", for "hundreds of vorticles".
- Particle emitters "opt-in … for relatively few places".
- Consumers: particles, grass and foliage, and interactive effects.

**God of War** [P, abstract only]: a "full 3D GPU fluid simulation". The numbers were not
retrievable.

**Wicked** [P]: global `windDirection`, `windRandomness`, `windWaveSize` and `windSpeed`, plus a
per-spring `wind_affection`.

### 7.6 Lightning

No primary source on shipped lightning was retrieved; the web-search budget was exhausted before
this topic was reached. The composition that the in-tree pieces already allow is recorded in design
§2.2 and left as an owner scope question:
- a short-lived directional or point flash through the light table, as the explosion flash does;
- a bolt drawn as a ribbon (particle plan P4);
- a sky-brightness pulse, owned by the post/sky threads.

---

## 8. Numbers ledger (every number the design consumes, with provenance)

| Quantity | Value | Tag | Source |
|---|---|---|---|
| Particle sim budget | `9 µs + 1.10·(128 B·N)/121 GB/s` | [M] | plan §Goal |
| Draw per tiny billboard | 1.55 ns at 102 400 | [M, SUSPECT] | plan gate #17 |
| Spawn through `Commands` | 20k spawns ≈ 0.6–2 ms ⇒ 30–100 ns each | [M-derived] | plan D1 |
| Effective blend fill, region off chip | 15.1–42 Gpx/s | [E] | §2 |
| Blend fill ceiling, region on chip | 66.6–81.7 Gpx/s (the ROP peak) | [S/E] | §2; Kanter 2016 for the mechanism |
| FP32 at 1.387–1.703 GHz | 10.65–13.08 TFLOPS (the source lists 6.9–10.9) | [E] | §2 |
| Atlas mip levels worth storing | down to one texel per cell | [P] | NVIDIA WP-01387-001 (2004) |
| Peak texture rate | 166.4–204.4 Gtex/s | [S] | Wikipedia |
| L2 / DRAM bandwidth | 3 MB / 288–336 GB/s | [S] | Wikipedia |
| CSM depth pass, 160k-vertex model, all cascades | 0.067 ms | [M] | [`../diagnostics/W2208.md`](../diagnostics/W2208.md) (`ZONE_GBUF_CSM_DEPTH`) |
| Off-screen particle speedups | 2.04× (73 instructions, mixed 4×4); 1.14× (9 instructions, mixed 2×2) | [P] | GPU Gems 3 ch. 23, 8800 GTX |
| Remember Me rain layers | 1.69 / 1.72 ms at 720p, flat in intensity | [P] | Lagarde 2a |
| Remember Me occlusion map | 256², 20 m; 0.32 / 0.20 ms | [P] | Lagarde 2a |
| Ripples 256² | 0.14 / 0.15 ms | [P] | Lagarde 2b |
| Whole rain system | ≈ 2.8 ms PS3, 720p | [P] | Lagarde 2b |
| GPU rain drops, 8800 GTX | 200k / 1M / 5M → 545 / 126 / 26 fps (raster) | [P] | Tariq 2007 |
| Snow deformation | < 1 ms, 2–4 MB, 2 surfaces/frame | [P] | Brisebois GDC 2014 |
| UE volumetric fog | 1 ms PS4 High; 3 ms GTX 970 Epic ("eight times more voxels") | [P] | UE docs |
| MBOIT | 3.5 ms vs 1.37 ms sorted, 1080p, RTX 3080 Laptop | [P via tree] | `TRANSPARENCY-DESIGN-SPACE.md` R7 |
| Opaque colour mip chain | 11.1 MB at 1080p; ≈ 30 MB/frame traffic | [E via tree] | `TRANSPARENCY-DESIGN-SPACE.md` R4 |
| Motion-vector flipbook gain | > 10× frame extension; 32 frames → > 30× | [P] / [S] | Lozar / 80.lv |

---

## 9. Pitfalls ledger (new beyond `PARTICLES-RESEARCH.md`)

1. **GPU-emitter events** are unsupported in Niagara. Here they must be GPU-side and, by D3,
   next-frame (§5.2).
2. **A CPU readback for splash placement** (Remember Me) violates the plan's "Readback: none,
   ever". Wicked's in-sim reposition and a vertex-stage map fetch both avoid it.
3. **Height maps are blind to overhangs** (Godot). A follow-camera map re-renders on every camera
   move unless it re-centres in steps.
4. **Depth-buffer collision** loses off-screen particles, needs previous-frame reprojection, and
   here needs two depth decodes (Deferred euclidean vs reverse-Z).
5. **Fourier Opacity Map self-shadowing rings** on opaque-ish translucency.
6. **Fog temporal reprojection smears flashes** (UE); the flash light must opt out of fog
   scattering.
7. **Six-way cards are still sprites**: flat under shadow, no neighbour occlusion, no reflections
   (Unity).
8. **Motion-vector sheets must be uncompressed** (Lozar). With no BC formats in this RHI, colour
   sheets are also uncompressed, so flipbook VRAM is about 3× a BC7 engine's (design §7).
9. **Distortion** bleeds foreground, clamps at screen edges and loses layering (Froyok); only
   opaque surfaces refract correctly (UE).
10. **Alpha particles under TAA ghost** without motion vectors or a reactive or coverage mask
    (FSR2). Sorted particles cannot carry motion vectors here (plan R10).
11. **Low-resolution particles** need a max-depth downsample and an edge re-render (GPU Gems 3),
    and gain little when the fragment shader is cheap (§4.6).
12. **Translational camera shake in 3D** is "super lame" (Eiserloh).
13. **Snow deformation** must be budgeted per frame (Batman: 2 surfaces/frame).
14. **Wind-driven particles over terrain** need height samples ahead of them (Ghost of Tsushima,
    carried).
15. **A time phase in `f32`** loses precision over a long session: at t = 24 h an `f32` second has
    a 7.8 ms ULP, which is 7 cm at 9 m/s [E, IEEE arithmetic]. Stateless effects must receive a
    host-wrapped phase.
16. **In-tree: a weather term added as a runtime branch in every lit producer** is the F24 "dark
    tax" class. The VB-SV0 detour cost +75 % while OFF (plan F24).
17. **In-tree: the pool is shared.** `MAX_EMITTERS = 256` and `CAP = 262 144` are shared by
    weather and every simultaneous explosion if both are pool clients. Clamps are counted
    (`clamped_spawns`) and not surfaced. A1 also pushes a row for every *enabled* emitter, idle or
    not (§1.1), so 256 idle emitters are enough to starve a 257th that wants to spawn.
18. **Flipbook atlases bleed at coarse mips and under warps** (NVIDIA 2004). A half-texel inset at
    mip 0 protects mip 0 only. A motion-vector warp that is not clamped to its cell samples the
    neighbouring frame.
19. **Near-camera overdraw is the particle frame-time cliff.** A count budget cannot see it: 48
    full-screen layers cost 48 × the per-screen time. Unity bounds it with a viewport-fraction size
    clamp and a camera fade (§4.1).
20. **A top-down occlusion map updated only on re-centre freezes whatever moved since.** Remember
    Me's map includes characters (§7.2), and Far Cry 6 handles dynamic objects separately with ray
    casts. A map without a dynamic-caster policy leaves dry ghosts where a vehicle stood.
21. **Wetness applied twice.** A per-material driver that darkens the material row, plus a
    per-pixel wet-surface term that darkens again, doubles Lagarde's darkening. Exactly one of them
    may apply it (design §2.4).
22. **A vertex-shader readback needs `vertexPipelineStoresAndAtomics`**, which this device does not
    enable (§1.3). A stateless effect computed in the VS has to be observed some other way.

---

## 10. Sources

*Fetched* = opened during this pass. *Carried* = the upstream report's retrieval of the same day,
not re-opened here.

1. Lagarde, "Water drop 2a – Dynamic rain and its effects" — https://seblagarde.wordpress.com/2012/12/27/water-drop-2a-dynamic-rain-and-its-effects/ (fetched)
2. Lagarde, "Water drop 2b" — https://seblagarde.wordpress.com/2013/01/03/water-drop-2b-dynamic-rain-and-its-effects/ (fetched)
3. Lagarde, "Water drop 3b – Physically based wet surfaces" — https://seblagarde.wordpress.com/2013/04/14/water-drop-3b-physically-based-wet-surfaces/ (fetched)
4. Tariq, "Rain", NVIDIA SDK 10 whitepaper, 2007 — https://developer.download.nvidia.com/SDK/10/direct3d/Source/rain/doc/RainSDKWhitePaper.pdf (fetched; text extracted from the PDF)
5. Wicked Engine `emittedparticle_simulateCS.hlsl` — https://raw.githubusercontent.com/turanszkij/WickedEngine/master/WickedEngine/shaders/emittedparticle_simulateCS.hlsl (fetched)
6. Wicked Engine `wiScene_Components.h` (`WeatherComponent`) — https://raw.githubusercontent.com/turanszkij/WickedEngine/master/WickedEngine/wiScene_Components.h (fetched)
7. Cantlay, "High-Speed, Off-Screen Particles", GPU Gems 3 ch. 23 — https://developer.nvidia.com/gpugems/gpugems3/part-iv-image-effects/chapter-23-high-speed-screen-particles (fetched)
8. Epic, "Volumetric Fog" — https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine (fetched)
9. Epic, "Niagara Fluids reference" — https://dev.epicgames.com/documentation/unreal-engine/niagara-fluids-reference-in-unreal-engine (fetched)
10. Unity, "Realistic smoke with 6-way lighting in VFX Graph" — https://unity.com/blog/engine-platform/realistic-smoke-with-6-way-lighting-in-vfx-graph (fetched)
11. Ubisoft, "Simulating tropical weather in Far Cry 6" — https://www.ubisoft.com/en-us/company/how-we-make-games/technology/articles/simulating-tropical-weather-in-far-cry-6 (fetched)
12. "Using vorticles to simulate wind in Ghost of Tsushima" — https://www.gamedeveloper.com/design/using-vorticles-to-simulate-wind-in-i-ghost-of-tsushima-i- (fetched)
13. Barré-Brisebois, "Deformable Snow Rendering in Batman: Arkham Origins", GDC 2014 transcript — https://archive.org/stream/GDC2014Brisebois/GDC2014-Brisebois_djvu.txt (fetched)
14. Eiserloh, "Juicing Your Cameras With Math", GDC 2016 transcript — https://archive.org/stream/GDC2016Eiserloh/GDC2016-Eiserloh_djvu.txt (fetched)
15. Lozar, "Frame blending with motion vectors" — https://www.klemenlozar.com/frame-blending-with-motion-vectors/ (fetched)
16. Courrèges, "DOOM (2016) – Graphics Study" — https://www.adriancourreges.com/blog/2016/09/09/doom-2016-graphics-study/ (fetched)
17. AMD FidelityFX FSR2 README — https://github.com/GPUOpen-Effects/FidelityFX-FSR2/blob/master/README.md (fetched)
18. Godot, `GPUParticlesCollisionHeightField3D` — https://docs.godotengine.org/en/stable/classes/class_gpuparticlescollisionheightfield3d.html (fetched)
19. Wikipedia, "GeForce 30 series" (mobile table, RTX 3060 Laptop) — https://en.wikipedia.org/wiki/GeForce_30_series (fetched; secondary)
20. GDC Vault, "Wind Simulation in God of War" (abstract only) — https://gdcvault.com/play/1026404/Wind-Simulation-in-God-of (fetched)
21. Epic, "Key concepts in Niagara" — https://dev.epicgames.com/documentation/en-us/unreal-engine/key-concepts-in-niagara-effects-for-unreal-engine (carried)
22. Epic, "Events and event handlers in Niagara" — https://dev.epicgames.com/documentation/unreal-engine/events-and-event-handlers-in-niagara-effects-for-unreal-engine (carried)
23. Epic, "Scalability and best practices for Niagara" — https://dev.epicgames.com/documentation/en-us/unreal-engine/scalability-and-best-practices-for-niagara (carried)
24. Epic, "Lit translucency" — https://dev.epicgames.com/documentation/en-us/unreal-engine/lit-translucency-in-unreal-engine (carried)
25. Epic, "GPU raytracing collisions in Niagara" — https://dev.epicgames.com/documentation/en-us/unreal-engine/gpu-raytracing-collisions-in-niagara-for-unreal-engine (carried)
26. Epic, "Using refraction" — https://dev.epicgames.com/documentation/en-us/unreal-engine/using-refraction-in-unreal-engine (carried)
27. Unity VFX Graph, output shared settings — https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/Context-OutputSharedSettings.html (carried)
28. Unity VFX Graph, Collide with Depth Buffer — https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/Block-CollideWithDepthBuffer.html (carried)
29. Unity VFX Graph, Trigger Event On Die — https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@15.0/manual/Block-TriggerEventOnDie.html (carried)
30. 80.lv, "VFX of Star Citizen: working on explosions" — https://80.lv/articles/vfx-of-star-citizen-working-on-explosions (carried; secondary)
31. 80.lv, "SIGGRAPH: the Destiny particle architecture" — https://80.lv/articles/siggraph-the-destiny-particle-architecture (carried; secondary)
32. Kovalovs, "GPU Driven Effects of The Last of Us Part II", SIGGRAPH 2020 (abstract) — https://history.siggraph.org/learning/gpu-driven-effects-of-the-last-of-us-part-two-by-kovalovs/ (carried)
33. Crane, Llamas, Tariq, "Real-Time Simulation and Rendering of 3D Fluids", GPU Gems 3 ch. 30 — https://developer.nvidia.com/gpugems/gpugems3/part-v-physics-simulation/chapter-30-real-time-simulation-and-rendering-3d-fluids (carried)
34. Lorach, "Soft Particles", NVIDIA 2007 — https://developer.download.nvidia.com/whitepapers/2007/SDK10/SoftParticles_hi.pdf (carried)
35. Hillaire, "Physically-based & unified volumetric rendering in Frostbite", SIGGRAPH 2015 (abstract) — https://advances.realtimerendering.com/s2015/index.html (carried)
36. Sousa & Geffroy, "The Devil is in the Details: idTech 666", SIGGRAPH 2016 — https://advances.realtimerendering.com/s2016/Siggraph2016_idTech6.pdf (carried)
37. Froyok, "Refraction" — https://www.froyok.fr/blog/2024-12-refraction/ (carried)
38. McGuire & Bavoil, "Weighted Blended Order-Independent Transparency", JCGT 2013 — https://jcgt.org/published/0002/02/09/ (carried)
39. Münstermann et al., "Moment-Based Order-Independent Transparency", I3D 2018 — https://momentsingraphics.de/I3D2018.html (carried)
40. Tatarchuk & Isidoro, "Artist-Directable Real-Time Rain Rendering in City Environments", EG NPH 2006 — https://diglib.eg.org/items/b4d11d22-3e3d-4ac5-b33c-6b091143add1 (carried)
41. Rockenbeck, "Blowing from the West: Simulating Wind in Ghost of Tsushima", GDC 2021 — https://gdcvault.com/play/1027124/Blowing-from-the-West-Simulating (carried)
42. JangaFX EmberGen — https://jangafx.com/software/embergen/ (carried; vendor)
43. PopcornFX — https://www.popcornfx.com/ (carried; vendor)
44. Khronos, `vkCmdDrawIndirectCount` — https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdDrawIndirectCount.html (carried)
45. Khronos, `VK_KHR_push_descriptor` — https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_push_descriptor.html (carried)
46. Kanter, "Tile-based Rasterization in Nvidia GPUs", RealWorldTech, 2016-08-01 — https://www.realworldtech.com/tile-based-rasterization-nvidia-gpus/ (fetched in pass 2; secondary, a measured analysis rather than vendor documentation)
47. NVIDIA, "Improve Batching Using Texture Atlases", SDK whitepaper WP-01387-001, July 2004 — https://download.nvidia.com/developer/NVTextureSuite/Atlas_Tools/Texture_Atlas_Whitepaper.pdf (fetched in pass 2; text extracted from the PDF)
48. Unity Manual, "Particle System Renderer module" — https://docs.unity3d.com/Manual/PartSysRendererModule.html (fetched in pass 2)
49. Unity Manual, "Standard Particle Shaders" — https://docs.unity3d.com/Manual/shader-StandardParticleShaders.html (fetched in pass 2)
50. Khronos, Vulkan specification, "Precision and Operation of SPIR-V Instructions" — https://docs.vulkan.org/spec/latest/appendices/spirvenv.html (**not retrieved**: the page is too large for the fetch tool; nothing in either document rests on it)
51. Epic, "Heterogeneous Volumes in Unreal Engine" — https://dev.epicgames.com/documentation/en-us/unreal-engine/heterogeneous-volumes-in-unreal-engine (fetched in pass 2)
52. Microsoft, "Variable Rate Shading" (Direct3D specification) — https://microsoft.github.io/DirectX-Specs/d3d/VariableRateShading.html (fetched in pass 2)
