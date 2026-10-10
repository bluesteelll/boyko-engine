# Architecture: R2, making the Deferred SDF marcher use the scene's sun instead of a boot constant

## Goal
The Deferred marcher pushes a boot-time constant as its sun direction (`light_dir`). It should push the same sun the resolve uses: the first directional light in the light table. That table is `LightTableStaging`, and the device uploads its bytes every frame. Two more outcomes:
- With no directional light, the marcher bakes no sun shadow.
- A sun that rotates at runtime moves the shadows on the next frame.

GPU cost is zero, host cost is a few tens of ns per frame, and there are no allocations.

## Context: what I checked in `D:/wt/vkval`
- **Only one shader reads the constant.** `DEFAULT_SUN_DIR` (`gpu_scene/mod.rs:610`) reaches a shader only through `sdf_gbuffer_composite.hlsl:1806` and `:1868`, where it becomes `gMaterial.r`. That value is then read at `deferred_pbr.hlsl:784`, at `:982-984`, and at `:1371`, where point and spot lights use it as their visibility when punctual shadows are off.
- **Forward and VB ignore the field.** `sdf_forward_march.comp.hlsl` never reads `pc.light_dir`; it picks the sun out of the table itself at `:1053-1065`.
- **The primary sun is always row 0.** `fold_light_table_slotted` (`light_system.rs:288-296`) writes the enabled directional lights first. So if any exist, the first one is row 0. That is exactly what the shaders' `primary_dir_seen` check picks: the first row in `[0, l0a_count)` whose `kind & LIGHT_KIND_MASK(0xFFFF) == DIRECTIONAL`.
- **The staged bytes match the device table.** `LightTableStaging::bytes()` at `scene()` time holds the same bytes the resolve reads this frame. The table is re-uploaded at `runner.rs:1861-1886` until every staging slot has the current generation, and the ECS frame runs before `scene()`.
- **The mesh-pixel arm is live even with an empty SDF edit list.** `sdf_soft_shadow` returns 0 when `dot(n,L) <= 0` (`:499-501`), and nothing skips the march when the edit list is empty.

## Key decisions

### D1: Derive the sun on the host from the staged table bytes and push it
**What:**
- Add `boyko_render::light_system::primary_directional_dir(table: &[u8]) -> Option<[f32;3]>`. It is a host copy of the `primary_dir_seen` check and returns the row's `dir_kind.xyz` bits unchanged.
- The runner calls it once per frame and passes the result into `scene()`.
- `scene()` turns it into the push values `(lighting_flags, light_dir)`.

**Why:**
- The marcher gets the exact bits the resolve normalizes, so both sides compute `normalize()` on the same input.
- No shader, `.spv`, push layout, descriptor or render-graph (RDG) change.
- Zero extra GPU instructions per pixel and per light.

**Alternatives rejected:**
- **Read the table inside the marcher** (like the other ten shaders do). This needs a 17th binding in the marcher's set (today 0-15). It needs a new RDG edge from the light-copy to the marcher, which would make the marcher wait for the light-upload copy on frames where the table changed. It adds a header load, at least one 4-dword row load and a loop on both arms, re-compiles `sdf_gbuffer_composite.comp.spv`, and needs a new shader branch for "no sun". It costs more for the same result, since both would read the same bytes.
- **Query ECS in the runner** (`Query<(&DirectionalLight, IsEnabled<LightEnabled>)>`). That is a second definition of "primary", with different bits (before `normalize3`) and its own order, filter and `MAX_LIGHTS` handling that could drift from the fold.
- **Cache the primary in `LightTableStaging` during the fold.** That is a second source of truth, and it only saves reading one row per frame.
- **Latch it at boot.** That is the defect again for a rotating sun.

**Trade-off:** the marcher stays the only consumer that gets the sun from the host rather than from the table in-shader. The same-bytes derivation plus gate G3 make up for that.

### D2: With no directional light, clear `LIGHTING_FLAG_SHADOWS` and keep AO
**What:**
- `None`, or a non-finite or zero-length direction (`len² <= 1e-12`, the same threshold as `normalize3`), gives `lighting_flags = LIGHTING_FLAG_AO` and `light_dir = compute::DEFAULT_LIGHT_DIR [0,0,1]`, which is then never read.
- A valid sun gives `SHADOWS | AO` and the row's bits. This is byte-identical to today's flags on every frame that has a sun.

**Why:**
- No sun means no sun shadow. Today a sunless Deferred scene masks its point and spot lights with a shadow from a sun that does not exist (`deferred_pbr.hlsl:1371`), and blacks out faces turned away from it.
- Clearing the bit also removes up to 128 field evaluations per pixel on those frames.
- The NaN guard matters. A NaN `L` passes the `dot <= 0` test, and `NMax` turns `max(NaN, step)` into `step`, so every pixel would march all `MAX_IT` = 128 steps.

**Alternative rejected:** a fallback constant sun, which is today's bug in another form.

**Trade-off:** none that affects pins; no pinned Deferred scene is sunless.

### D3: No shader change, and the dead Forward/VB push field stays
Removing `SdfForwardMarchPush.light_dir` would change the push layout and re-compile four `.spv` variants (manifest rows `:264-267`). That is a refactor, and the owner's rule is refactor last. The field keeps receiving `scene.light_dir`, which is now the real sun, and its doc is corrected to say the shader does not read it.

## Data structures / API
```rust
// boyko_render/src/light.rs, next to :33
/// Mirrors light_table.hlsli:55 LIGHT_KIND_MASK (kind enum in bits 0..16; bit 16 = CASTS_SHADOW, 17..22 = atlas slot).
pub const LIGHT_KIND_MASK: u32 = 0xFFFF;

// boyko_render/src/light_system.rs, after write_light_table (~:197). Total function, no panic, no alloc.
/// First row i in [0, min(l0a_count, rows)) with (kind & LIGHT_KIND_MASK) == LIGHT_KIND_DIRECTIONAL
/// -> Some(dir_kind.xyz bits, verbatim). l0a_count = header counts_exposure.z bits (byte 8).
/// len < LIGHT_HEADER_BYTES -> None. Words read with from_ne_bytes (the bytes are write_pod's native POD image).
#[inline] pub fn primary_directional_dir(table: &[u8]) -> Option<[f32; 3]>;
impl LightTableStaging { #[inline] pub fn primary_directional_dir(&self) -> Option<[f32; 3]>; } // = f(self.bytes())

// boyko_app/src/gpu_scene/marcher_sun.rs (new; `mod marcher_sun;` in gpu_scene/mod.rs)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MarcherSun { pub lighting_flags: u32, pub light_dir: [f32; 3] } // 16 B, stack only
impl MarcherSun { #[inline] pub(crate) fn from_primary(p: Option<[f32; 3]>) -> Self; } // D2 rules
```

## Hot-path cost
| | Today | After |
|---|---|---|
| GPU, per pixel and per light | — | **0 change** (same `.spv`, same 32 B push; sunless frames skip the shadow march entirely) |
| Host, per frame | a 12 B field copy | 1 resource lookup (already done on frames that upload), 1 header word, ≤ `l0a_count` kind words (in practice 1), a 12 B copy, 4 compares. About 10-30 ns, 0 allocations |
| Memory | a 12 B field in `GpuSceneBundles` | field removed |

## Multithreading
Everything runs on the main thread in the runner, after the ECS frame and before recording. It reads a `Resource` through `&World`. There is no new shared state, atomic or synchronization point.

## 1. The fix: every change, file:line
**No eDSL change. No `.spv` is re-compiled. No row in `SHADER-VARIANT-MANIFEST.md` is touched.** Hand-written HLSL is untouched too.

1. `crates/boyko_render/src/light.rs:33`: add `LIGHT_KIND_MASK`. Re-export it wherever `LIGHT_KIND_DIRECTIONAL` is re-exported (`lib.rs`).
2. `crates/boyko_render/src/light_system.rs`:
   - after `:197`, add the free function `primary_directional_dir`;
   - in `:133-169`, add the method on `LightTableStaging`.
3. `crates/boyko_app/src/gpu_scene/marcher_sun.rs`: new file, `MarcherSun` plus its `#[cfg(test)]` tests.
4. `crates/boyko_app/src/gpu_scene/mod.rs`:
   - `:603-610`: delete `DEFAULT_SUN_DIR` and its doc.
   - `:1120`: delete the `light_dir` field.
   - `:4632`: delete `light_dir: DEFAULT_SUN_DIR,`.
   - `:6128-6133`: add the parameter `primary_sun: Option<[f32; 3]>` right after `light_upload`, with a parameter doc: "the staged table's primary directional, read by the runner this frame".
   - `:6698-6706`: replace with `let marcher = MarcherSun::from_primary(primary_sun);`, then `lighting_flags: marcher.lighting_flags, light_dir: marcher.light_dir,`.
   - After this, `DEFAULT_SUN_DIR` must return 0 hits across the tree.
5. `crates/boyko_app/src/runner.rs`:
   - In block 5c (`:1861-1886`), outside the `light_upload_due` branch, add `let primary_sun = world.resource::<LightTableStaging>().primary_directional_dir();`. The result is `Copy`, so no borrow is held.
   - At `:2620`, pass `primary_sun` after `light_upload`.

## 2. Pixel expectations
Legs are software and hwrt in `scripts/golden.ps1`.

| Pin | Leg | Prediction | What the viewer should see |
|---|---|---|---|
| `grand_showcase_2mat` | **both** (software == hwrt must still hold after the re-bless; no CSM, so no TLAS term) | **MOVES** | One thin crescent per sphere along the edge of the lit side. It is widest (about 5-6 px at 512²) on the camera-facing lower-right of each sphere and tapers to nothing at the silhouette. Today that crescent is cut hard to ambient light; after the fix it becomes the natural soft falloff (direct lighting up to about 0.136 × peak, since sin 7.79° ≈ 0.136). My estimate is on the order of 10³ changed pixels in total. Sky, highlights and the true dark sides stay identical. Look at an abs-diff ×8 image: it should show five crescents and nothing else. |
| `taa_armed`, `taa_armed_basis`, `taa_rcas`, `particle_sdf_collide` | both | **Likely byte-identical** | The sun is the same up to rounding: today's push is the raw unnormalized constant, the table holds the quaternion round-trip normalized twice. If anything moves: ≤1/255, a handful of pixels, mixed sign. Before re-blessing, confirm the cause with an A/B: push the old raw bits through the new path in a scratch build; the pin must go back to its old sha. |
| `deferred_sdf_casters` (both legs), `particle_sdf_collide` hwrt | — | PENDING | Bless them on the fixed tree only. |
| **Must NOT move:** `forward_mesh`, `forwardplus_mesh`, `forward_both`, `sdf_forward_only`, all `vb_*`, `particle_additive` | both | identical | This is a live test of D3. For the `grand_showcase_2mat` family, the Forward/VB push **bytes change** (constant → real sun) while the shader never reads them. Any pixel movement disproves the "dead field" claim and stops the lane. |
| **Must NOT move:** `grand_showcase`, `deferred_sdf_only`, `deferred_mesh_only`, every offscreen golden in `window_present_gbuffer` / `sdf_gbuffer_hybrid`, every `*_spv_sync` / `*_edsl_sync` | — | identical | That code path is not touched. |

Acceptance rules for re-blessing `grand_showcase_2mat`. Each one can fail:
1. Every changed pixel lies on a sphere; none are sky.
2. Every changed pixel is brighter or equal on all three channels. Visibility only goes from 0 to 1, so it can only add light; a darker pixel disproves the prediction.
3. The count of pixels that differ from the `forward_mesh` BMP (same scene, Forward already lights that band) goes down.

After that, the owner or the delegated visual oracle signs off, as the pin's own comment requires.

## 3. The gates
**G3: windowed test, red on today's tree.** New file `crates/boyko_app/tests/sdf_marcher_sun.rs`, `#![cfg(windows)]`, each test marked `#[ignore = "gpu-windowed: needs a Vulkan device + window; --test-threads=1"]`. Run it on both legs. Written first and run on today's tree before the fix.

Scene:
- Deferred × Both, AA off, 512².
- A mesh floor at y=0 with **no `ShadowCaster`**, so CSM is disarmed and the marcher is the only thing casting shadows.
- One SDF sphere, r≈0.3, about 1.5 units above the floor.
- A steep camera, so all three shadow spots below are visible and not hidden by the sphere.

- **T10 `marcher_shadow_follows_the_table_sun_after_runtime_rotation`**
  - The sun spawns at B = norm(0, .8, −.6). A system rotates its `Transform` to A = norm(.6, .8, 0) at frame 5, and the dump is taken at frame 30.
  - Probes are 5×5 mean luminance at the projected umbra centres for A, B and C (C = old constant), plus a reference R on open floor.
  - Assertions: R > LIT_MIN; exactly one of {A, B, C} < 0.5·R and the other two > 0.85·R; the dark one must be A. The failure message names the dark spot: C means the boot constant, B means the value was latched at spawn, none means vacuous (no SDF shadow).
  - **On today's tree:** C is dark, so it fails for the right reason.
- **T11 `sunless_scene_has_no_phantom_marcher_shadow`**
  - Same scene, no directional light. A point light above the scene, punctual shadows off.
  - Assertions: C′ > LIT_MIN, where C′ is the point mirrored across the light's axis, so it gets the same irradiance. C ≥ 0.9·C′.
  - **On today's tree:** the phantom sun darkens C, so it fails.
- **Named mutations:**
  - M6, revert `scene()` to the constant: T10 goes red with C dark.
  - M7, latch the primary on the first `scene()` call: T10 goes red with B dark.
  - M4, keep SHADOWS when there is no sun: the `[0,0,1]` placeholder gives `dot(n,L)=0` on the floor, the whole floor goes black, and T11's LIT_MIN guard goes red.

**G1: without a device.** New file `crates/boyko_render/tests/primary_directional.rs`, reusing `le_support::common` (`lighting_app`, `spawn_*_light`).

| Test | Checks | Mutation that turns it red |
|---|---|---|
| T1 | one sun with a non-constant pose → result equals the `from_directional` bits after reconcile (`to_bits` compare) | — |
| T2 | first sun disabled via `LightEnabled` → returns the second | — |
| T3 | empty table / sky only / point+spot only → `None` | M3: return row 0 unconditionally |
| T4 | hand-built table: directional row with bit 16 and slot bits set → `Some` | M1: drop the mask |
| T5 | hand-built: `l0a=1`, row0 = SKY, row1 looks directional → `None`; the same with `l0a=2` → row1 | M2: scan every row, not just `[0, l0a_count)` |
| T12 | 1,000-case seeded xorshift loop (no new dev-dependency): random lights and enables, run the fold, then parse → equals the first enabled directional's bits, or `None` | — |

These functions do not exist on today's tree, so red-first is shown by the named mutations above.

**G2: without a device.** `#[cfg(test)]` tests in `marcher_sun.rs`:
- T7: a valid sun gives `(SHADOWS|AO, bits unchanged)`.
- T8: `None` gives `AO` with the SHADOWS bit clear. Mutations: M4, and M4′ (flags 0, which kills AO).
- T9: NaN or zero-length gives the same as `None`. Mutation: M5, drop the guard.

**Proof it is not vacuous:**
- G3 requires exactly one dark spot among three candidates plus a lit reference. So "shadows off", "no sphere" and "everything black" all fail.
- Each G1/G2 case is paired with a mutation that turns it red.

**Invariants and debug assertions:**
- In `scene()`: `debug_assert!(flags & SHADOWS == 0 || (dir finite && len² > 1e-12))`.
- `primary_directional_dir` clamps instead of asserting, because it is a parser and must never panic.

## 4. Docs and comments that are false today
- `gpu_scene/mod.rs:603-610`: says the SDF lane is "empty, bound-but-inert". Both claims are false: the runner uploads ECS edits (`runner.rs:1502-1529`), and the mesh-pixel arm is live. Deleted together with the constant.
- `gpu_scene/mod.rs:6091`, in the `scene()` doc: "R4 wiring: SDF empty". Replace it with where the marcher's sun comes from.
- `gpu_scene/mod.rs:6277-6285`: a leftover doc for the retired `sv0_bench_lighting_flags` parameter. Delete it.
- `gpu_scene/mod.rs:6698-6705`: says "every frame pushes what every non-bench frame always pushed". That becomes false for sunless frames; rewrite it.
- `scene_types.rs:2350-2352` (`lighting_flags`): add the third value, `AO` alone, which the production host pushes when there is no primary directional.
- `scene_types.rs:2371-2380` (`light_dir`): name the production source, `LightTableStaging::primary_directional_dir`, and add "no primary ⇒ the caller clears SHADOWS; value unread".
- `gbuffer.rs:1472-1474`: "with the default directional light" becomes "with the scene's primary directional (none ⇒ shadows off)".
- `compute.rs:3761` (`SdfForwardMarchPush.light_dir`): "the shader normalizes it" is false. Replace with "UNREAD by `sdf_forward_march` (it reads the table, `:1053-1065`); kept only so the push layout stays stable."
- The HLSL comments at `sdf_forward_march.comp.hlsl:121` and `:270` become true after the fix, so they are not edited. Leaving them avoids touching a byte-gated source and shifting the docs anchors (`:297`).

## 5. What the fix must NOT do
- Must not change any shader, `.spv`, `FineMarcherPush` (32 B) or `SdfForwardMarchPush` (40 B) layout, descriptor set or RDG edge.
- Must not normalize or negate the direction on the host. Push the row's bits as they are; both sides use "direction TO the light".
- Must not change `lighting_flags` on frames that have a sun, and must not turn off AO on sunless frames.
- Must not cache or latch the primary, and must not read it from an ECS query.
- Must not touch `window_present_gbuffer.rs:4071-4083` (`marcher_light_dir`), which feeds the rhi-harness pins.
- Must not touch the multi-light path (`sdf_soft_shadow_ranged`) or the legacy `vis = shadow` rule for point and spot lights.
- Must not change the CSM sun selection in this commit (see R2b).
- Must not remove the dead Forward/VB push field (D3).

## Sibling defect R2b: separate commit, same lane, after R2
`resolve_csm_cascades` (`csm_config.rs:1337`, `:1351`) takes `suns.iter().next()` with no `LightEnabled` filter. If the first sun is disabled, CSM fits a light the resolve does not use. Its doc at `:1301-1307` claims all three consumers agree.

- **Fix:** `Query<(&DirectionalLight, IsEnabled<LightEnabled>)>` with `find_map(|(l,en)| en.then_some(l))`, and correct the doc.
- **Gate (no device, red on today's tree):** two suns, the first disabled → `ResolvedCsm` must equal `resolve_csm(.., second.direction, ..)`. Add a consistency test: three suns across archetypes with mixed enables, and CSM's chosen direction must agree with `primary_directional_dir()` of the same frame.
- **Pixels:** no pinned scene has a disabled sun, so no pin moves.
- **Why CSM cannot simply read the table:** `sync_csm_light_gate` has to run before `LightCollectSet`, so CSM cannot use the table of the same frame.

## Implementation order
1. **Test-only commit:** add G3. The GPU run on today's tree must be RED, with C dark in both T10 and T11; record that output.
2. **Fix commit:** changes 1-5 from section 1, G1 and G2, and the doc fixes. Tester checks:
   - `cargo test --workspace --all-targets --no-fail-fast`;
   - G3 on both legs;
   - `golden.ps1 -Check` on both legs, with the section 2 table as the expected result;
   - the three acceptance rules for `grand_showcase_2mat`, then owner sign-off, then re-bless in `goldens/PINS.toml` with a comment that explains R2.
3. **R2b commit.**

## Open questions
None of these is a values call.

- Re-blessing `grand_showcase_2mat` needs the visual sign-off its pin comment already requires.
- If a command-stream pin hashes push-constant bytes, it will move on Forward/VB for the `grand_showcase_2mat`-family scenes. The cause is known in advance (the push field now carries the real sun), so re-bless it on that basis.