# Architecture review: R2, making the Deferred SDF marcher use the scene's sun instead of a boot constant

## Verdict
**APPROVED WITH CHANGES.** The design is right and I have no blocking remarks: host derivation from the staged table bytes, D2's sunless rule, and D3's dead field. The gate design and the pin table need the changes W1–W3 before the red-first run.

I checked every file:line claim in the plan against `D:/wt/vkval`. These hold:
- **Only one shader reads the constant.** `pc.light_dir` is read only at `sdf_gbuffer_composite.hlsl:1806` and `:1868`.
- **The Forward/VB marcher never reads it.** The only push fields `sdf_forward_march.comp.hlsl` reads are `brick_*`, `extent_*` and `view_z_a/b`; it takes the sun from the table at `:1053-1065`.
- **The 0 is returned on the back face only.** `sdf_soft_shadow` returns 0 when `dot(n,L) <= 0` (`:498-501`).
- **An empty edit list marches the mesh arm to exactly 1.0.** `FAR = 1e9` (`sdf_field.hlsli:41`), and nothing skips the dispatch on an empty edit list.
- **Directionals come first in the table.** The fold writes them first (`light_system.rs:288-296`); the enable filter is at `:642`.
- **The upload is gated by generation** at `runner.rs:1861-1886`, and `scene()` has one caller (`runner.rs:2616`).
- **Point/spot lights take the sun's shadow when punctual shadows are off:** `vis = shadow` at `deferred_pbr.hlsl:1371`.
- **The two stale docs are real:** `mod.rs:603-610` and the retired-parameter doc at `mod.rs:6277-6285`.
- **The pin census is complete.** The boyko-app Deferred pins are `grand_showcase_2mat`, `taa_armed`, `taa_armed_basis`, `taa_rcas`, `deferred_sdf_casters` and `particle_sdf_collide`.
  - Only `grand_showcase_2mat` has a sun that differs from the constant: `grand_showcase_2mat.rs:43` is `[-0.40,0.78,0.48]`, 7.79° off.
  - `taa_jitter_eval.rs:171` and `particle_scene/mod.rs:196` use exactly the constant.
  - No pinned Deferred scene is sunless, so D2 moves no pin.

## Blocking
None.

## Important

### W1. T10 and T11 cannot share one test binary
**Where:** §3 G3, "New file `crates/boyko_app/tests/sdf_marcher_sun.rs`" containing both T10 and T11.

**Problem:** the repo's rule is one `app.run()` per windowed boyko-app binary. It is stated in:
- `room_smoke.rs:12-14`: "`EnginePlugins` composes `LightingPlugin`, whose light eviction hooks are process-global — do not co-locate a second light-archetyping test here".
- The same warning at `csm_fit_eval.rs:31-33` and `interp_smoke.rs:14-15`.
- `particle_lab.rs:46-49`: "the device singleton boots once".

**Consequence:** with `--test-threads=1`, the second scene boots in the same process and fails for a reason unrelated to R2. So step 1's required outcome, "RED with C dark in both T10 and T11", cannot be produced.

**Confidence:** CONFIRMED (the citations above).

**What is needed:** one binary per scene, or the worker re-exec pattern this lane already ships (`unwritten_shadow_map_gate.rs:21-23`, `hzb_engine_pyramid_gate.rs:734`, `boot_validation_clean.rs:693`).

### W2. G3's thresholds are not derived from the fixture's lighting
**Where:** T10's `< 0.5·R` / `> 0.85·R` / `LIT_MIN`; T11's `LIT_MIN` and `C ≥ 0.9·C′`; the M4 claim.

**Problem:** the plan does not say whether the fixture has a SkyLight. Every existing boyko-app scene has one. It also does not say whether luminance is measured on the gamma-encoded BMP. The resolve applies a manual gamma-2.2 OETF (`pbr_lighting.hlsli:184`) after ACES.

**Consequence, T10:** with the taa scene's sky (0.26, 0.32, 0.42), sun 2.8 and albedo about 0.7:
- Umbra-to-lit ratio after ACES: about 0.26 linear.
- The same ratio in the encoded BMP: about 0.55, which is above 0.5.
- T10 would then report "none dark (vacuous)" on both trees, and fail for the wrong reason before and after the fix.
- With half that ambient the ratio is about 0.36 and T10 works. The verdict therefore depends on a constant the plan does not fix.

**Consequence, T11:** consider a wiring-level M4: `scene()` keeps the `SHADOWS|AO` literal while `MarcherSun` itself is correct.
- G2's T8 cannot see this; it only tests `from_primary`.
- The `[0,0,1]` placeholder gives `dot(n,L)=0` on the floor, so the whole floor becomes ambient-only.
- C equals C′, so the 0.9 ratio check passes.
- Only `LIT_MIN` could catch it, and only if `LIT_MIN` is above the ambient-only level. The plan's "whole floor goes black" holds only when there is no SkyLight.

**Confidence:** PLAUSIBLE. The fixture's light set and the measurement space are unspecified.

**What is needed:** fix the fixture's ambient (for example, none), state the measurement space, or derive each threshold from a control run in the same build (sun off, or occluder removed). Then each named mutation's red follows from the setup rather than from a guessed constant.

### W3. `deferred_sdf_casters` is misclassified as PENDING
**Where:** §2 table, "`deferred_sdf_casters` (both legs) … PENDING".

**Problem:** `PINS.toml:1217` holds a real software digest (`ea6b1df8…`); only hwrt is PENDING (`:1218`). The pin's own comment (`:1208-1209`) disowns that value, so the lane is mid-bless. The pin's sun is the constant (`taa_jitter_eval.rs:171`). It therefore belongs with `taa_*` in the "same sun up to rounding; A/B before re-bless" class.

**Consequence:**
- On the fixed tree, `golden.ps1` reports PASS or FAIL against `ea6b1df8`, not exit 2.
- If rounding moves it, the plan sends it to "bless on the fixed tree". That skips the A/B the plan requires for exactly this class, so an R2-caused move gets frozen with nothing measured before R2.

**Confidence:** CONFIRMED (`PINS.toml:1217-1218`).

**What is needed:** order R2 after the lane's own bless of this pin on the pre-R2 tree, and list it in the `taa_*` row.

## Optional

1. **The `-Check` flag does not exist** (CONFIRMED). The implementation order says "`golden.ps1 -Check`", but the script's parameters are `Pin`, `Hwrt`, `Bless` and `ValidationOn` (`golden.ps1:63-69`). `PINS.toml:12-13` says passing `-Check` is a binding error. Use `golden.ps1 -Pin <name> [-Hwrt]`.

2. **The acceptance rules for `grand_showcase_2mat` can misfire.**
   - **Rule 2** ("brighter on all three channels") does not follow from "visibility only rises". The default tonemap is Hill ACES (`light.rs:486-488`), and `ACES_OUT` has negative off-diagonal entries (`pbr_lighting.hlsli:201`). Red-dominant added light can lower G.
   - In the dark crescent the minor channels clamp at 0 in my worked example, so the risk is small. Still, state the rule on luminance, or allow a channel to be "unchanged or clamped".
   - **Rule 3** ("the count of pixels differing from `forward_mesh` goes down") is weak. Mesh attributes pass through RGBA8 (`mod.rs:597`), so crescent pixels can still differ by 1 LSB, and a single exact match satisfies "goes down". An L1 distance over the changed-pixel mask would express the actual claim.

3. **No integration check covers more than one sun.** G3 has one directional. The host copy of the selection rule and the shaders' `primary_dir_seen` agree only by construction and by G1's parser tests. A second, dimmer directional in T10 (the dark spot must stay under the first) would pin "marcher sun == resolve primary" end to end.

4. **T12's oracle order needs defining.** If the table is built through the ECS, "the first enabled directional" must mean query iteration order; archetype order is not spawn order. If it is built from slices, T12 tests only the parser.

5. **Hand-copied `f6147f90` citations will go stale after the re-bless.** They appear in `grand_showcase_2mat.rs:191,226`, `forward_mesh.rs:16,85`, `vb_mesh.rs`, `vb_mesh_ssao.rs`, and in docs: RENDER-PARITY-PLAN, MULTI-PARADIGM-RENDER-PLAN, RENDER-AA-AND-TAILS-PLAN and PARTICLES-PLAN. Add them to §4.

6. **Unpinned owner reference dumps will change visibly.** These run Deferred × Both with a sun different from the constant: `pbr_material_showcase.rs:77` (about 31° off), `pbr_showcase.rs:30`, `textured_smoke.rs:34` and `grand_showcase_mvpm.rs:32`. None has an assert; list them so the new images are not mistaken for a regression.

7. **Scope note: D2 fixes only the sunless half.** On sunlit frames, point/spot lights (`deferred_pbr.hlsl:1371`) and extra directionals (`:982`) still take the primary sun's shadow mask. After R2 that mask moves from the constant's back faces to the real sun's in unpinned scenes with punctual lights. Record this as a sibling defect; it is not R2's to fix.

## Positive (keep these)
- **Deriving the sun on the host from the staged bytes has precedent.** The RHI harness already does it (`window_present_gbuffer.rs:4071-4083`, `marcher_light_dir`). It avoids re-compiling byte-gated `.spv`, allocates nothing, and pushes the same bits the resolve normalizes. Not normalizing or negating on the host is correct.
- **D2 brings Deferred in line with the other paths.** VB's `sdf_mesh_shadow.comp.hlsl:148-174` already leaves `vis=1.0` with no directional, and Forward reads the table. After R2, all three paths agree that no sun means no sun shadow.
- **The zero-length/NaN guard covers two real traps.** `LIGHT_KIND_DIRECTIONAL == 0`, so an all-zero row parses as a directional. `normalize3` passes a degenerate vector through unchanged (`light.rs:1180-1182`).
- **Every named mutation is paired with a gate that turns it red.** G3 is written and run red-first, and its failure messages name the cause (M6 → C dark, M7 → B dark).
- **D3 doubles as a live test.** The Forward/VB push bytes change while the shader never reads them, so any pixel movement disproves the dead-field claim.
- **The A/B protocol for the constant-sun pins is the right defence** against blessing a rounding shift without knowing its cause.
- **Splitting R2b out is correct.** The unfiltered `suns.iter().next()` is verified at `csm_config.rs:1337/1351`, and its doc at `:1304` makes the false claim R2b corrects.

## Open questions for the architect
- Does the lane re-bless `deferred_sdf_casters` software before R2 lands, or is `ea6b1df8` meant to stay?
- What is G3's exact light set (SkyLight or not), and in which space is luminance measured?