# Research: R1, where an un-slotted punctual light samples punctual atlas layer 0

## Brief summary (TL;DR)
- **Six shader sites, not three, and all six are hand-written.** Every one gates the atlas sample on `slot != SLOT_NONE` alone. None of them sits inside a `GENERATED` sentinel, and `boyko_shaderdsl` has no atlas-slot code. Up to 20 committed `.spv` modules contain one of these sites.
- **Bit 16 means two different things.** In the shaders, bit 16 is `LIGHT_FLAG_CASTS_SHADOW` = "casts an **SDF** shadow". It is read only by the multi-light SDF march in `deferred_pbr.hlsl`, and never by any atlas gate. On the host, the same bit is `CASTS_SHADOW_BIT` = "has a real atlas slot". Production sets it only through `pack_atlas_slot`. The golden `GoldenLight::with_sdf_shadow()` sets it **without** a slot.
- **The brief's claim that `vb_mesh_froxel` and `vb_mesh_tex_froxel` are exposed does not hold up on static reading.** Neither scene spawns a `ShadowCaster`, so `CsmCasterScratch::batch_count() == 0`. That makes `depth_pass_armed` false, so header bit 3 never arms. By this reading, **no pinned scene is exposed**. One un-pinned scene is: `examples/vb_lab.rs`. This needs an on-device check (see the last open question).
- **The byte-identity guard has no working test behind the test that claims to protect it.** `slotted_fold_no_assignment_is_byte_identical_to_unslotted` compares the fold with itself, because `fold_light_table` delegates to `fold_light_table_slotted` with `SLOT_NONE`. The test that *does* pin the raw bytes, `slotted_fold_loser_packs_slot_none_not_zero`, asserts the opposite of its own name.
- **What other engines do:**
  - URP and HDRP store a `-1` sentinel in the index and test `< 0` / `>= 0`.
  - Filament, Bevy and Wicked use a separate "has a map" flag. Their index can be 0 for a non-caster, and correctness rests on the flag test.
  - Godot uses a value gate (`shadow_opacity > 0.001`).
  - boyko today carries both encodings on the host, never writes the sentinel, and its shaders test only the sentinel.

## (1) In-tree findings — D:/wt/vkval

### Consumers of the atlas-slot field (bits 17..21) and bit 16

**Shaders** (all hand-written; none eDSL-owned):

| Site | What it tests | `.spv` variants carrying it (per `docs/SHADER-VARIANT-MANIFEST.md`) |
|---|---|---|
| `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:1332-1344` | bit3 → `light_atlas_slot(L.kind) != SLOT_NONE` | `deferred_pbr`, `_wrap`, `_hwrt`, `_hwrt_denoised` (the `vis`/`vis_mv` variants return before lighting, manifest :722-724) |
| `crates/boyko_rhi_vulkan/shaders/forward_opaque.fs.hlsl:414-424` | same | `forward_opaque.fs`, `forward_opaque_froxel.fs` |
| `crates/boyko_rhi_vulkan/shaders/sdf_forward_march.comp.hlsl:1108-1118` | same | 4 variants (manifest :264-267) |
| `crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl:464-474` | same | `vb_resolve`, `vb_resolve_froxel` |
| `crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl:619-629` | same | `vb_shade`, `_tex`, `_froxel`, `_tex_froxel` |
| `crates/boyko_rhi_vulkan/shaders/vb_shade_split.comp.hlsl:586-596` | same | `vb_shade_split`, `_tex`, `_hwrt`, `_tex_hwrt` |
| `shaders/light_table.hlsli:225-234` | defines `SLOT_NONE`/`light_atlas_slot` | — |
| `shaders/light_table.hlsli:56, 309-311` | `LIGHT_FLAG_CASTS_SHADOW = 0x10000`, documented as "casts an SDF shadow" | — |
| `shaders/deferred_pbr.hlsl:1098, 1372` | the only readers of bit 16 (`light_casts_sdf_shadow`), under `multi_light` (header bit 0) | deferred_pbr lighting variants |
| `shaders/light_table.hlsli:303-305` → `cluster_cull.hlsl:323,356,475`, `sdf_mesh_shadow:157`, `vb_shadow_vis:201`, `vb_geo:411`, `sdf_probe_update:310`, `forward_sky:115,119` | `light_kind()` masks `0xFFFF`, so bits 16..21 never reach a kind comparison | — |

- `shadow_apply.hlsli:349-373` (spot) and `:396-449` (point) have no internal ownership guard. A point that decodes base 0 reads `gFaces[0].light_pos/inv_range` and layers 0..5.
- Header bit 0 (`multi_light`) has no `LightingConfig` writer (`light.rs:401-403`, `:766-779`). Only `GoldenLightHeader` sets it.

**Host:**
- **Constants:** `crates/boyko_render/src/shadow_atlas.rs:62-82`, plus compile-time asserts at `:137-138`.
- **Pack/unpack:**
  - `pack_atlas_slot` at `:471-485` sets bit 16 if and only if `slot != SLOT_NONE`.
  - `light_atlas_slot` is at `:490-492`.
  - Re-exports are in `crates/boyko_render/src/lib.rs:609-611`.
- **The only production writer** is `slot_pack` in `crates/boyko_render/src/light_system.rs:445-456`. It is called from `fold_light_table_slotted` (`:323, :339`), which is fed by `collect_lights` through `assign.base_for(id)` (`:640-651`).
- **Code that treats bit 16 as "slotted":**
  - `crates/boyko_app/src/shadow_poison.rs:139, 153-174` (`staged_shadow_words` → `slotted_rows` in the probe).
  - `crates/boyko_render/tests/mesh_shadow_arming_agreement.rs:25-28, 88-97, 191-212`.
- **Golden mirrors:**
  - `crates/boyko_rhi_vulkan/src/compute.rs:5452-5465`.
  - `crates/boyko_rhi_vulkan/src/goldens.rs:1352-1410`:
    - `GoldenLight::point`/`spot` build slot field 0.
    - `with_sdf_shadow()` sets bit 16 with no slot.
    - `with_atlas_slot()` mirrors `pack_atlas_slot`.
  - The CPU oracle reads `casts_sdf_shadow()` at `:3291, :3380, :4250, :4340`.
- **Tests:**
  - `light_system.rs:1180-1304`.
  - `shadow_atlas.rs:1512-1526, 1628-1661`.
  - `window_present_gbuffer.rs:5437-5466, 5584-5599, 5616-5631, 5814-5819, 5868-5879, 9202`.
  - `lighting_l1_host_oracle.rs:617-660`.
  - `sdf_gbuffer_hybrid.rs:4664, 4986-4991`.

**Docs that claim the shader tests bit 16 for the atlas. It does not:**
- `light_table.hlsli:223-224`
- `shadow_atlas.rs:78-81` and `:464-466`
- `light_system.rs:251-253` and `:1244-1246`
- `goldens.rs:1380-1381`

`shadow_atlas.rs:1116-1135` (the lane's new doc) is the only accurate statement of the behaviour.

### Pins with an un-slotted punctual light while bit 3 is armed
Exposure needs all of these at once: `ShadowConfig.enabled`, a mesh leg, a winning `CastsPunctualShadow` light (`mode_word == 1`), at least one `ShadowCaster` batch (`shadow_atlas.rs:310-312`), and an un-slotted point or spot.

- **`[vb_mesh_froxel]`, `[vb_mesh_tex_froxel]`:** 14 punctual lights, 2 flagged (point 6 layers + spot 1 layer = 7 ≤ 16, so both win). That leaves 12 un-slotted rows with slot field 0. But **there is no `ShadowCaster`** (`vb_mesh_froxel.rs:182-293` spawns plain `MeshBundle`s, and `bundles.rs:43-58` has no caster). `gather_shadow_casters` filters `With<ShadowCaster>` (`csm_caster.rs:210-214`), so `batch_count == 0`. As a result `sync_punctual_light_gate` (`shadow_atlas.rs:1164-1176`) never sets bit 3. **Not exposed on static reading.**
- **`[grand_showcase]`, `[deferred_sdf_only]`, `[deferred_mesh_only]`:** hand-built tables with one punctual light, slotted by `with_atlas_slot(0)` (`window_present_gbuffer.rs:5878-5879`). Not exposed.
- **Every other pin:** no point or spot lights. A grep over `crates/boyko_app/tests` finds `PointLight|SpotLight` only in 8 files, and among the pinned binaries only in the two froxel pins.
- **Exposed but not pinned:** `crates/boyko_app/examples/vb_lab.rs`:
  - `ShadowConfig` is enabled at `:109`; casters are at `:223` and `:240`.
  - A flagged spot is at `:289-306` and an **un-slotted point** at `:309-313`.
  - The only slotted source is one spot, so `active_layers == 1`. The recorder renders only `[0..active_layers)` (`gbuffer.rs:2304`, `vb.rs:1433`, `forward.rs:595`).
  - The point therefore reads `gFaces[0]` (the spot's record) and layers 1..5, which **were never rendered**.
- **Not exposed:** `boot_validation_clean.rs` and `unwritten_shadow_map_gate.rs` flag every punctual light.

**Whether the poison knob can see R1 (from the mechanism, not a run):**
- An un-slotted **spot** reads layer 0, which is rendered on every armed frame. The poison clears only at boot, so a two-poison comparison cannot see this case.
- An un-slotted **point** whose faces 1..5 land on layers at or above `active_layers` reads boot or poisoned memory, so poison can see it.

### What the byte-identity guard protects, and whether a test enforces it
- **What it protects.** For a row whose base is `SLOT_NONE`, `dir_kind.w` stays exactly the `GpuLight::from_*` tag (`1`/`2`): slot field 0, bit 16 clear. The table bytes of an unassigned world then equal the pre-Inc-1-GPU fold. Without the guard the word would be `kind | 0x003E_0000`.
- **`slotted_fold_no_assignment_is_byte_identical_to_unslotted`** (`light_system.rs:1254-1279`) **does not enforce it.** The reference side, `fold_light_table` (`:225-245`), calls `fold_light_table_slotted(.., (SLOT_NONE, p))`, so both sides run the same `slot_pack`. Mutation: deleting `if base != SLOT_NONE` leaves it green. This is static reasoning; I did not run it.
- **`slotted_fold_loser_packs_slot_none_not_zero`** (`:1221-1249`) **does enforce it.** It asserts `lose_kind == LIGHT_KIND_SPOT`, so the same mutation turns it red. But its name and doc ("must pack SLOT_NONE — never a stale base 0") state the opposite contract, and its comment wrongly says the shader falls back on the clear bit.
- **Image pins.** By static reading, no pinned pixel depends on the raw slot field of an un-slotted row. The only such rows come from the froxel pins, where bit 3 is off and every kind comparison is masked. Hand-built `GoldenLight` tables never pass through `slot_pack`.

## (2) How other engines encode "this light has no shadow map"

| Engine | Encoding | Shader test | Value a no-shadow light gets |
|---|---|---|---|
| Unity URP | Sentinel: `_AdditionalShadowParams.w` = first slice, "-1 for non-shadow-casting-lights" | `if (shadowSliceIndex < 0) return 1.0;` | Every entry is initialised to `c_DefaultShadowParams = (0,0,0,-1)` before real slices are assigned. According to the fetched page summary, lights that don't fit the atlas keep that default. |
| Unity HDRP | Sentinel: `int shadowIndex; // -1 if unused` | `(light.shadowIndex >= 0) && (light.shadowDimmer > 0)`: the sentinel plus a value gate | -1 |
| Filament | Separate flag: `channels` bit 16 `castsShadows`; index in `typeShadow` bits 8..15 | `if (light.castsShadows)` only | `ShadowInfo{castsShadows=false, index=0}`. `true` and a real index are written only for lights that received a shadow map (`ShadowMapManager.cpp`). Index 0 is valid, so the flag carries correctness. |
| Bevy 0.16 / 0.17 | Flag `SHADOWS_ENABLED = 1<<0`; the index is implicit (`light_id`; spot uses `light_id + spot_light_shadowmap_offset`) | Tests the flag, plus the mesh receiver bit | Lights are sorted so shadow casters come first. The flag is granted only if `light.shadows_enabled && index < *_shadow_maps_count`. |
| Godot 4 (master) | Value gate | `shadow_opacity > 0.001` | `shadow_opacity = 0.0` unless the light owns an atlas slot and has shadows enabled |
| Wicked Engine | Flag `ENTITY_FLAG_LIGHT_CASTING_SHADOW` plus a 12-bit matrix index | `IsCastingShadow()` | Host condition not verified |
| Unreal | `INDEX_NONE == -1` (Core). `FLightSceneProxy::ShadowMapChannel` is `int32` (for static shadowing). | No reliable public information found on the GPU local-light / VSM encoding; the source is access-restricted | — |
| Frostbite, DOOM, clustered write-ups | No reliable information found | — | — |

**Tradeoffs visible in these sources:**
- **Sentinel in the index (URP, HDRP).** "Has a map" and "which map" live in one field, so they cannot disagree. The cost is that every producer path must write the sentinel. URP guarantees this by initialising every entry first. With a signed `-1`, a zero-filled word is *not* the sentinel. With boyko's all-ones 5-bit `0x1F` sentinel, a zero or untouched field decodes to a valid slot, which is exactly the R1 mechanism.
- **Separate flag (Filament, Bevy, Wicked).** A default index of 0 is valid, so every consumer must test the flag. In Filament and Bevy the flag means "got a map", not "asked for one". Filament's host sets the flag only inside the shadow-map loop.
- **Both (HDRP's sentinel plus dimmer; boyko today).** The redundancy is safe only if both encodings come from one fact. In boyko they don't: an un-slotted row has bit 16 clear (says "no map") but slot field 0 (says "layer 0"), and the shaders read only the latter. On top of that, bit 16 is shared with the P6 R1 SDF flag in the shaders, and golden fixtures set it without a slot.
- **Cost.** Every option is one integer AND or compare per light iteration. I found no measurements.

## Open questions for the architect
- Where should the fix live: the producer (emit the sentinel), the six consumers (test bit 16), or both? Bit 16's SDF meaning is still live in golden fixtures (`with_sdf_shadow`, header bit 0), so "bit 16 ⇔ slotted" holds only for the production fold.
- Should `GoldenLight::point`/`spot` default to `GOLDEN_SLOT_NONE`? Today they default to slot 0.
- A red-first gate needs a scene that arms bit 3 with an un-slotted row. No pin does that today, so a pin cannot be the red-first run.
- The brief says the two froxel pins are exposed. To settle it on device, read `punctual_armed=` from the dump-stats line (`crates/boyko_app/src/runner.rs:3590`) or `punctual_armed_frames` / `header_word7` from the poison probe.

## Sources
[1] https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.universal/ShaderLibrary/Shadows.hlsl — URP `.w = -1` sentinel and the `< 0` early-out
[2] https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.universal/Runtime/Passes/AdditionalLightsShadowCasterPass.cs — `c_DefaultShadowParams (0,0,0,-1)` initialisation
[3] https://raw.githubusercontent.com/Unity-Technologies/Graphics/master/Packages/com.unity.render-pipelines.high-definition/Runtime/Lighting/LightEvaluation.hlsl — HDRP `shadowIndex >= 0 && shadowDimmer > 0`
[4] https://raw.githubusercontent.com/Unity-Technologies/Graphics/master/Packages/com.unity.render-pipelines.high-definition/Runtime/Lighting/LightDefinition.cs — `shadowIndex // -1 if unused`
[5] https://raw.githubusercontent.com/google/filament/main/shaders/src/surface_light_punctual.fs — `if (light.castsShadows)`, decode of bit 16 and bits 8..15
[6] https://raw.githubusercontent.com/google/filament/main/libs/filabridge/include/private/filament/UibStructs.h — `packTypeShadow` / `packChannels`
[7] https://raw.githubusercontent.com/google/filament/main/filament/src/details/Scene.h and https://raw.githubusercontent.com/google/filament/main/filament/src/ShadowMapManager.cpp — `ShadowInfo` defaults and where they are set
[8] https://raw.githubusercontent.com/bevyengine/bevy/refs/tags/v0.16.0/crates/bevy_pbr/src/render/light.rs, `.../pbr_functions.wgsl`, `.../shadows.wgsl` (v0.17.0 re-checked) — flag grant, flag test, implicit index
[9] https://raw.githubusercontent.com/godotengine/godot/master/servers/rendering/renderer_rd/shaders/scene_forward_lights_inc.glsl and `.../storage_rd/light_storage.cpp` — `shadow_opacity` gate
[10] https://raw.githubusercontent.com/turanszkij/WickedEngine/master/WickedEngine/shaders/ShaderInterop_Renderer.h — `IsCastingShadow` and index layout
[11] https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Engine/FLightSceneProxy?application_version=5.3 — `ShadowMapChannel` int32
[12] https://api.unrealengine.com/INT/API/Runtime/Core/Misc/INDEX_NONE/index.html — `INDEX_NONE = -1`