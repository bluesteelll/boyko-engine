# Rung 3a — spatial (à-trous) RT soft-shadow denoise

Opt-in, single-frame (NO history → no ghosting on moving boxes), engine-native
"SPATIAL not temporal". Default `None` = **byte-identical** to today (`58f6c6c3`
disarmed, `af934c50` RT-armed). Full design: workflow `wf_20c2c59f-46d` output.

## The pivot (why it is correct)

The RT shadow trace is INLINE in the resolve, deeply nested in the primary-directional
CSM arm (`deferred_pbr.hlsl` ~:1091-1140), using live `P, n, l, NoL, view_t,
csm_view_z, px, py`. A from-scratch pre-pass would reconstruct `P` from gViewT (lossy
r32f round-trip) + re-derive `l` per light → **breaks byte-identity** (the v1 fatal flaw
the critic caught). Instead: the **resolve shader IS the pre-pass**, compiled in THREE
`SHADOW_STAGE` variants of ONE source (like the HWRT split already trusted):

- `RESOLVE_INLINE` (=None, default): preprocesses to **exactly today's shader** (inline
  Vogel loop verbatim, `vis = min(vis, mesh_vis)`, then lights). Byte-identical by
  construction (new `#if`/`#else` legs dead-strip).
- `VIS` (ON): identical call-site + identical live inputs → runs the Vogel loop, then
  **writes** `gShadowVis[px,py] = RG8(mesh_vis, validity)` and RETURNS (lighting stripped).
  Non-mesh-arm pixels write `RG8(1.0, 0.0)` (validity=0). Zero reconstruction → `mesh_vis`
  is bit-identical to the inline path.
- `RESOLVE_DENOISED` (ON): identical to RESOLVE_INLINE except the inline loop is replaced
  by `mesh_vis = gShadowVis.Load(px,py).r` (one load), then the identical `vis = min(...)`.

À-trous filter (`shadow_atrous.comp.hlsl`, Dammertz 2010): 2D **25-tap/level** (5×5 B3,
compile-time-constant width), `levels` iterations, `step=1<<level` (push-const), edge-stop
`w = h · pow(max(0,dot(n_t,n_c)),SIGMA_N) · exp(-|z_t-z_c|/(SIGMA_Z·|o·step|+eps)) · valid_t`,
normalized `Σ(w·vis)/Σw`. NOT separable (non-linear edge-stop → axis streaks). z = linear
view depth from gViewT (same space as `csm_view_z`).

## Targets / framegraph

- `shadow_vis` **RG8_UNORM** (R=mesh_vis, G=validity), `shadow_vis2` ping-pong **RG16_UNORM**
  (avoids 3× cumulative 8-bit rounding), both full-res, ringed per-FIF (WAR fix).
- `FRAMEGRAPH_IMAGE_COUNT` 11 → **cfg-selected 13 on hwrt** (2 new images at ResId 11/12,
  declared LAST in the image block, before the first `add_buffer`; buffers re-base by the
  same const at the three `- FRAMEGRAPH_IMAGE_COUNT` sites → consistent). ALL denoise wiring
  `#[cfg(feature="hwrt")]`-walled (non-hwrt: const stays 11, byte-unchanged).
- Passes: VIS → à-trous×levels (ping-pong, RAW barriers RDG-derived per pass) → RESOLVE_DENOISED.
  `final_is_vis2 = levels%2==1` threaded from `levels`; `debug_assert resolve_read_resid ==
  last_atrous_write_resid`. Per-frame gate `scene.shadow = Some(..)` iff `mode==Spatial &&
  backend==HardwareTri && has_primary_directional && tlas_nonempty`; else `None` → RESOLVE_INLINE.

## Config (SSAO-mirrored, default None)

`ShadowDenoiseConfig { mode: None|Spatial, levels(3), sigma_z(1.0), sigma_n(128.0) }` +
`ResolvedShadowDenoise { sigma_z, sigma_n, _pad, _pad }` (16 B) + policy + `ShadowDenoisePlugin`.
Perf split: `ray_count` stays the RayShadowConfig spec-const (now specializes VIS/INLINE);
`levels` = host à-trous dispatch count (cold, retunable); `sigma_z/n` = UBO (live). HYBRID
lever: `ray_count=8 + levels=3` ≈ 16-ray inline at half the traversals.

## The 7 committable sub-steps (byte-identity gate at each)

1. **Config+plugin** (pure Rust, no render change). — **IN PROGRESS**
2. **HLSL SHADOW_STAGE scaffold, RESOLVE_INLINE only** (VIS/DENOISED empty stubs). GATE
   (RISKIEST): recompiled RESOLVE_INLINE `.spv` byte-hash == current (→ `af934c50`/`58f6c6c3`).
   Fallback if it drifts: keep RESOLVE_INLINE as the literal current body, VIS/DENOISED as
   sibling entry points.
3. **Targets + ResId re-base + sink** (hwrt), no passes. GATE: both goldens; non-hwrt const=11.
4. **VIS variant + VIS pass node + activation** (host still None). GATE: None → both goldens.
5. **À-trous shader + levels nodes + ping-pong + final-ResId parity** (host None). GATE: None goldens.
6. **RESOLVE_DENOISED variant + gShadowVis binding** (host None). GATE: RESOLVE_INLINE `.spv` unchanged.
7. **Host wiring + per-frame gate** — flip to Spatial. GATE: None→goldens; Spatial→owner-eval +
   grain metric (`residual(Spatial) < 0.4·residual(None)` where residual = L1(full − 8×down↕up)).

## Gates (orchestrator runs GPU; subagents can't — os-740)

- Byte: RESOLVE_INLINE `.spv` hash (step 2); `grand_showcase` golden `58f6c6c3` ±hwrt; RT-armed
  showcase default(None) == `af934c50`.
- **C3 algebraic anchor**: `levels=0` / pass-through filter Spatial run MUST reproduce `af934c50`
  bit-exactly (proves VIS mesh_vis == inline mesh_vis + identical min-combine).
- Spatial ON: owner-eval before/after (grainy → smooth, shape preserved, no ghosting) + the grain
  metric. dxc: quote `"-fspv-target-env=vulkan1.3"`; recompile only the hwrt variants; prove
  software `#else` invariant (temp-compile sha == frozen 65456 B).

## 3b TAA seam

`final_vis_res` (filtered vis) is exactly what 3b reprojects: 3b adds history ring + motion
vectors + reproject feeding RESOLVE_DENOISED's `gShadowVis` read. The 3-variant split + ringed
targets + validity channel are 3b-ready. NOT built here (no history/MV; engine no-TAA convention).
