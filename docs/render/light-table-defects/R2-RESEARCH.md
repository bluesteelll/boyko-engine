# Research: Defect R2, where the SDF marcher gets its sun direction

All of this comes from reading the code. I did not build, run, or measure anything. Tree: `D:/wt/vkval`.

## Brief summary (TL;DR)
- **Only one reader uses the boot constant.** `DEFAULT_SUN_DIR` feeds the Deferred marcher (`sdf_gbuffer_composite.hlsl:1806`, `:1868`), which writes its result to `gMaterial.r`. Ten other shaders pick the primary directional straight from the light table.
- **The push field is written but never read on Forward, Forward+ and VB.** `SdfForwardMarchPush.light_dir` is filled in, but `sdf_forward_march.comp.hlsl` never reads `pc.light_dir`; it uses `L.dir` from the table.
- **The "bound but inert" comment at `gpu_scene/mod.rs:603-609` is out of date, for two reasons.**
  - The runner does upload ECS SDF edits (`runner.rs:1502-1529`).
  - Even with an empty edit list, `sdf_soft_shadow` returns 0 whenever `dot(n,L) <= 0` (`sdf_gbuffer_composite.hlsl:499-501`). So on Deferred × Both, every mesh pixel gets a hard terminator relative to `DEFAULT_SUN_DIR` in `gMaterial.r`.
- **Pins:**
  - `grand_showcase_2mat` (sun `[-0.40,0.78,0.48]`, about 7.8° from the constant) is predicted to move.
  - `taa_armed`, `taa_armed_basis`, `taa_rcas`, `particle_sdf_collide` and `deferred_sdf_casters` use the same direction as the constant. Whether their bytes change is unknown and has to be measured.
  - Every Forward, Forward+ and VB pin is unaffected by construction.
- **Sign convention:** both sides use "direction TO the light", so no sign flip is needed. The two sides differ only in normalisation and in the float path the value takes.
- **In other engines, no sun direction is a boot constant.** Bevy, Godot SDFGI and UE all take the direction from the light's transform every frame, in the same record that drives direct lighting.

## In-tree findings

### Readers of the marcher's `light_dir`
| # | Site | What it does | Live? |
|---|---|---|---|
| 1 | `crates/boyko_app/src/gpu_scene/mod.rs:610` | `const DEFAULT_SUN_DIR = [-0.45,0.82,0.36]`; the doc at `:603-609` says "to the light" and "inert" | the only value ever written |
| 2 | same file, `:1120` (field), `:4632` (seeded at boot) | `GpuSceneBundles::light_dir`; nothing reassigns it | — |
| 3 | same file, `:6706` | copies it into `GBufferScene::light_dir` every frame | — |
| 4 | `crates/boyko_rhi_vulkan/src/present/scene_types.rs:2371-2380` | Field doc: "MUST equal the resolve's PRIMARY directional … (the first directional in the light table …)". **gpu_scene breaks this written contract.** | — |
| 5 | `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs:1497-1505` → `FineMarcherPush.light_dir` at offset 16 (`compute.rs:3494-3509`, const-assert `:3569`) | Deferred marcher push, rebuilt every frame (a change needs no re-record) | **Yes**, on Deferred × {Both, Sdf} |
| 6 | `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl:401`, `:1806`, `:1868` | `normalize(pc.light_dir)` feeds the SDF-hit shadow and the mesh-pixel cast shadow, then goes to `gMaterial.r`. Both lines are hand-written; the only generated span is `:502-525` (the `sdf_soft_shadow` loop). There is one `.spv` (`sdf_gbuffer_composite.comp.spv`) and no manifest row. | **Yes** |
| 7 | `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:784` (`shadow = gMaterial.r`) | Used as the primary's visibility at `:982-984`, then min-combined with CSM (`:1095`) or, under `#if HWRT`, with the TLAS rays (`:1001-1092`). Extra directionals default to `vis = shadow` (`:982`). Punctual lights use `vis = shadow` when the punctual shadow mode is OFF (`:1371`). | **Yes**, in both the software and HWRT resolve variants |
| 8 | `passes/forward.rs:936-952`, `passes/vb.rs:4739-4761` → `SdfForwardMarchPush.light_dir` at offset 16 (`compute.rs:3750-3821`) | Declared at `sdf_forward_march.comp.hlsl:270` and never read. The shader takes the first directional's `L.dir` (`:1053-1065`). | **No** (dead). Removing it would change the push layout for the four `.spv` variants (manifest `:264-267`). |
| 9 | `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs:8613`, `:9898` | `marcher_light_dir(&cfg.light_elems)` (`:4071-4083`) returns the first directional in the harness table. Lines `:2456` and `:3571` pass `DEFAULT_LIGHT_DIR [0,0,1]`. | Test harness only; it already honours the contract |

- **Shaders that read the primary directional from the light table in-shader** (Grep for `normalize(L.dir)` / `primary_dir_seen = true`): `deferred_pbr`, `forward_opaque.fs`, `sdf_forward_march`, `sdf_mesh_shadow`, `sdf_probe_update`, `vb_geo`, `vb_resolve`, `vb_shade`, `vb_shade_split`, `vb_shadow_vis`.
- **The marcher's descriptor set has no light-table binding.** Its bindings are 0-15 (`sdf_gbuffer_composite.hlsl:102-324`).
- **No test checks that the marcher's `light_dir` equals the primary.** The whole tree has two occurrences of `DEFAULT_SUN_DIR`, both in `gpu_scene`.

### How the primary directional light is chosen
1. **Authoring.** `DirectionalLight.direction` is "world direction TO the light", normalised in `new()` (`boyko_render/src/light.rs:286-293`, `:1183-1188`). `#[require(Transform, GlobalTransform)]` guarantees every sun has a transform.
2. **`light_reconcile`** (`light_reconcile.rs:105-117`). On `Changed<GlobalTransform>` it overwrites `direction` with `normalize(matrix3·(0,0,-1))`, bit-gated. It is registered `.before(collect_lights)` (`light_plugin.rs:119-120`). So the table direction is derived from the transform, not from the constructor argument.
3. **`collect_lights`** (`light_system.rs:582-661`). Rebuilds when a light changed or `LightTableDirty` is set. It keeps only `IsEnabled<LightEnabled>` directionals and folds them first (`fold_light_table_slotted`, `:273-296`), via `GpuLight::from_directional` (normalise again; `light.rs:1301-1304`).
4. **Deferred PBR resolve.** The primary is the first `LIGHT_KIND_DIRECTIONAL` in the l0a loop, via the `primary_dir_seen` latch (`deferred_pbr.hlsl:969-984`). `forward_opaque`, the `vb_*` shaders, `sdf_forward_march` and `sdf_mesh_shadow` (`:150-158`) use the same latch.
5. **CSM fit** (`csm_config.rs:1325-1366`). `suns.iter().next()` over `Query<&DirectionalLight>`, with **no `LightEnabled` filter**. With no sun, or no mesh-shadow producer, it publishes DISABLED. It is ordered after `light_reconcile` only by add-order ("loose one-frame stagger … self-correcting", `csm_plugin.rs:25-46`).
6. **Availability each frame.** It is available.
   - The ECS frame (runner step 2) runs at `runner.rs:1307-1317`, which is before `LightTableStaging` is read (`:1872`), before `ResolvedCsm` is read (`:1898`), and before `scene()` is called (`:2610`).
   - At that point the reconciled `DirectionalLight` query and `LightTableStaging::bytes()` are current.
   - The staged bytes are already parsed host-side elsewhere: `shadow_poison::staged_shadow_words` (`shadow_poison.rs:158`) and `dump_diagnostics` (`runner.rs:3557-3574`).
   - There is **no resolved "primary sun" resource**, and `ResolvedCsm` has no direction field (`csm_config.rs:427-444`).
   - The single device light table matches the World within a frame: uploads are gated per slot on the table generation, and the copy is recorded in the same frame.

### Zero or several directional lights
- **Zero.** The table has no directional, the resolve has no sun term, and CSM is DISABLED. The Deferred marcher still marches toward `DEFAULT_SUN_DIR`. `gMaterial.r` then masks point and spot lights (`deferred_pbr.hlsl:1371`, when the punctual shadow mode is OFF and the light is not a multi-light flagged caster), including a hard 0 on faces turned away from the phantom sun. This is inferred from the code and not measured. The Forward and VB SDF paths produce no sun shadow at all in this case.
- **Several.**
  - The primary is the first enabled directional in query order.
  - Extra directionals reuse the primary's `shadow` by default. In multi-light mode they get their own `sdf_soft_shadow_ranged` (`:975-982`, `:1098-1104`).
  - CSM can pick a different light than the resolve if the first directional is disabled. That divergence already exists and is separate from R2.

### Sun direction for every pin
The scene constant is compared with `DEFAULT_SUN_DIR`. `[-0.40,0.78,0.48]` is 7.79° away (cos 0.99077). Inside the terminator band where the two suns disagree, the table sun's NoL reaches up to sin 7.79° ≈ 0.136.

| Pin(s) | Harness · path × legs | Scene sun | Uses the constant? | Expected effect of the fix |
|---|---|---|---|---|
| `grand_showcase_2mat` | boyko_app, Deferred × Both (default), empty edit list, no CSM | `[-0.40,0.78,0.48]` (`grand_showcase_2mat.rs:43`) | **Yes** (mesh-pixel arm) | **Predicted to move** on both legs: a band along the terminator gets `vis 0 → 1`. Not measured. |
| `taa_armed`, `taa_armed_basis`, `taa_rcas` | `taa_jitter_eval`, Deferred × Both, SDF sphere, CSM | `[-0.45,0.82,0.36]` (`taa_jitter_eval.rs:171`) | Yes | Same direction. The table value is quaternion round-tripped and re-normalised, while today's push is the raw constant. **Byte movement unknown; measure.** |
| `deferred_sdf_casters` | `taa_jitter_eval`, Deferred × Sdf | same | Yes | Same as the row above; both legs PENDING, so there is no baseline |
| `particle_sdf_collide` | `particle_lab`, Deferred × Both, SDF slab | same (`particle_scene/mod.rs:196`) | Yes | Same as the TAA row; hwrt leg PENDING |
| `grand_showcase`, `deferred_sdf_only` | rhi harness | `SHOWCASE_SUN_DIR` = the same constant (`window_present_gbuffer.rs:3995`, `:5873`) | No (the harness supplies its own) | Unaffected by a gpu_scene-only fix; already consistent with its table |
| `deferred_mesh_only` | rhi harness, Deferred × Mesh | same | No (marcher not dispatched) | Unaffected |
| `forward_mesh`, `forwardplus_mesh`, `forward_both`, `sdf_forward_only`, `vb_mesh`, `vb_mesh_hzb`, `vb_occ_split`, `vb_occ_mixed{,_off,_keep,_late}`, `vb_both`, `vb_both_sdf`, `vb_both_sdf_tex`, `vb_sdf_only`, `vb_mesh_tex`, `vb_mesh_ssao`, `vb_mesh_froxel`, `vb_mesh_tex_froxel` | Forward / Forward+ / VB | `[-0.40,0.78,0.48]` | No (push field dead) | Unaffected by construction |
| `vb_taa`, `vb_taa_rcas`, `vb_both_taa`, `vb_sdf_taa`, `vb_mesh_shadows`, `particle_additive` | VB | the constant | No | Unaffected |

- **No pinned scene rotates its sun at runtime.**
- **Examples:** most use the constant. `paradigm_lab.rs` and `vb_lab.rs` use `[-0.42,0.80,0.42]`.

### Sign convention
- **The engine side is "direction TO the light" everywhere:**
  - `DirectionalLight.direction` (`light.rs:287`).
  - `light_reconcile`: the transform's −Z is aimed at the light (`light_reconcile.rs:25-42`); `look_at_rh` in the tests builds −Z = `normalize(SUN_DIR)` (`boyko_math/src/affine.rs:51-81`).
  - The resolve: `l = normalize(L.dir)`, `NoL = dot(n,l)` (`deferred_pbr.hlsl:973-974`).
  - CSM: `fwd = -sun_dir` (`csm_config.rs:505-506`, `:522`).
  - HWRT rays are cast "toward l (the world dir TO the light)" (`deferred_pbr.hlsl:1005-1006`).
- **The marcher side is also TO the light.** Docs at `gpu_scene/mod.rs:603` and `scene_types.rs:2371`; code `sdf_soft_shadow(p,n,L)` early-outs on `dot(n,L) <= 0` and marches `p + L*t` (`sdf_gbuffer_composite.hlsl:498-519`).
- **Result: no negation is needed.** The only differences are normalisation (the push is un-normalised and the shader normalises it; the table is normalised on the host and again in the shader) and the float path.
- **Contrast with other engines:** Bevy and Godot treat a light's −Z as the direction light travels, which is the opposite of boyko's pose convention. Each engine is consistent with itself.

## Approaches in state-of-the-art engines

### Bevy (`bevy_pbr`, main)
- **Approach:** `prepare_lights` runs every frame in the render world and packs `dir_to_light: light.transform.back().into()`, commented "direction is negated to be ready for N.L".
- **Data structures:** `GpuDirectionalLight { cascades, color, dir_to_light, flags, soft_shadow_size, …, sun_disk_angular_size, … }`. `MAX_DIRECTIONAL_LIGHTS = 10`.
- **Ordering:** directional lights are sorted by `(volumetric, shadow_maps_enabled, entity)`; the entity key keeps the chosen set stable when the cap is exceeded.
- **Convention:** a `DirectionalLight` "shines along the forward direction of the entity's transform".

### Godot SDFGI (renderer_rd)
- **Approach:** `GI::SDFGI::pre_process_gi()` gathers directional lights every frame. It computes `dir = -light_transform.basis.get_column(AXIS_Z)` (the direction light travels), scales y, normalises, and caps the count at `SDFGI::MAX_DYNAMIC_LIGHTS`. Lights set to sky-only are skipped.
- **Shader:** `sdfgi_direct_light.glsl` negates it (`direction = -lights.data[i].direction`) and then marches the SDF cascades.
- **Trade-off:** lights with the "Dynamic" bake mode contribute indirect light in real time but are "slower compared to Static".
- **Convention:** `DirectionalLight3D` emits along its −Z.

### Unreal Engine (DF soft shadows)
- **Approach:** a per-light feature. The directional light must be Movable, with Distance Field Shadows enabled.
- **Penumbra:** the Light Source Angle sets the penumbra size; a larger angle costs more.
- **Hand-off:** beyond the Cascaded Shadow Map distance, distance fields take over.
- **Method:** "By tracking the closest distance a ray passed by an occluding object, an approximate cone intersection can be computed with no extra cost."
- **Not read:** the engine source (it needs an Epic account). The 4.27 docs returned 403, and I could not parse the PDF of Wright's SIGGRAPH 2015 talk.

### Inigo Quilez / Aaltonen (reference algorithm)
- **Approach:** `softshadow(ro, rd, mint, maxt, k)`, where `rd` points toward the light and is passed in on each call.
- **Loop:** `res = min(res, k*h/t)`; `k` is roughly the inverse of the light's size.
- **Improvement:** Aaltonen's GDC 2018 version triangulates between consecutive samples to reduce banding.

### Filament / Frostbite
- **Filament:** `vec3 l = normalize(-lightDirection);` — the host stores the direction light travels.
- **Frostbite** (Lagarde & de Rousiers 2014): the sun is a single direction for diffuse and an oriented disk for specular; the sun's angular diameter is about 0.5°.
- **No reliable information found** on how Frostbite or Guerrilla (Decima) feed the sun into SDF shadows.

## Comparative table
| Aspect | boyko (today) | Bevy | Godot SDFGI | UE DF shadows |
|---|---|---|---|---|
| Source of the SDF or shadow sun | Boot constant (Deferred marcher only) | Light transform, every frame | Light transform, every frame | The light (Movable), every frame |
| Direction stored on the host | TO the light | TO the light (`dir_to_light`) | Travel direction (−Z) | not verified |
| Negated in the shader | no | no | yes | not verified |
| Light cap | `MAX_LIGHTS` table | 10 directionals | `MAX_DYNAMIC_LIGHTS` | per light |
| Penumbra control | `SHADOW_K = 8` constant | `soft_shadow_size` | — | Light Source Angle |

## Key algorithms and techniques
- **The shadow direction and the direct-lighting direction come from the same record.** Every surveyed engine derives both from one light record each frame. In boyko, ten shaders already do this in-shader through the `primary_dir_seen` latch.
- **Cone or closest-distance soft shadow.** Visibility is `min(k·d/t)` along the ray toward the light, where `k` stands in for light size or angle (IQ, UE).

## Pitfalls and mistakes
- **A convention mix-up flips shadows.** One side stores "TO the light", the other "travel direction". Godot and Filament negate in the shader; Bevy negates on the host.
- **A comment claiming a lane is inert went stale.** The mesh-pixel arm is live even with an empty edit list, because of the `dot(n,L) <= 0 → 0` early-out.
- **Pins with an equal-direction sun can hide a direction bug.** Most examples and the TAA pins use exactly `DEFAULT_SUN_DIR`.

## Relevant academic works
- "RTSDF: Generating Signed Distance Fields in Real Time for Soft Shadow Rendering", Tan, Chua, Koh, Bhojan, 2022. SDFs are regenerated every frame for soft shadows under dynamic lights. https://arxiv.org/abs/2210.04449 (abstract only).

## Applicability to boyko-engine (facts only)
- **A "first directional in the table" helper already exists in tests:** `marcher_light_dir`, `window_present_gbuffer.rs:4071-4083`.
- **A host-side change touches no HLSL or `.spv`.** Moving the source into the shader would mean hand-editing `sdf_gbuffer_composite.hlsl` outside its generated span, adding a light-table binding the marcher's set does not have today, and re-running the DXC sync gate on one `.spv`.
- **`GBufferScene { light_dir: … }` literals exist at five sites:** `gpu_scene/mod.rs:6706` and `window_present_gbuffer.rs:2456`, `:3571`, `:8613`, `:9898`.

## Open questions for the architect
- Should the marcher's source be the World `DirectionalLight` query, the staged table bytes, or the table read in-shader? The CSM query ignores `LightEnabled`, so these can disagree.
- With zero directionals, what should the marcher write to `gMaterial.r`, given that it currently masks point and spot lights?
- What to do with the dead `SdfForwardMarchPush.light_dir` field.
- Where the ULP drift on the equal-direction pins would be measured: the software and hwrt legs of the TAA pins, and `particle_sdf_collide`.

## Sources
[1] https://iquilezles.org/articles/rmshadows/ — the reference soft-shadow algorithm; `rd` points toward the light
[2] https://dev.epicgames.com/documentation/en-us/unreal-engine/distance-field-soft-shadows-in-unreal-engine — UE: Movable light, Light Source Angle, hand-off beyond CSM distance
[3] https://dev.epicgames.com/documentation/en-us/unreal-engine/mesh-distance-fields-in-unreal-engine — UE cone approximation from closest distance
[4] https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_pbr/src/render/light.rs — Bevy `dir_to_light = transform.back()`, sort key, cap
[5] https://docs.rs/bevy/latest/bevy/prelude/struct.DirectionalLight.html — Bevy convention (shines along forward)
[6] https://raw.githubusercontent.com/godotengine/godot/master/servers/rendering/renderer_rd/environment/gi.cpp — Godot SDFGI per-frame gather, `-basis.z`, cap
[7] https://raw.githubusercontent.com/godotengine/godot/master/servers/rendering/renderer_rd/shaders/environment/sdfgi_direct_light.glsl — Godot shader-side negation
[8] https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html — Dynamic vs Static bake mode cost
[9] https://docs.godotengine.org/en/stable/classes/class_directionallight3d.html — Godot light emits along −Z
[10] https://google.github.io/filament/Filament.md.html — `l = normalize(-lightDirection)`
[11] https://seblagarde.wordpress.com/2015/07/14/siggraph-2014-moving-frostbite-to-physically-based-rendering/ — Frostbite sun treatment
[12] https://arxiv.org/abs/2210.04449 — RTSDF