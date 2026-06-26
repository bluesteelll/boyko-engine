# Render P7 — Screen-Space AO (HBAO-lite, no-trig) as a 2nd AO strategy

Status: PLAN (locked). Branch `ecs`. Produced by the P7 design workflow (research → 2 designs
→ adversarial critique → synthesis). Governing law: **Principle 9** (a capability carries N
strategies, near-zero-cost selector at the coarsest stable granularity, admitted ONLY with a
real winning situation, 0%-gate = OFF path byte-identical).

## Why & the winning situation (Principle 9 admission)

AO today has ONE strategy: the **A2 SDF march** (`sdf_ao` / `host_ao`, 5-tap iquilezles normal
march into `gMaterial.g`). It is exact for the SDF field but:
1. **Mesh pixels** (raster-owned, P5) have NO field → `gMaterial.g` is forced to `1.0` (no AO at
   all). SSAO is the **only** AO option there — a *structural* capability win.
2. **Cross-representation occlusion** (a mesh occluding an SDF pixel, or disjoint SDF edits beyond
   the local `5·AO_STEP = 0.5` world-unit march reach) — the per-field march returns ~1; a
   screen-space gather sees the occluder. A capability win the analytic path cannot reach.

**Perf is NOT in the admission basis**: on SDF pixels the combine is *additive* (the marcher still
runs the 5-tap A2 AND the SSAO pass runs), so it is never cheaper. Admission rests SOLELY on the
two structural capability wins above. (Hybrid-perf rule: SSAO is admitted for what analytic per-
field AO *cannot do*, never "because screen-space is nicer".)

## Chosen algorithm — full-res HBAO-lite (no trig), deterministic, no separate blur

| Decision | Choice | Justification |
|---|---|---|
| Family | **HBAO-lite** (horizon max-cosine reducer), NOT GTAO arc | GTAO's arc integral needs `sin/cos/acos` → an irreducible GPU↔CPU transcendental ULP gap that NO stated tolerance covers, and the IGN `fract` rotation has an integer-boundary discontinuity under FMA. The horizon reducer needs only `dot/max/sqrt/div` → bit-comparable host oracle. **This is the single decision that makes P7 shippable/verifiable.** |
| Reconstruction | From `gViewT` + the shared `composite_ray` (NOT a proj-matrix inverse) | `composite_ray`/`oct_decode` are already host-mirrored; no precision loss. |
| Taps | **2 slices × 4 steps × 2 horizons = 16 taps**, full-res | HBAO classic = 32; 16 is the floor that survives single-frame with NO blur. `pow2` step distribution clusters samples toward crevices. |
| Rotation | **Integer-hash dither** (NOT float `fract` IGN) | `uint h = px*1103515245u + py*12345u; slot = h % ROT_N;` → a pre-baked `(cos,sin)` rotation-table lookup. Integer-only → bit-exact GPU↔host, no FP-contraction / integer-boundary jump. |
| Blur | **None (MVP)** | The deterministic rotation is structured/stable/mirror-able. A deterministic depth-aware 3×3 bilateral is a *later* P9 increment, only if a moving-camera showcase measures it needed. |
| Combine | **select-by-class then `min`**: `final_ao = min(class_ao, ssao)`, `class_ao = (view_t ≥ 1e30 ? 1.0 : gMaterial.g)` | UE5 precedent (DFAO/SSAO are min/alternatives, never multiplied). Mesh→pure SSAO; SDF→exact march unless SSAO sees a cross-rep occluder. |
| Feeds | **Ambient term only** | Matches A2 today + most engines; direct/spec keep the analytic A1 shadow. |
| Projection | **Forward-only**: offset in *screen pixels*, march the horizon in screen space; reconstruct each tapped pixel forward via `composite_ray(px',py')·gViewT'` | NO world→pixel inverse (that is the un-mirrorable step). The only rounding is `px' = round(px + dir.x·step·pix_radius)` — identical integer math GPU & host. |

**Honest combine semantics**: `min(class_ao, ssao)` on SDF pixels is NOT clean isolation — it
overrides whenever SSAO is darker for any reason, including its own residual noise. Accepted as a
hybrid trade because (a) AO modulates *only* ambient (a fraction of the lit pixel), (b) the
deterministic no-blur kernel is low-variance, (c) the **default is OFF**, so the exact A2 path is
fully preserved whenever SSAO is not admitted.

## Pass structure, barriers, targets

Slot: `raster MRT → barrier → marcher (G-buffer in GENERAL) → [marcher→reader barrier] → **SSAO
compute** → [SSAO→resolve barrier] → resolve → present`. All SSAO recording gated
`if let Some(ssao) = scene.ssao { … }` (the verified coarse-/L1-cull `Some`-guard pattern).

- **New target** on `GBufferTargets`: `ssao: Option<VulkanTexture>` (R8_UNORM, STORAGE, full
  `present_extent`, lives in GENERAL its whole life) + `ssao_set: Option<…>` (written once per
  extent in `sync_gbuffer`, only when `ssao.is_some()`). `Option<>` ⇒ OFF allocates NOTHING.
  Add `r8_unorm_storage_ok` to the `DeviceCaps` fail-fast (fallback `R8G8B8A8_UNORM`, use `.r`).
- **SSAO bind-group layout** (dedicated, 5 bindings): 0 `gNormal` (R), 1 `gMaterial` (R, mask `.b`),
  2 `gViewT` (R), 3 `ssao` out (W), 4 camera UBO (the 80B `CompositePushConstants` block).
  `gAlbedo` NOT bound.
- **Resolve binding = 11** (the cap is **16**, `device.rs`; the resolve uses 0–10 today, so 5 slots
  free): `[[vk::image_format("r8")]] RWTexture2D<float> gSsao : register(u11);`. OFF-path: bind a
  persistent **1×1 R8_UNORM placeholder** (never read because the structural `if` is false) so the
  descriptor interface is **stable regardless of DXC dead-code elimination**.
- **Barriers** (each gated on `ssao.is_some()`): (1) input — the existing marcher→resolve
  store-to-load barrier already makes `gNormal`/`gMaterial`/`gViewT` visible (confirm its image set
  ⊇ those three at impl time; it does) → **reuse, no new input barrier**; (2) add `&targets.ssao`
  to the existing `UNDEFINED→GENERAL` transition batch loop, inside the `Some` guard; (3) SSAO
  dispatch (`dispatch_group_count_x, 1, 1`, the marcher/resolve 1D grid); (4) NEW
  `COMPUTE→COMPUTE`, `SHADER_WRITE→SHADER_READ`, GENERAL→GENERAL barrier on `targets.ssao` so the
  resolve's `gSsao.Load` sees the store.

**OFF command stream = byte-identical**: `scene.ssao == None` ⇒ no transition add, no dispatch, no
output barrier, no image allocation.

## Selector + 0%-gate

- **Global (record-time)**: SSAO pass recorded iff `scene.ssao.is_some()` (a CPU branch, zero GPU
  cost). The resolve combine gated by an `ssao_mode` **header word** — a structural
  `if (ssao_mode != 0u)`, the exact zero-cost mechanism `shadow_mode` uses (word 7).
- **Per-pixel (ON path only)**: class from the `view_t >= 1.0e30` sentinel — **already loaded** at
  `deferred_pbr.hlsl` for `P` reconstruction → zero extra fetch. (Mesh pixels carry the `1e30`
  sentinel; SDF-lit pixels carry the real `t`; both `mask==1`.)
- **The flag = header word 11** (verified free: 0–3 counts/exposure, 4–6 sky_diffuse, 7
  shadow_mode, 8–10 sky_spec, **11 FREE**, 12–15 cluster_params). `load_ssao_mode(LightBuf) →
  LightBuf[11]`, mirroring `load_shadow_mode`. Word 11 is 0 on every pre-P7 scene ⇒ automatic
  byte-identical 0%-gate. A test asserts word 11 == 0 on all pre-P7 golden scene tables.

The resolve consume (structural `if`, NOT `min(x,1.0)` which is not an FP no-op for x slightly >1):
```hlsl
float ao_final = ao;                                   // ao = gMaterial.g (today)
if (ssao_mode != 0u) {                                 // load_ssao_mode(LightBuf) == word 11
    float ao_class = (view_t >= 1.0e30) ? 1.0 : ao;    // mesh→no field AO; SDF→exact march
    ao_final = min(ao_class, gSsao.Load(coord).r);     // cross-rep: most-occluded wins
}
ambient += (spec_ambient + diff_ambient) * ao_final;   // (the existing line, ao→ao_final)
```
When `ssao_mode==0`: `ao_final == ao == gMaterial.g`, `gSsao.Load` never executes → arithmetic
byte-identical to today. **The resolve `.spv` WILL change and `DEFERRED_PBR_SPV` WILL bump** —
OFF *pixels* are byte-identical (the gate the test asserts), NOT the `.spv` hash.

## eDSL authoring (`boyko_shaderdsl/src/ssao.rs`, sibling of `shadow.rs`)

**ZERO new transcendental leaves, ZERO float `fract`/`floor`** — the no-trig algorithm is chosen
precisely so the host oracle is bit-comparable. **REFINEMENT to the synthesis (orchestrator):
author `dot` INLINE** (`a.x()*b.x() + a.y()*b.y() + a.z()*b.z()` via existing `Cf` component reads +
`FieldScalar` mul/add) rather than adding a `vec3_dot` leaf — this adds **ZERO** eDSL leaves, so the
frozen marcher/field/shadow/brick/resolve `.spv` **physically cannot fork** (no enum/printer change)
and Step-1's firewall risk disappears. Still run all existing sync pins as a sanity gate.

- `ssao_horizon_step_body` — one horizon sample: `delta = P' - P`,
  `falloff = clamp01(1 - dot(delta,delta)/(R*R))` (range check),
  `sampleCos = dot(delta, slice_dir3) / max(length(delta), eps)`,
  `horizonCos = max(horizonCos, sampleCos * falloff)`. (`length = sqrt(dot(d,d))`.)
- `ssao_slice_body` — the ±dir 2-horizon reduction over `STEPS` (unrolled / `runtime_for`).
- `ssao_estimate_body` — the `SLICES` fold → `occ`, `ao = clamp01(1 - SSAO_STRENGTH·occ/SLICES)`,
  then `ao*ao` (integer self-mul power, NOT `pow`). Tuning consts (`R`, `SLICES`, `STEPS`,
  `SSAO_STRENGTH`, `SAMPLE_DISTRIBUTION_POWER`, `BIAS`, rotation-table values) via `named_lit`.
- HBAO works in the **slice plane** — NO tangent basis, NO `cross`/`normalize` (the "projected
  normal" is `dot(N, slice_axis)`-based from existing component reads + scalar `dot`).

**Irreducible hand-written glue** (legitimate FFI-like exception, OUTSIDE the generated span):
`[numthreads(64,1,1)]` entry + `SV_DispatchThreadID` + `idx<count` + `px/py` decode; the 5 resource
decls + `#include "ray_gen.hlsli"` (the SHARED ray-gen, NOT re-authored) + the
`composite_ray`/`gViewT.Load`/`gNormal.Load`/`gMaterial.Load`/`gSsao` store call sites; the
integer-hash rotation + table lookup (integer, bit-exact, mirrored in host); the `camera_mode`
branch for `pix_radius` (`persp: R·(h/2)/(z·tan_half_fov)` clamped `[2, RADIUS_PIX_MAX]`;
`ortho: R·(h/2)/SDF_HALF_EXTENT`); the forward neighbor reconstruct `composite_ray(px',py')·gViewT'`.

**Sync pins** (`tests/ssao_edsl_sync.rs`): `emit_hlsl_ssao()` `.contains()`-gated against the
committed `sdf_ssao.comp.hlsl` GENERATED span + a re-DXC byte-identity test
(`dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`) asserting the regenerated
`.spv == sdf_ssao.comp.spv`.

The resolve `min`-combine span is **hand-written glue** (the AO consume is plain HLSL today, not
eDSL-owned — mirrors the hand-written `shadow_mode` per-light select), an explicit owner-sanctioned
exception (consumer-side, not field math).

## Host oracle + golden + tolerance

Two-stage gather (the legitimate Principle-0 transient-test-scratch exception):
- **Stage 1** — build the host G-buffer ONCE per golden invocation:
  `let gbuf: Vec<MarcherAttributes> = (0..w*h).map(|i| golden_marcher_attributes(…)).collect();`
  (`view_t` exact fp32; `oct_rg` byte-quantized as the GPU reads). Shared across all pixels (NOT
  O(N²)).
- **Stage 2** — `golden_ssao_attributes(gbuf, px, py, …) -> f32`: reconstruct center via
  `composite_ray` + `oct_decode`, compute the SAME integer-hash rotation, then CALL
  `ssao_estimate_body::<EvalCf>(…)` with a host neighbor-fetch closure reproducing the EXACT forward
  march (`px' = round(px + dir.x·step·pix_radius)`, bounds-clamp, read `gbuf[py'*w+px'].view_t/.mask`,
  skip if `mask != 1 || view_t >= 1e30`). Host == GPU by construction (dual-instantiation).

**Determinism**: integer rotation + integer step-rounding are bit-exact; the only GPU↔host
divergence is `sqrt`/`div` last-ULP (the parity `composite_ray` already relies on) — bounded, not a
discontinuity. The oct-byte `±1/255` normal disagreement propagates linearly through
`dot(N, slice_dir)` (no basis amplification) → bounded.

**Golden tolerances**: AO channel (R8 readback vs `golden_ssao_attributes`) **±6/255** (consumer-
side; the `sqrt`/`div` ULP budget). Combined resolve pixel (lit RGBA8) keeps the **existing ±2/255**
(AO modulates only ambient, compressing the AO delta) — do NOT relax the established pixel gate.

**Non-vacuity proofs** (count-based bands, tolerate ±1px edge disagreement WITHOUT masking):
1. **Mesh AO**: a mesh quad in a concave corner — ≥N mesh pixels `final_ao ≤ 0.85` (today exactly
   1.0 there ⇒ any darkening is SSAO), AND corner darker than open-face by ≥20/255 in the lit image.
2. **Cross-rep**: a mesh box touching an SDF sphere — ≥M contact-crevice pixels strictly darker than
   march-only `gMaterial.g` (a monotone darkening-band COUNT, not a per-pixel edge match).
3. **Flat-region invariance** (AWAY from edges): an open flat region within ±2/255 of the OFF image.
4. **0%-gate golden**: `ssao_mode=0` LIT image == pre-P7 LIT image byte-for-byte.

## Ordered, independently-testable steps (commit groups in braces)

1. **{1}** *(SKIPPED per the orchestrator refinement — inline `dot`, no new leaf.)* Sanity:
   run ALL existing sync pins + every frozen `.spv` re-DXC byte-identity — must be UNCHANGED
   (no leaf added). Gate: do not proceed unless green.
2. **{2}** `boyko_shaderdsl/src/ssao.rs` (new): author the 3 bodies over `<C: Cf>` using only
   existing ops (inline `dot`) + `named_lit` consts + rotation table. Unit-test
   `ssao_estimate_body::<EvalCf>` on a hand-built tiny G-buffer (corner→darker, flat→~1.0).
3. **{2}** `emit.rs` `emit_hlsl_ssao()` + emit bin; hand-write the glue frame; splice the GENERATED
   span; DXC → `shaders/sdf_ssao.comp.spv`.
4. **{2}** `tests/ssao_edsl_sync.rs`: `.contains()` drift gate + re-DXC `.spv` byte-identity.
5. **{3}** `compute.rs`: `golden_ssao_attributes` (Stage-1 gbuf ONCE + Stage-2 gather via
   `ssao_estimate_body::<EvalCf>`); SSAO tuning consts next to `AO_STEP`. Unit-test on a synthetic
   gbuf.
6. **{3}** `light_table.hlsli` + `deferred_pbr.hlsl` + `compute.rs`: `ssao_mode` = header word 11 +
   `load_ssao_mode`; resolve structural-`if` `min`-combine (reuse `view_t`, `gMaterial.g`, new
   `gSsao @11`); mirror in `golden_deferred_resolve_table[_shadowed]`. Re-DXC
   `deferred_pbr.comp.spv`, bump `DEFERRED_PBR_SPV`. Host test: `ssao_mode==0` resolve byte-identical;
   assert word 11 == 0 on all pre-P7 scene tables.
7. **{4}** `swapchain.rs` `GBufferTargets`: `ssao: Option<VulkanTexture>` + `ssao_set` + resolve
   layout binding 11 + 1×1 R8 placeholder for OFF + destroy/error-unwind arms + `r8_unorm_storage_ok`
   cap. `cargo check --all-targets`.
8. **{4}** `swapchain.rs` `record_gbuffer` + `GBufferScene`: `scene.ssao: Option<SsaoActivation>`;
   record transition/dispatch/barriers gated on `is_some()`; set `ssao_mode` IFF `is_some()`. Confirm
   the marcher→resolve barrier already covers {normal,material,viewt} (reuse). Record-op-log test:
   `scene.ssao=None` → byte-identical pre-P7 command stream.
9. **{5}** `tests/sdf_gbuffer_hybrid.rs`: GPU AO-channel golden (±6/255), combined-pixel golden
   (±2/255), the 4 non-vacuity proofs, 0%-gate LIT byte-identity. `#[ignore]` offscreen screenshot.
   `BOYKO_DISABLE_VALIDATION=1`.
10. **{6}** `boyko_render`/`boyko_demo`: expose the SSAO pipeline + `ssao_mode` toggle for a windowed
    A/B. Owner visual oracle (BMP→PNG) before commit.

## Constraints (HARD)

- Shaders via the eDSL (byte-identical `.spv` + Eval mirror + sync pins). ZERO unsafe in
  `boyko_shaderdsl`.
- 0%-gate: SSAO default OFF → byte-identical command stream + pixels.
- Principle 0: no parallel data system (the host-oracle gbuf `Vec` is the legitimate transient-
  test-scratch exception).
- GPU validation broken on the dev box → `BOYKO_DISABLE_VALIDATION=1` for all GPU tests
  (windows-gnu). DXC recipe: `-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3` (no `-O`).
