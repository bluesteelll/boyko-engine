# Render feature-parity plan — Textured PBR + SDF-object shadows for Forward / ForwardPlus

**Status:** DESIGN (Rev 2, architect ↔ critic hardened — 2 P0 + 7 P1 resolved). Not yet
implemented. Companion to [MULTI-PARADIGM-RENDER-PLAN.md](MULTI-PARADIGM-RENDER-PLAN.md)
(the render-path campaign this extends).

**Owner ask:** bring two currently **Deferred-only** features to the raster forward paths —
**(F1)** textured PBR materials from `assets/materials`, and **(F2)** shadows cast by SDF
objects onto mesh surfaces — at full visual parity, for **Forward** and **ForwardPlus**
(Deferred is the reference — it already has both). VisibilityBuffer (VB) is specified as an
**optional** follow-up (same pattern), out of the owner's stated `forward, forward+, deferred`
scope.

---

## 0. Measured diagnosis (why this plan exists)

Both features are deliberate **v1 scope cuts** in the non-Deferred paths, confirmed in code:

- **Textures** — only `deferred_pbr.hlsl` / `gbuffer_mrt.{vs,fs}.hlsl` sample the bindless texture
  table. `forward_opaque.fs.hlsl:31-37` is documented `NON-TEXTURED ONLY` (a `#ifdef TEXTURED`
  seam "for a later rung"); `vb_resolve.comp.hlsl:38,89` "no bindless texture table is bound …
  v1 non-textured". So a real material pack renders full PBR **only under Deferred**.
- **SDF-object shadows on mesh** — the Deferred marcher marches `sdf_soft_shadow(P_mesh,N,L)` per
  pixel and writes a decoupled screen-space term into `gMaterial.RG`
  (`sdf_gbuffer_composite.hlsl:1853-1884`). Forward/VB mesh pixels are shaded by
  `forward_opaque.fs` / `vb_resolve`, which have "no baked SDF shadow term"
  (`vb_resolve.comp.hlsl:287`).

**Measurement (paradigm_lab, look-down camera, Deferred vs VB, floor-shadow pixel counts):**
mesh-caster CSM shadows on the floor are **already correct under VB** (right band: Deferred 1025
vs VB 1023 shadowed px — identical); the **only** missing shadow is the SDF sphere's shadow on
the mesh floor (center band: Deferred 979 vs VB 776). So "VB has no floor shadows" is precisely
"the SDF object doesn't cast onto mesh under VB/Forward" — the F2 gap.

---

## 1. Executive summary

| Feature | Deferred reference (works, unchanged) | Target |
|---|---|---|
| **F1 Textured PBR** | `gbuffer_mrt.{vs,fs}` `-D TEXTURED` variant → bindless `Texture2D gTextures[]` (Set 1) + `PerInstanceMaterialTex` slots + OGL normal-map `n_ts.y=-n_ts.y` | `forward_opaque.{vs,fs}` (+ froxel); VB `vb_resolve` (optional) |
| **F2 SDF-shadow on mesh** | marcher writes `sdf_soft_shadow(P_mesh,N,L)` into `gMaterial.RG`; resolve reads it | a decoupled `sdf_mesh_shadow.comp` prepass whose R8 term `forward_opaque.fs`/`vb_shade` sample |

**Strategy.** New behavior lands as **opt-in** so the frozen goldens
(`58f6c6c3`/`a5ad662d`/`f6147f90`, `forward_mesh`/`forward_both`/`forwardplus_mesh`/`vb_mesh`)
hold at every rung:
- **F1** = new compiled variants (`-D TEXTURED`), exactly like `gbuffer_mrt`'s TEXTURED variant →
  base `.spv` stays `cmp`-frozen.
- **F2** = a **runtime uniform 0%-gate** (not a compile flag): `min(vis, gSdfMeshShadow.Load())`
  with the term bound to a 1×1 placeholder + a `sdf_mesh_shadow_enabled=0` uniform is a provable
  no-op (`min(vis,1.0)=vis`). Adding the binding *does* change `forward_opaque.fs.spv` bytes once →
  a **one-time, image-golden-proven re-pin** at rung SF0 (sanctioned by Decision 3: image goldens
  are authoritative, `.spv`-cmp is secondary). This buys a **2× smaller pipeline matrix** (4 forward
  FS variants, not 8) vs a `-D SDF_SHADOW` compile flag.

**Two discoveries that shrank scope (verified in code):**
1. **VB texture derivatives are already solved.** `vb_geom_fetch.hlsli:211-307` (`vb_uv_grad`) already
   emits the analytic screen-space UV gradient pair; VB texturing is
   `SampleGrad(gTextures[NonUniformResourceIndex(slot)], gTexSampler, uv, ddx, ddy)` — no new math,
   no hardware `ddx/ddy` (unavailable in compute).
2. **The SDF-vocab march is already a reusable, sync-pinned copy.** `sdf_forward_march.comp.hlsl:258-278`
   is a verbatim, `sdf_field_edsl_sync`-pinned copy of the generated `sdf_soft_shadow`, already bound
   to the live SDF field. F2's `sdf_mesh_shadow.comp` reuses that binding set + copy discipline.

---

## 2. Feature 1 — Textured PBR materials

### 2.1 Reference (Deferred, done — do NOT edit)

- `gbuffer_mrt.vs.hlsl` `-D TEXTURED`: `VsIn` adds `uv:TEXCOORD0` (loc 3) + `tangent:TANGENT` (loc 4)
  at `Vertex` offsets uv@40 / tangent@48 (`mesh.rs:83`); reads a `PerInstanceMaterialTex` SSBO (48 B,
  `mesh_draw.rs:169`); forwards `world_T = normalize(mul(m3, tangent.xyz))`, `tex_w = tangent.w`, `uv`,
  the 5 bindless slots + fallback `metallic`/`roughness` + `tex_mat_id` + `tex_base_color` as
  `nointerpolation` interpolants (`gbuffer_mrt.fs.hlsl:129-142`).
- `gbuffer_mrt.fs.hlsl` `-D TEXTURED`: Set 1 = `[[vk::binding(0,1)]] Texture2D gTextures[]` +
  `[[vk::binding(1,1)]] SamplerState gTexSampler` (register `t0/s0, space1`, lines 169-170),
  `NonUniformResourceIndex`-gated, per-instance `slot != 0` guarded; OGL normal-map `n_ts.y=-n_ts.y`
  (line 275). The WHOLE new block is under `#ifdef TEXTURED`, so the base (no-`-D`) compile is frozen.
- Host: `BindlessTextureTable` (`crates/boyko_render/src/bindless.rs`, `register`) — UPDATE_AFTER_BIND
  stable set, **no rebind on texture add**; `bindless.set().set_layout()` (`gpu_scene/mod.rs:3284`)
  is a **reusable layout-object handle**. Deferred TEXTURED pipeline is built **lazily** by
  `build_textured_resources` (`gpu_scene/mod.rs:3206`) *after* boot (the bindless Set-1 layout does
  not exist at `boot()`).
- Loader: `boyko_render::load_material_folder` (`texture.rs:844`) reads
  `<pack>/pbr/{albedo,normal,metallic_roughness,ao,emissive}.png`. Packs live in `assets/materials/`
  (gitignored, owner-supplied). Working references: `tests/pbr_material_showcase.rs`, `textured_smoke.rs`.

### 2.2 Forward / ForwardPlus design

**Shader variants** (opt-in; base `.spv` frozen). Forward FS variant count stays **4** (`{∅,froxel} ×
{∅,tex}`):

| New `.spv` | Compile | Notes |
|---|---|---|
| `forward_opaque_tex.vs.spv` | `-D TEXTURED=1` | VsIn += uv(loc3)/tangent(loc4); reads `PerInstanceMaterialTex`; forwards the `gbuffer_mrt.vs` TEXTURED interpolant set verbatim |
| `forward_opaque_tex.fs.spv` | `-D TEXTURED=1` | Set 1 = bindless table; samples 5 maps; OGL `n_ts.y=-n_ts.y`; feeds the SAME `Surface`/`eval_pbr_*` path |
| `forward_opaque_froxel_tex.fs.spv` | `-D FROXEL=1 -D TEXTURED=1` | ForwardPlus × TEXTURED cross (froxel walk is orthogonal to the texture block) |

The FS TEXTURED block is a near-verbatim splice of `gbuffer_mrt.fs.hlsl:223-320`, retargeted: instead
of writing G-buffer MRTs, it sets `base`/`metallic`/`roughness`/`emissive`/`n`/`ao_final` locals that
feed `forward_opaque.fs`'s existing `Surface`/light loop → the single BRDF source is preserved.

**Descriptor sets (the key structural decision).** Base forward is 2-set (Set 0 core, Set 1 shadow =
`forward_layout1`). TEXTURED forward is a **distinct 3-set variant pipeline**:

| Set | Base forward | TEXTURED forward | Rule |
|---|---|---|---|
| 0 | core (`forward_layout0`) | core + `PerInstanceMaterialTex` ring (`forward_tex_layout0`, new) | mirrors Deferred `tex_instance_material_layout` |
| 1 | shadow | **bindless texture table** | **reuses Deferred's `bindless.set().set_layout()` object verbatim** (R5 one-shared-layout rule — structurally-identical-but-distinct layout objects silent-black with validation off) |
| 2 | — | shadow (`forward_layout1`, reused, bound at index 2) | `shadow_apply.hlsli`'s "fixed type/global names, any binding numbers" idiom handles the Set-1→Set-2 shift; base arm verbatim ⇒ base golden frozen |

Because the base pipeline is a different object with a different layout, no base golden churns.
Vulkan `maxBoundDescriptorSets ≥ 4` ⇒ 3 sets is safe.

**Host wiring** — `build_forward_textured_resources(ctx, bindless)` (new, mirrors
`build_textured_resources`): built lazily after boot, producing `forward_tex_layout0` + a FIF-ringed
`PerInstanceMaterialTex` SSBO + the 3-set TEXTURED forward pipeline; caches the `bindless_set` (a
`Copy` handle) for the recorder to bind at Set 1 each textured frame. The recorder selects the
TEXTURED pipeline per-frame via `mesh_tex_active()` (Deferred's existing "any instance bound a
non-zero slot this frame" gather).

**W7 — instance-ring growth (a REAL coupling, hard-clamped).** `grow_shared_instance_rings`
(`gpu_scene/mod.rs:3489-3512`) rebinds `tex_bind_groups[s]`@0 (`instance_rings`) on a past-cap grow,
but `tex_bind_groups[s]`@1 (`tex_instance_material_rings[s]`, the `PerInstanceMaterialTex` ring) is
**FIXED at boot `INSTANCE_CAPACITY`=1024 and NOT rebound**. A scatter past cap = silent device OOB
(no validation layer — the F7-C1 class). **Resolution:** the forward TEXTURED scatter applies a
**hard structural clamp** `let n = gathered.min(INSTANCE_CAPACITY)` (survives release), mirroring the
Deferred TEXTURED gather's existing CPU OOB-clamp — overflow instances are dropped (the disclosed T6c
cap), never OOB-written. **NOT** a `debug_assert`. Charted follow-up (lifts the cap): add
`tex_instance_material_rings`@1 to `grow_shared_instance_rings` lockstep growth.

**Resolver changes: none structural.** Textures are *not* a pre-light consumer, so
`cap_forward_v1_consumers` correctly leaves them untouched. Texture activation stays a per-frame
gather + boot-lazy pipeline build, orthogonal to `ResolvedRenderPath`.

### 2.3 VB (optional)

- **Derivatives — solved** by `vb_uv_grad` → `SampleGrad`.
- **Tangent** — `vb_load_vertex` (`vb_geom_fetch.hlsli:100`) currently skips `tangent`@48; TEXTURED VB
  adds it to `VbVertex` + a `vb_interp` channel + world-tangent via the instance `m3`, then the SAME
  OGL TBN + `n_ts.y=-n_ts.y`. Carry `tangent.w` handedness as a flat nearest-vertex value (matching
  `gbuffer_mrt.vs`'s `nointerpolation tex_w`).
- **Material slots** — read `PerInstanceMaterialTex` (48 B ring) instead of `PerInstanceMaterial`.
- **Set budget** — VB already uses 3 real sets (0 core / 1 shadow / 2 geometry). TEXTURED VB adds the
  bindless table as **Set 3** → **4 sets = the Vulkan floor** (boot `debug_assert`). This is the one
  place the feature reaches the floor.
- New `.spv`: `vb_resolve_tex.comp.spv` (`-D TEXTURED`).

---

## 3. Feature 2 — SDF-object shadows on mesh surfaces

### 3.1 Reference (Deferred, done)
The marcher marches from the mesh surface point: `P_mesh = ro + rd*t_mesh`, reads back the raster
normal from `gNormal` (oct-decoded), marches `sdf_soft_shadow(P_mesh + N*bias, N, light)`, and writes
the result into `gMaterial.RG` — a **decoupled screen-space term** (`sdf_gbuffer_composite.hlsl:1863-1884`).
This is architecturally **Option B already**.

### 3.2 The options (Option B chosen, critic-agreed)

- **(A) Inline march in `forward_opaque.fs` / `vb_resolve`** — bind the 13-binding SDF vocab into the
  raster/compute shade pipeline, call `sdf_soft_shadow` per shaded pixel. **Rejected:** the forward FS
  runs per *covered fragment*, so without early-Z the engine's heaviest per-pixel op runs
  `overdraw ×` more than necessary; fat 13-binding descriptor set on the raster hot path (I/D-cache).
  Defeats Forward's low-overdraw premise; blows VB's set/binding budget (Option A under VB → 5 sets).
- **(B) Decoupled `sdf_mesh_shadow.comp` prepass** — reads mesh depth + normal, reconstructs `P_mesh`,
  marches once per screen pixel, writes an R8 term the shade sites sample & `min`-combine.
  **CHOSEN.** Overdraw-invariant (1 march/pixel); parity-by-construction with Deferred's existing
  decoupled term; reuses the already-bound SDF vocab of `sdf_forward_march.comp` (no SDF descriptors on
  the raster pipeline); folds into `ShadowSources::SDF_SOFT_MARCH` + `shadow_apply.hlsli`. External
  corroboration: UE `DistanceFieldShadowing.usf` is a separate screen-space pass for exactly this reason.
- **(C) Hybrid selector** — a knob over A/B via the existing `sdf_shadows_wanted` consumer; not a
  distinct algorithm.

### 3.3 C1 (resolved) — plain Forward needs an EQUAL shade pipeline

Plain Forward's shade pipeline is `create_graphics_pipeline_forward` = `GREATER + depth-write ON`
(`device.rs:1702`) and is the sole first-touch depth producer. Forcing a prepass + `LOAD_OP_LOAD`
would make `forward_opaque` re-test `GREATER(d,d)=false` → every fragment rejected → **silent black**
(validation off — the R5 class).

**Resolution — reuse ForwardPlus's proven EQUAL machinery.** When the resolver sets
`needs_depth_prepass` under plain Forward (only because SDF-shadow is armed — §3.5), boot selects a
two-pass mesh path identical in shape to ForwardPlus:
1. `depth_prepass` pipeline (`create_graphics_pipeline_forward_prepass`, `device.rs:1716` — GREATER +
   write-ON, depth-only) drives `depth_prepass.vs`, the sole depth producer.
2. The shade pipeline switches to `create_graphics_pipeline_forward_plus` (`device.rs:1759` —
   `VK_COMPARE_OP_EQUAL` + depth-write-OFF), carrying the **all-lights (non-froxel) `forward_opaque.fs.spv`**
   in its `desc`. **No new `.spv`** — that pipeline factory is parameterized by whichever FS the desc
   supplies; only a new `VkPipeline` object (existing VS+FS, EQUAL depth-state) is created.

**VS-position-identity requirement.** EQUAL passes iff `depth_prepass.vs`'s `SV_Position` is
bit-identical to `forward_opaque.vs`'s. `depth_prepass.vs.hlsl:17-24` is verified token-for-token
identical (same `instances[base+id]` → `mul(m3,pos)+t` → `mul(view_proj,…)`, same 88-byte push, same
`forward_view_proj_rows`). SF0 adds a `debug_assert`/CI check that both consume `forward_view_proj_rows`
(never the marcher matrix). **VB unaffected** — `vb_raster` is VB's sole depth producer;
`vb_resolve`/`vb_shade` are compute (no depth test), so the SDF-shadow pass just reads the existing
`vb` depth (no prepass).

**Trade-off / owner decision:** plain-Forward-with-SDF-shadow draws mesh geometry twice (prepass +
EQUAL shade) — the cost ForwardPlus always pays. **Alternative (owner VALUES call):** degrade
SDF-shadow OFF under *plain* Forward, supporting it only under ForwardPlus/VB where depth-before-shade
is free. See §6.

### 3.4 C2 (resolved) — the mesh normal for the march

`sdf_soft_shadow` needs the normal for the `dot(n,L)<=0 → 0.0` back-face early-out **and** the acne
bias `P_mesh + N*SHADOW_NORMAL_BIAS`. Deferred oct-decodes it from `gNormal`; Forward's depth-only
prepass has none.

**Resolution — the Forward prepass writes an oct-encoded `thin_normal` MRT when SDF-shadow is armed.**
This is the *planned* normal-consumer prepass form (Decision-8 prepass-MRT infrastructure), not a
divergence: the prepass VS gains the M⁴ inverse-transpose normal export (token-identical to
`forward_opaque.vs:145-153`); the prepass FS writes `oct_encode(N)` (the SAME eDSL `oct_encode` span
`gbuffer_mrt` uses) to a `thin_normal` R8G8B8A8 MRT; `sdf_mesh_shadow.comp` `oct_decode`s it → `N_mesh`,
exactly as Deferred does. **Parity:** this stores the interpolated *shading* normal — the identical
quantity `forward_opaque` lights with and Deferred stores — so the early-out and bias match Deferred
in representation and the term cannot flip 0/1 vs lit pixels at silhouettes. **Rejected — option (b)**
(reconstruct a geometric normal from depth derivatives): faceted, silhouette 0/1 flips, not parity.
Cost of (a): a color MRT on the prepass (forfeits double-rate depth-only rasterization — the same
disclosed Decision-8 cost as the motion MRT), benchmarked at SF0. **Under VB:** `sdf_mesh_shadow.comp`
re-fetches `N_mesh` via `vb_geom_fetch` (it holds `vb_id` + the geometry table) — VB's native model,
no `thin_normal` MRT.

### 3.5 W1 (resolved) — resolver wiring (separate depth-prepass term)

`sdf_mesh_shadow` is **not** folded into the shared `pre_light` union
(`render_path_config.rs:676-680`) — that would flip `mesh_geo_shade_split`/`sdf_geo_shade_split`/
`sdf_surface_cache` true under Forward×Both (split mode Forward doesn't implement). It is a separate
depth-prepass term tied to the existing `ShadowSources::SDF_SOFT_MARCH` bit:

```rust
// render_path_config.rs, resolve_rules — CHANGED:
let sdf_mesh_shadow = shadow.contains(ShadowSources::SDF_SOFT_MARCH) && mesh_leg; // W6: SDF_SOFT_MARCH already ⇒ sdf_leg

// pre_light UNTOUCHED (ssao|ddgi|denoise_spatial|temporal|ssr) — it alone gates the splits.
let needs_depth_prepass = mesh_leg
    && (matches!(path, RenderPath::ForwardPlus)
        || (matches!(path, RenderPath::Forward) && (pre_light || sdf_mesh_shadow)));

let prepass_writes_normal = thin_aux.contains(ThinAuxMask::NORMAL) || sdf_mesh_shadow; // C2
```

`RenderPathConsumers` gains `sdf_mesh_shadow_on: bool` (host-threaded, mirrors `sdf_shadows_wanted`);
`ResolvedRenderPath` gains `sdf_mesh_shadow: bool` + `prepass_writes_normal: bool`.

**Cap exemption (R8-widening lesson, deliberate).** `cap_forward_v1_consumers`
(`render_path_config.rs:865-878`) caps `ssao/ddgi/denoise_spatial/temporal/ssr/hwrt`.
`sdf_mesh_shadow_on` is deliberately **absent** from that list → exempt: its producer
(`sdf_mesh_shadow.comp`) reads only depth + normal + the edit-list, independent of the still-unimplemented
SSAO/DDGI thin-aux producers. Grep-sweep every per-path `matches!(...)` and every consumer-cap for the
new consumer; SF0's resolver truth-table test asserts `sdf_mesh_shadow_on` survives the cap AND that
`pre_light` (hence the splits) stays `false` when only `sdf_mesh_shadow_on` is set.

### 3.6 The `.spv` matrix decision (W5) — runtime uniform gate, one-time re-pin

**Decision: a runtime uniform gate, NOT `-D SDF_SHADOW`.** `forward_opaque.fs` unconditionally (in
source) declares one new binding + one boot-set uniform lane and runs, at the primary-directional shade
site (after the CSM `min`), the codebase's proven 0%-gate idiom (mirrors `pc.brick_enabled != 0u`):

```hlsl
[[vk::binding(B, S)]] Texture2D<float> gSdfMeshShadow;   // R8_UNORM, per-FIF (W3)
// uint sdf_mesh_shadow_enabled;  // a lane in the existing extent/Camera UBO, host-set once at boot
float sdf_sh = 1.0;
if (sdf_mesh_shadow_enabled != 0u)          // runtime-uniform 0%-gate, never folded
    sdf_sh = gSdfMeshShadow.Load(int3(px, py, 0)).r;
vis = min(vis, sdf_sh);                      // primary directional only (Deferred R-lane parity)
```

- **The base-`.spv` freeze tension, resolved honestly.** Adding the binding + gate **changes**
  `forward_opaque.fs.spv` bytes — it cannot stay `cmp`-frozen. This is a **one-time, image-golden-proven
  re-pin** at SF0, sanctioned by **Decision 3** (image goldens are authoritative; `.spv`-cmp is
  secondary): every currently-blessed config threads `sdf_mesh_shadow_enabled=0`, so `min(vis,1.0)=vis`
  and `forward_mesh`/`forward_both`(pre-shadow)/`forwardplus_mesh`/`58f6c6c3` all reproduce
  **byte-identically**. After SF0 the new `.spv` set is the pinned baseline. This buys a **2× smaller
  pipeline matrix** (4 forward FS variants, not 8) vs the compile-flag explosion.
- **Zero-cost when unarmed** (honors W6): the binding is always in the layout; when off it points at a
  shared **1×1 R8 boot placeholder**, never `.Load`ed (the uniform gate skips it) — cheaper than a
  full-res image cleared to 1.0 each frame. When armed, it points at the per-FIF full-res R8. The lane
  lives in the existing extent/Camera UBO (`forward_opaque.fs.hlsl:98`) — no push-constant/VS change.

**Scope note.** SF0 wires the **shadow** term (Deferred's `gMaterial.R` lane, primary-directional
`min`). Deferred's `gMaterial.G` contact-AO on mesh is a distinct ambient term — a charted follow-up,
not part of the "SDF-object shadows on mesh" ask.

### 3.7 W3 / W4 — per-FIF image + depth barrier round-trip

- **W3.** `sdf_mesh_shadow` is a **per-FIF ring** (`[VulkanTexture; FRAMES_IN_FLIGHT]`, like
  `ForwardTargets::depth`) so frame N+1's compute WRITE never races frame N's forward READ
  (cross-frame WAR "torn shimmer in motion"). Appended last to the fixed framegraph ResId order.
- **W4.** Depth ping-pongs `DEPTH_ATTACHMENT_WRITE` (prepass) → `SHADER_READ_ONLY`
  (`sdf_mesh_shadow.comp` `.Load`) → `DEPTH_ATTACHMENT` (forward_opaque EQUAL test). All three passes
  declare their depth access so the framegraph auto-derives the `SHADER_READ_ONLY → DEPTH_ATTACHMENT`
  round-trip (the same auto-barrier machinery `sdf_forward_march`'s depth `.Load` already relies on).
  `forward.depth` already carries SAMPLED usage (`targets.rs:477`) → no image-create change. SF0 adds a
  framegraph test asserting the derived barrier set contains that round-trip transition (a wrong layout
  is silent) + the O1 declare/record parity `debug_assert`.

### 3.8 Ordering & the pass placement

Pass order (mirrors how `csm`/`atlas`/`light_cull` are declared *before* `forward_opaque` so their
outputs can be sampled): `depth_prepass → sdf_mesh_shadow → (csm/atlas/light_cull) → forward_opaque`.
ForwardPlus already has the prepass unconditionally (free); VB has `vb_raster → sdf_mesh_shadow → vb_shade`
(mesh depth from `vb_raster`, no prepass).

---

## 4. Data structures

```rust
// render_path_config.rs
pub struct RenderPathConsumers {
    // ...existing...
    /// F2: mesh pixels should receive the SDF geometry's soft shadow (Option B decoupled term).
    /// Host-set when sdf_leg && mesh_leg && shadows-on && !hwrt. A separate depth-prepass trigger
    /// under Forward (NOT in the pre_light union) and EXEMPT from cap_forward_v1_consumers.
    pub sdf_mesh_shadow_on: bool,
}

pub struct ResolvedRenderPath {
    // ...existing...
    pub sdf_mesh_shadow: bool,       // SDF_SOFT_MARCH armed && mesh_leg
    pub prepass_writes_normal: bool, // thin_aux.NORMAL || sdf_mesh_shadow (C2)
}
```

`VbVertex` (`vb_geom_fetch.hlsli:91`) gains `float4 tangent;` (loaded from @48) under `-D TEXTURED`.
`PerInstanceMaterialTex` (48 B, `mesh_draw.rs:169`, offsets pinned) is reused as the forward/VB texture
ring. `ResolvedRenderPathGpu` (the POD device mirror, `gpu_scene/mod.rs:316`): append the two new bools
host-side; O2 — verify appending preserves any device-read prefix (else base goldens perturb), and
extend its default/round-trip tests.

---

## 5. Rung breakdown (final, Rev 2)

Every rung: default `Deferred × Both` byte-identical (`58f6c6c3` ±hwrt, `a5ad662d`, `f6147f90`);
existing `forward_mesh`/`forward_both`(pre-shadow)/`forwardplus_mesh`/`vb_mesh` byte-identical (image
goldens — after SF0 the re-pinned `forward_opaque.{vs,fs}.spv` is the frozen baseline); `clippy -D
warnings`; full suite; Miri where new `unsafe`; author-only commit+push. Sequence **Forward → ForwardPlus
→ SDF-shadow → optional VB**.

| Rung | Lands | New `.spv` | New golden | Size |
|---|---|---|---|---|
| **TF0** Forward TEXTURED foundation | `forward_opaque_tex.{vs,fs}`; `build_forward_textured_resources`; `forward_tex_layout0` + **Set 1 = bindless (Deferred's SAME layout object, R5)** + shadow→Set 2; forward `PerInstanceMaterialTex` ring; `mesh_tex_active()` select; **W7 hard clamp `min(gathered, INSTANCE_CAPACITY)`** | `forward_opaque_tex.vs/fs` | `forward_mesh_tex` (blessed vs Deferred `pbr_material_showcase`) | **L** — 3-set layout + shared-layout-object identity assert; TEXTURED pixel-matches Deferred TEXTURED; W7 OOB test (`CAP+1` → in-bounds) |
| **TF1** ForwardPlus TEXTURED | `forward_opaque_froxel_tex.fs` (froxel × tex) | `forward_opaque_froxel_tex.fs` | `forwardplus_mesh_tex` | **S** — orthogonal define combine |
| **SF0** SDF-shadow-on-mesh (Forward + ForwardPlus, Option B) | `sdf_mesh_shadow.comp` (verbatim `sdf_soft_shadow` + **O3 sync-pin** over span+consts+`Buf`); **per-FIF R8 (W3)** + 1×1 placeholder; **C1** prepass + EQUAL/write-OFF shade pipeline (reuse `..._forward_prepass` + `..._forward_plus` w/ all-lights FS); **C2** prepass `thin_normal` MRT (`prepass_writes_normal`); **W1** resolver (separate prepass term, cap-exempt) + `ResolvedRenderPath.{sdf_mesh_shadow,prepass_writes_normal}`; **W5** runtime uniform gate → **one-time `.spv` re-pin**; `shadow_apply.hlsli` min-combine; **W4** depth round-trip barrier; **O5** doc fix; `path_has_sdf_mesh_shadow` predicate (declare+record) | `sdf_mesh_shadow.comp`; **re-pinned** `forward_opaque.{vs,fs}` (+ froxel/tex crosses) | `forward_both_sdfshadow`, `forwardplus_both_sdfshadow` (blessed) | **L** — **W2** gate: (a) host-oracle bit-exact reverse-Z reconstruct, (b) march-span sync-pin, (c) owner golden; C1 VS-position-identity check; W6 provable no-op on Mesh-only; framegraph barrier round-trip test; cap-exemption grep-sweep |
| **TV0** *(optional, VB)* VB TEXTURED | `vb_resolve_tex.comp` (tangent + `SampleGrad` via pre-built `vb_uv_grad` + **Set 3 bindless-tex**, shared layout object); VB tex ring + W7 clamp | `vb_resolve_tex.comp` | `vb_mesh_tex` | **M** — derivatives pre-solved; **O1 4-set floor assert**; tangent-interp handedness |
| **SV0** *(optional, VB)* VB SDF-shadow-on-mesh | reuse `sdf_mesh_shadow.comp` under VB (mesh depth from `vb_raster`, **no prepass**; `N_mesh` via `vb_geom_fetch`); one `gSdfMeshShadow` binding added to vb **Set 0** (**O1: no 5th set**); runtime gate in `vb_resolve`/`vb_shade` | `vb_resolve_sdfshadow.comp` (or re-pin) | `vb_both_sdfshadow` | **M** — reuses SF0 pass; `vb_raster → sdf_mesh_shadow → vb_shade` |

---

## 6. Risks / open owner decisions (VALUES/SCOPE)

1. **VB in v1 scope?** Owner scoped `forward, forward+, deferred`; VB (TV0/SV0) is specified as an
   optional follow-up, drop-in.
2. **Plain-Forward SDF-shadow prepass cost.** Supporting SDF-shadow under *plain* Forward forces a
   depth prepass (mesh drawn twice — the ForwardPlus cost). Alternative: degrade SDF-shadow OFF under
   plain Forward, support it under ForwardPlus/VB only (depth-before-shade free there). Owner call.
3. **SF0 base-`.spv` one-time re-pin acknowledged?** `forward_opaque.{vs,fs}.spv` change bytes once
   (the runtime gate binding); image goldens reproduce byte-identically and are the authoritative gate
   (Decision 3). Confirm the re-pin is acceptable (vs the 8-`.spv` compile-flag alternative).
4. **Contact-AO (Deferred `gMaterial.G`) on mesh under Forward/VB** — out of scope here; charted
   follow-up.

Decisions already made (critic-agreed): Option B for F2; runtime gate over `-D SDF_SHADOW`; hard clamp
over `debug_assert` (W7); EQUAL-pipeline reuse (C1); prepass `thin_normal` MRT (C2).

---

## 7. Validation

- **Resolver truth table** — extended for `sdf_mesh_shadow_on`: the separate prepass term (not in
  `pre_light`), cap exemption, `sdf_mesh_shadow`/`prepass_writes_normal` derived flags across all
  path×leg combos; W6 no-op on Mesh-only.
- **Layout-object identity** — TEXTURED forward binds the *same* `VkDescriptorSetLayout` handle as
  Deferred TEXTURED (R5 silent-black regression guard).
- **Bit-exact GPU-vs-host** — SF0 reverse-Z `P_mesh` reconstruction (reuse `forward_view_z_coeffs`,
  the existing round-trip test family); `PerInstanceMaterialTex` 48 B offset pins (reused).
- **eDSL sync-pin** — `sdf_field_edsl_sync` extended so `sdf_mesh_shadow.comp` `.contains()`-asserts
  the generated `sdf_soft_shadow` span **+** the `SHADOW_*` consts (`sdf_forward_march.comp.hlsl:244-249`)
  **+** the `sdf_field.hlsli` / `Buf @t0` precondition (O3).
- **Framegraph** — the depth `SHADER_READ_ONLY→DEPTH_ATTACHMENT` round-trip barrier is derived +
  declare/record parity (O1).
- **W7 OOB test** — `INSTANCE_CAPACITY+1` textured instances → ring write stays in-bounds.
- **Benchmarks (criterion + GPU capture)** — textured FS gather cost vs non-textured; **Option B vs a
  synthetic Option A** at overdraw 1×/2×/4× (must show B flat, A linear — locks the recommendation);
  plain-Forward prepass cost; descriptor-set-count asserts (TEXTURED forward = 3, TEXTURED VB = 4).

---

## Appendix — file:line anchors (verified)

- **Textures (reference):** `gbuffer_mrt.fs.hlsl:169-170` (bindless Set 1 decl), `:223-320` (TEXTURED
  body, `n_ts.y=-n_ts.y`@275); `gbuffer_mrt.vs.hlsl:129-142` (interpolants); `material.rs:144-170`
  (`MaterialTextures{albedo,normal,metal_rough,ao,emissive}`); `mesh_draw.rs:169` (`PerInstanceMaterialTex`
  48 B); `bindless.rs` `register` (no-rebind); `gpu_scene/mod.rs:3206` (`build_textured_resources`),
  `:3284` (`bindless.set().set_layout()`), `:3489-3512` (`grow_shared_instance_rings`, @1 not rebound),
  `:127` (`INSTANCE_CAPACITY`).
- **Textures (seam):** `forward_opaque.fs.hlsl:31-37`, `:133` (shadow Set 1); `vb_geom_fetch.hlsli:100`
  (skips tangent), `:211-307` (`vb_uv_grad`), `:566-567` (perspective-correct UV).
- **SDF shadow (reference):** `sdf_gbuffer_composite.hlsl:1863-1884` (Deferred decoupled term);
  `sdf_forward_march.comp.hlsl:258-278` (sync-pinned `sdf_soft_shadow` copy), `:244-249` (`SHADOW_*`
  consts), `:864` (reverse-Z `view_z = B/(depth−A)`), `:882` (`brick_enabled` 0%-gate idiom).
- **Pipelines / depth:** `device.rs:1702` (`create_graphics_pipeline_forward` GREATER/write-ON),
  `:1716` (`..._forward_prepass` depth-only), `:1759` (`..._forward_plus` EQUAL/write-OFF), `:1737`
  (`..._vb_raster`); `present/passes/forward.rs:662-666` (depth load-op select), `:839` (HAS_MESH
  reverse-Z coeffs); `depth_prepass.vs.hlsl:17-24` (position-identity); `forward_opaque.vs.hlsl:145-153`
  (normal export); `present/targets.rs:477` (forward.depth SAMPLED usage).
- **Resolver:** `render_path_config.rs:648-680` (`pre_light` union + splits), `:707`
  (`depth_kind`), `:727` (`SDF_SOFT_MARCH` = `sdf_leg && sdf_shadows_wanted && !hwrt`), `:856,865-878`
  (`cap_forward_v1_consumers`); `gpu_scene/mod.rs:316` (`ResolvedRenderPathGpu`).
- **Shadow combine / doc:** `shadow_apply.hlsli:37-68` (includer-declared bindings), `:66-67` (stale
  "Set-2" doc — O5 fix).
