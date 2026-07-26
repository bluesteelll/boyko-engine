//! `emit_probe_gi` — generates the SDFDDGI I2 committed probe-update compute shader
//! `sdf_probe_update.comp.hlsl` as ONE file whose `GI_MAX_IT` sphere-trace trip count is a
//! Vulkan SPECIALIZATION CONSTANT (id 0, default 64). The former 4 baked-const variant files
//! (`sdf_probe_update_it{32,64,96,128}.comp.hlsl`) collapse to this single source — the bench
//! sweep now overrides `GI_MAX_IT` per value via a `SpecConstant` at pipeline-create, so
//! measured==shipped from ONE `.spv` (refactor A-1, plan Part A §A1).
//!
//! The shader single-sources its eDSL spans from `boyko_shaderdsl`: the GENERATED `oct_decode`
//! function, the `probe_march` loop+tail span (inside the per-ray loop), and the `probe_blend`
//! / `probe_depth_blend` accumulate spans (inside the two per-texel gather loops). The
//! `sdf_soft_shadow_ranged` visibility function is COPIED VERBATIM from the committed
//! `deferred_pbr.hlsl` (pinned equal by the `sdf_soft_shadow_ranged_copy_matches_resolve` sync
//! test) — a shared `.hlsli` dedup is deferred to I3 (it would touch the frozen resolve →
//! 0%-gate risk, plan §1.1). The remaining structure (the STORAGE decls, the include contract,
//! the probe-index→world-position glue, the round-robin subset gate, the classification
//! read/write, the Fibonacci ray fetch, the per-ray direct-light shade loop, the groupshared
//! cooperative thread mapping, and — SDFDDGI I7 — the octahedral tile BORDER-COPY
//! (`border_copy_index` + the per-atlas border-fill loops run after each atlas's interior write,
//! behind a `DeviceMemoryBarrierWithGroupSync`) is hand-authored HLSL glue per plan §1.6 / §2. I7
//! is a texel-COPY index map, not marcher/field math, so it is glue here rather than a new eDSL
//! leaf — it does not touch the frozen `oct_decode`/`probe_march`/`probe_blend` spans.
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_probe_gi`
//!
//! Then DXC the single file with the frozen recipe (in the shader header):
//!   `dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//!        sdf_probe_update.comp.hlsl -Fo sdf_probe_update.comp.spv`
//! (cwd = the shaders dir, so the relative `#include "sdf_field.hlsli"` resolves). The
//! `emit_probe_gi` drift/sync tests pin the committed `.spv` to a fresh re-DXC of the
//! re-emitted `.hlsl`.

use std::path::PathBuf;

use boyko_shaderdsl::emit;

/// The `GI_MAX_IT` sweep values (plan §5) the bench overrides via a `SpecConstant` (id 0). The
/// shader is emitted ONCE (the trip count is a spec-const, default 64), so these values no longer
/// drive per-file emission — the bench binds them at pipeline-create to MEASURE each on the ONE
/// shipped `.spv`. Kept to document the sweep the emitter's spec-const default (64) anchors.
const GI_MAX_IT_VARIANTS: [u32; 4] = [32, 64, 96, 128];

fn main() {
    // The shaders dir is resolved relative to this crate's manifest (a sibling crate under the
    // same workspace). The bin is a developer tool, not shipped, so the cross-crate path is
    // acceptable (it mirrors the `emit_ssao_variants` precedent).
    let shaders = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("boyko_rhi_vulkan")
        .join("shaders");

    // The single-sourced eDSL spans (variant-INDEPENDENT — they spell the tuning consts
    // symbolically, so the SAME text appears in every variant).
    let oct_decode = emit::emit_hlsl_oct_decode();
    let probe_march = emit::emit_hlsl_probe_march();
    let probe_blend = emit::emit_hlsl_probe_blend();
    let probe_depth_blend = emit::emit_hlsl_probe_depth_blend();

    let shader = build_shader(&oct_decode, &probe_march, &probe_blend, &probe_depth_blend);
    let out = shaders.join("sdf_probe_update.comp.hlsl");
    std::fs::write(&out, &shader)
        .unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", out.display()));
    println!(
        "wrote {} ({} bytes) — GI_MAX_IT is spec-const id 0 (default 64); bench sweeps {GI_MAX_IT_VARIANTS:?}",
        out.display(),
        shader.len()
    );
}

/// Assembles the single committed `sdf_probe_update.comp.hlsl`. `GI_MAX_IT` is a Vulkan
/// specialization constant (id 0, default 64) — resolved at pipeline-create, NOT baked — so ONE
/// source serves every sweep value. The eDSL spans + all hand-written glue are carried verbatim.
fn build_shader(
    oct_decode: &str,
    probe_march: &str,
    probe_blend: &str,
    probe_depth_blend: &str,
) -> String {
    format!(
        r#"// SDFDDGI I2 — the probe-update compute pass (`sdf_probe_update.comp.hlsl`).
//
// Sphere-traces the CSG edit-list from each ACTIVE probe over a Fibonacci ray set, shades each
// hit (direct light + `sdf_soft_shadow_ranged` visibility), and blends the results into the
// probe's octahedral irradiance tile (+ two-moment depth tile). Subset-limited by round-robin
// from frame one (plan §4). SDFDDGI I4: temporal hysteresis (an EMA-blend against the persistent
// atlas) + a per-frame smoothly-advancing ray rotation converge the field over frames (no more
// strobe). All work stays behind the GI-OFF 0%-gate: recorded ONLY when `ResolvedDdgi::enabled()`.
//
// # Single-source (eDSL — `boyko_shaderdsl`)
//
// The `// === GENERATED oct_decode / probe_march / probe_blend / probe_depth_blend BEGIN/END
// ===` spans are MACHINE-GENERATED by `boyko_shaderdsl::emit` (this file is produced by the
// `emit_probe_gi` bin). A hand-edit of a generated span fails the `emit_probe_gi` drift
// gate. The `sdf_soft_shadow_ranged` visibility function is COPIED VERBATIM from
// `deferred_pbr.hlsl` (pinned equal by `sdf_soft_shadow_ranged_copy_matches_resolve`); a shared
// `.hlsli` dedup is deferred to I3.
//
// # The I2 → I3 oct-decode contract (load-bearing)
//
// The tile-UV↔texel REMAP + probe-spacing reconstruction I3 owns MUST live in the texel→UV
// chain OUTSIDE `oct_decode` (in the hand-written `texel_dir_irr`/`texel_dir_depth` glue),
// never inside `oct_decode` — else this pass's WRITE iteration and I3's READ desync.
//
// # Compiled offline (hermetic) with (ONE file; GI_MAX_IT is spec-const id 0, default 64):
//   dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       sdf_probe_update.comp.hlsl -Fo sdf_probe_update.comp.spv

// --- Resources (dedicated update bind-group, set 0 — plan §2.2) ----------------------------
//   t0 : StructuredBuffer<uint>   Buf            — the SDF edit-list (`sdf_field.hlsli` contract)
//   u1 : RWTexture2DArray<float4> gIrrOut        — the irradiance atlas (B10G11R11, STORAGE write)
//   u2 : RWTexture2DArray<float2> gDepthOut      — the two-moment depth atlas (RG16F, STORAGE write)
//   t3 : RWStructuredBuffer<uint> Classification — 1 u32/probe (bit0 active, bit1 converged-once)
//   t4 : StructuredBuffer<float4> RayTable       — the Fibonacci ray directions (boot-static)
//   t5 : StructuredBuffer<uint>   LightBuf       — the L0 light table
//   b6 : cbuffer DdgiUpdate                      — the grid/subset/ray params
//
// The `[[vk::image_format]]` pin fixes each storage image's OpTypeImage: `r11g11b10f` →
// SPIR-V `R11fG11fB10f` (the exact format for VK_FORMAT_B10G11R11_UFLOAT_PACK32),
// `rg16f` → `Rg16f` (VK_FORMAT_R16G16_SFLOAT). A wrong decoration string is silent write
// corruption (plan §10.3 / P2-1). `shaderStorageImageWriteWithoutFormat` is OFF.
StructuredBuffer<uint> Buf : register(t0);
[[vk::image_format("r11g11b10f")]] RWTexture2DArray<float4> gIrrOut   : register(u1);
[[vk::image_format("rg16f")]]      RWTexture2DArray<float2> gDepthOut : register(u2);
// `Classification` is READ+WRITE, so it binds a `u` UAV register (HLSL requires `u` for a
// `RWStructuredBuffer`); the Vulkan binding number is still 3 (the `u3`/`t3` namespaces are
// separate in HLSL, unified in the SPIR-V binding). RayTable/LightBuf are READ-ONLY `t` SRVs.
RWStructuredBuffer<uint>  Classification : register(u3);
StructuredBuffer<float4>  RayTable       : register(t4);
StructuredBuffer<uint>    LightBuf       : register(t5);

// The `sdf_field.hlsli` INCLUDE CONTRACT requires `StructuredBuffer<uint> Buf : register(t0)`
// in scope BEFORE the include (the field eval reads `Buf[0]` = edit_count, then the packed
// edits). It provides `field_distance(p)`, `sdf(p)`, `sdf_normal(p)` (the central-difference
// gradient — the SAME eDSL-generated `sdf_normal` the resolve uses, pinned by
// `sdf_field_edsl_sync`), `MAX_SDF_EDITS`, `GRAD_H`, `FIELD_LIPSCHITZ_L`, etc.
#include "sdf_field.hlsli"

// The shared light-table decode (`light_table.hlsli`) — a READ-ONLY shared header (like
// `sdf_field.hlsli`), guarded by `#ifndef LIGHT_TABLE_HLSLI`, indexing the SAME
// `StructuredBuffer<uint> LightBuf` already in scope above. It does NOT touch the frozen
// resolve. Provides `load_light(LightBuf, i)` (the 16-word header + 12-word GpuLight[] layout,
// `dir@+0..2 / kind@+3 / pos@+4..6 / range@+7 / color@+8..10 / cone@+11`), `light_kind()`, and
// the `LIGHT_KIND_*` enum — the ONE source of truth for the GPU light table (P0-1 fix).
#include "light_table.hlsli"

// --- The shadow-march tuning (mirror `deferred_pbr.hlsl` — the copied `sdf_soft_shadow_ranged`
// reads these; `FIELD_LIPSCHITZ_L`/`GRAD_H`/`FAR` come from `sdf_field.hlsli`). Identical values
// to the resolve so the copied marcher behaves byte-identically. `EPS` is the field hit epsilon
// (also the base of `GI_HIT_EPS` / `SHADOW_HIT_EPS`), declared here before its consumers.
static const float EPS                = 0.001;
static const uint  MAX_IT             = 128u;
static const float SHADOW_K           = 8.0;
static const float SHADOW_MINT        = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP   = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS     = 2.0 * EPS;
// The normal-offset march-origin lift (anti grazing-acne) — the SAME value the resolve uses
// (`deferred_pbr.hlsl` SHADOW_NORMAL_BIAS = 0.02), so the GI shadow origin agrees with it (P1-1).
static const float SHADOW_NORMAL_BIAS = 0.02;

// binding b6: the per-frame update parameters (grid origin/spacing/dims + subset + ray count +
// light count). `grid_dims.xyz` = the grid dims (probes per axis) as uint; `origin.xyz` = the
// grid world origin, `origin.w` = the probe spacing. Phase 2's Rust `DdgiUpdateUbo` matches this
// field name (P2-1 — `grid_dims`, NOT the misleading `inv_spacing_dims`, which held dims, not an
// inverse spacing).
cbuffer DdgiUpdate : register(b6) {{
    float4 origin;             // xyz = grid world origin, w = probe spacing
    uint4  grid_dims;          // xyz = grid dims (probes per axis), w = asfloat(hysteresis alpha)
    uint   frame_index;        // host-frame-derived (the round-robin phase)
    uint   subset_n;           // round-robin divisor N (divides DDGI_PROBE_COUNT)
    uint   rays_per_probe;     // Fibonacci ray count (== RayTable length)
    uint   light_count;        // light-table entry count
}};

// --- The atlas geometry (mirror boyko_rhi_vulkan::ddgi; host-pinned) -----------------------
// Y-plane-major: array layer = probe Y; within a layer, tile column = X, tile row = Z. The
// irradiance tile is 8x8 (6x6 valid + 1-texel border); the depth tile 16x16 (14x14 + border).
// GI_MAX_IT is a Vulkan SPECIALIZATION CONSTANT (id 0): the sphere-trace `[loop]` trip count,
// resolved at pipeline-create. Its DEFAULT (64) makes a pipeline built with `spec_constants: &[]`
// byte-identical to the former baked `static const 64u`; the bench overrides it per sweep value via
// a `SpecConstant` (id 0). A spec-const on a `[loop]` bound is structurally identical to a baked
// const (the loop is never unrolled either way) — same dynamic loop, ZERO per-thread cost.
[[vk::constant_id(0)]] const uint GI_MAX_IT = 64;
// The `probe_march` tuning (the generated span spells these SYMBOLICALLY — plan §1.2). The
// SHADOW_MINT-class start bias, the occluder-hit epsilon, the min per-step advance, and the
// escape bound. GRAD_H/EPS come from `sdf_field.hlsli` / the shadow tuning block above.
static const float GI_MINT      = 16.0 * GRAD_H;
static const float GI_HIT_EPS   = 2.0 * EPS;
static const float GI_MINT_STEP = 16.0 * GRAD_H;
static const float GI_T_MAX     = 10.0;
static const uint  DDGI_IRR_TILE_EDGE   = 8u;
static const uint  DDGI_DEPTH_TILE_EDGE  = 16u;
static const uint  DDGI_IRR_VALID_EXTENT  = 6u;   // the valid octahedral interior extent (irr)
static const uint  DDGI_DEPTH_VALID_EXTENT = 14u; // the valid interior extent (depth)
static const uint  DDGI_TILE_BORDER      = 1u;    // the 1-texel border inset
static const float DDGI_MIN_SUM_WEIGHT   = 1.0e-6; // the resolve-side cosine-sum divide guard

// SDFDDGI I7: the octahedral tile border-RING texel counts (the 1-texel ring around the valid
// interior — `tile_edge^2 - valid_extent^2 == 4*(tile_edge-1)`), the loop bound the border-copy
// pass below iterates over. Symbolic, mirroring the valid-extent/tile-edge pair per atlas.
static const uint  DDGI_IRR_BORDER_COUNT   = 4u * (DDGI_IRR_TILE_EDGE - 1u);   // 28
static const uint  DDGI_DEPTH_BORDER_COUNT = 4u * (DDGI_DEPTH_TILE_EDGE - 1u); // 60

// The classification bits (plan §4).
static const uint DDGI_CLASS_ACTIVE    = 1u;      // bit0: probe not inside geometry
static const uint DDGI_CLASS_CONVERGED = 2u;      // bit1: first successful tile write done
static const float GI_INSIDE_EPS       = 0.0;     // `field_distance(probe) < eps` ⇒ inside ⇒ inactive

// --- SDFDDGI I4 temporal-accumulation + quality tuning (update-side; the RESOLVE is untouched) ---
// Per-frame ray rotation (smoothly-advancing, deterministic — see `rotate_ray`). A SMALL per-frame
// orientation delta keeps the alpha-hysteresis EMA stable while the set sweeps the sphere over
// frames (a per-frame RANDOM reorientation would inject variance a 0.9x filter cannot settle —
// RTXGI tolerates random only via an adaptive-hysteresis relief we do not have).
static const float GI_ROT_SPIN    = 0.2393; // primary spin per frame (rad), constant angular vel
static const float GI_ROT_PRECESS = 0.0409; // slow axis precession per frame (rad), incommensurate
static const float GI_ROT_TILT    = 0.9553; // precession cone half-angle (~54.7 deg, even coverage)
// Firefly clamp on ONE ray's shaded radiance (anti a lone bright hit freezing into the EMA for
// ~1/(1-alpha) frames). Generous — a sunlit Lambert surface is O(1), so 16 never clips real signal.
static const float DDGI_MAX_RADIANCE = 16.0;
// Depth two-moment distance clamp (x spacing): a sky-miss ray writes GI_T_MAX (10); left raw it
// blows up E[d^2] -> a huge Chebyshev variance -> light leak. 1.5*spacing (the RTXGI rule) keeps the
// moments inside the resolve's probe-neighbourhood query range.
static const float GI_DEPTH_CLAMP_SCALE = 1.5;
// A single constant standing in for the diffuse bounce reflectance the update shade omits (the
// update set binds NO material table, so `shade_hit` returns the hit's incident light, not
// rho/pi * E). 1.0 = the I3 behaviour (white bounce, no per-hit albedo tint — colored bleeding is a
// follow-up needing the material table in the update set); tuned from the owner-eval picture.
static const float GI_BOUNCE_SCALE = 1.0;

// The groupshared cooperative ray cache (plan §2.4): one thread-block per active probe; the 64
// threads cooperatively march the rays into LDS, sync, then cooperatively gather the texels.
// R = rays_per_probe <= 128; 128 * (dir3 + L3 + t1) = 128 * 7 floats = 3.5 KB, well within 32 KB.
static const uint GI_MAX_RAYS = 128u;
groupshared float3 gs_dir[GI_MAX_RAYS]; // the cached ray direction
groupshared float3 gs_L[GI_MAX_RAYS];   // the cached shaded radiance
groupshared float  gs_t[GI_MAX_RAYS];   // the cached marched hit distance

{soft_shadow}

// === GENERATED oct_decode BEGIN ===
{oct_decode}// === GENERATED oct_decode END ===

// The GI-ray sphere-trace, wrapping the eDSL-generated `probe_march` span (which spells a
// function-`return true/false` + writes the `out float hit_t`). Returns the occluder-hit flag;
// `hit_t` = the marched hit distance (or GI_T_MAX on a sky miss). The span reads `ro`/`rd`.
bool probe_march(float3 ro, float3 rd, out float hit_t) {{
    // === GENERATED probe_march BEGIN ===
{probe_march}    // === GENERATED probe_march END ===
}}

// --- Probe index <-> grid coordinate + world position (Y-plane-major) ----------------------
uint3 probe_coord(uint probe_index) {{
    uint dx = grid_dims.x;
    uint dy = grid_dims.y;
    uint x = probe_index % dx;
    uint y = (probe_index / dx) % dy;
    uint z = probe_index / (dx * dy);
    return uint3(x, y, z);
}}

float3 probe_world_pos(uint3 c) {{
    // origin + coord * spacing (the world-fixed D1 grid — probe i is the same world point
    // every frame).
    return origin.xyz + float3(c) * origin.w;
}}

// The atlas tile texel origin (Y-plane-major): array layer = Y; ox = X * tile_edge; oy = Z *
// tile_edge (mirrors `boyko_rhi_vulkan::ddgi::ddgi_probe_tile_origin`).
uint3 tile_origin(uint3 c, uint tile_edge) {{
    return uint3(c.y, c.x * tile_edge, c.z * tile_edge);
}}

// SDFDDGI I7 — the octahedral tile BORDER-COPY index map. Maps a border-ring texel index `bt`
// (in [0, 4*(tile_edge-1))) to its LOCAL destination texel `dst` (in the full [0,tile_edge)^2
// tile) and its LOCAL source INTERIOR texel `src` (in [0,valid_extent)^2) to copy from. The
// interior occupies local [DDGI_TILE_BORDER, DDGI_TILE_BORDER + valid_extent - 1]^2; the border
// ring is the outermost 1-texel edge. Octahedral wrap-with-flip (the RTXGI/Majercik DDGI
// convention): a top/bottom edge texel copies the OPPOSITE interior row, column REVERSED; a
// left/right edge texel copies the OPPOSITE interior column, row REVERSED; the 4 corner texels
// copy the DIAGONALLY-OPPOSITE interior corner. This makes a LINEAR SampleLevel tap straddling a
// tile edge read the octahedrally-continuous neighbor instead of the boot-clear 0 (closing the
// I3 resolve's seam-bleed / depth-leak at `ddgi_irr_uv`/`ddgi_depth_uv`'s oct-UV extremes).
void border_copy_index(uint bt, uint tile_edge, uint valid_extent, out uint2 dst, out uint2 src) {{
    uint v = valid_extent;
    if (bt < tile_edge) {{
        // Top row (local y = 0); bt is the local x (column).
        uint bx = bt;
        dst = uint2(bx, 0u);
        if (bx == 0u) {{ src = uint2(v - 1u, v - 1u); return; }}          // top-left <- bottom-right interior
        if (bx == tile_edge - 1u) {{ src = uint2(0u, v - 1u); return; }}  // top-right <- bottom-left interior
        uint cx = bx - DDGI_TILE_BORDER;
        src = uint2(v - 1u - cx, v - 1u);                                 // bottom interior row, column reversed
        return;
    }}
    bt -= tile_edge;
    if (bt < tile_edge) {{
        // Bottom row (local y = tile_edge - 1); bt is the local x (column).
        uint bx = bt;
        dst = uint2(bx, tile_edge - 1u);
        if (bx == 0u) {{ src = uint2(v - 1u, 0u); return; }}              // bottom-left <- top-right interior
        if (bx == tile_edge - 1u) {{ src = uint2(0u, 0u); return; }}      // bottom-right <- top-left interior
        uint cx = bx - DDGI_TILE_BORDER;
        src = uint2(v - 1u - cx, 0u);                                    // top interior row, column reversed
        return;
    }}
    bt -= tile_edge;
    if (bt < v) {{
        // Left col (local x = 0), corners excluded; bt is the local y offset within the interior span.
        dst = uint2(0u, bt + DDGI_TILE_BORDER);
        src = uint2(v - 1u, v - 1u - bt);                                // right interior col, row reversed
        return;
    }}
    bt -= v;
    // Right col (local x = tile_edge - 1), corners excluded; bt is the local y offset.
    dst = uint2(tile_edge - 1u, bt + DDGI_TILE_BORDER);
    src = uint2(0u, v - 1u - bt);                                        // left interior col, row reversed
}}

// The valid-interior texel (tx, ty) -> the [0,1]^2 tile UV oct_decode remaps to [-1,1]^2. The
// texel CENTER maps through `(tx + 0.5) / VALID_EXTENT` (mirrors the I0b host oracle
// `goldens::ddgi_texel_dir`). This tile-UV chain is I2's business; the I3 remap lives OUTSIDE
// oct_decode (the load-bearing contract).
float2 texel_uv(uint tx, uint ty, uint valid_extent) {{
    float extent = (float)valid_extent;
    return float2(((float)tx + 0.5) / extent, ((float)ty + 0.5) / extent);
}}

// --- The direct-light shade of one ray hit (plan §1.6 per-ray glue) -------------------------
// Loops the light table, accumulating each DIRECTIONAL light's diffuse Lambert term gated by the
// SDF visibility `sdf_soft_shadow_ranged`. A minimal single-bounce DIFFUSE direct shade
// (multi-bounce probe feedback is I5; specular is a later cone-trace). `hit_pos` is the marched
// surface point, `n` the TRUE SDF field normal at the hit (`sdf_normal(hit_pos)`, P1-2).
//
// Layout / double-count fix (P0-1): the light table is the shared `light_table.hlsli` layout —
// 16-word header, 12-word `GpuLight[]` entries; `e.dir` is the TO-LIGHT direction (normalize it),
// and `e.color` is `linear_color × illuminance` ALREADY BAKED (`from_directional`), so the shade
// multiplies by `e.color` ONLY — NO separate illuminance word (the resolve does the same:
// `lit_direct += (diff + spec) * (NoL * vis) * L.color`).
float3 shade_hit(float3 hit_pos, float3 n) {{
    float3 lit = float3(0.0, 0.0, 0.0);
    for (uint li = 0u; li < light_count; ++li) {{
        LightElem e = load_light(LightBuf, li);
        // I2 handles DIRECTIONAL lights (the showcase's main lights). Point/spot need per-kind
        // attenuation/cone handling the resolve does in its L0b block; skip them here rather than
        // silently mis-shading them as directional.
        // TODO(I-later): fold POINT/SPOT into the GI shade (attenuation + cone), per resolve L0b.
        if (light_kind(e) != LIGHT_KIND_DIRECTIONAL) {{ continue; }}
        // `e.dir` is the TO-LIGHT world direction (the resolve `normalize(L.dir)`); the diffuse
        // Lambert cosine against the true field normal.
        float3 l = normalize(e.dir);
        float NoL = max(dot(n, l), 0.0);
        if (NoL <= 0.0) {{ continue; }}
        // SDF visibility toward the light, ranged to GI_T_MAX (a directional reaches everywhere),
        // from a NORMAL-BIASED origin so grazing rays clear the surface (anti self-occlusion,
        // P1-1). Mirrors the resolve `sdf_soft_shadow_ranged(P + n*SHADOW_NORMAL_BIAS, n, l, ...)`.
        float vis = sdf_soft_shadow_ranged(hit_pos + n * SHADOW_NORMAL_BIAS, n, l, GI_T_MAX);
        // Diffuse only: `e.color` already carries `color × illuminance` (no double-count).
        lit += e.color * (NoL * vis);
    }}
    // SDFDDGI I4: the single-constant bounce-reflectance stand-in (GI_BOUNCE_SCALE; 1.0 = white
    // bounce — the update set binds no material table, so no per-hit albedo tint yet).
    return lit * GI_BOUNCE_SCALE;
}}

// SDFDDGI I4 — the per-frame ray-set rotation. A smoothly-advancing DETERMINISTIC rotation (NOT a
// per-frame random reorientation): a primary spin at constant angular velocity about an axis that
// slowly precesses on a cone, the two rates incommensurate. Successive frames' 64-ray sets stay
// nearly aligned (small delta -> the hysteresis EMA settles) yet fill each other's angular gaps and
// sweep the sphere over frames. Transcendentals are fine here — this is the UPDATE pass, not the
// bit-exact RESOLVE. `frame` is the raw monotonic frame index (never the subset phase).
float3 rotate_ray(float3 v, uint frame) {{
    float f = (float)frame;
    float spin = f * GI_ROT_SPIN;
    float prec = f * GI_ROT_PRECESS;
    // The precessing unit axis: a cone about +Y at half-angle GI_ROT_TILT.
    float3 axis = float3(sin(GI_ROT_TILT) * cos(prec), cos(GI_ROT_TILT), sin(GI_ROT_TILT) * sin(prec));
    // Rotate v by the unit quaternion (axis*sin(spin/2), cos(spin/2)):
    //   v' = v + 2 s (q x v) + 2 q x (q x v),  q = axis*sin(h), s = cos(h), h = spin/2.
    float h = 0.5 * spin;
    float s = cos(h);
    float3 q = axis * sin(h);
    float3 t = 2.0 * cross(q, v);
    return v + s * t + cross(q, t);
}}

[numthreads(64, 1, 1)]
void main(uint3 gid : SV_GroupID, uint3 lid : SV_GroupThreadID) {{
    // One thread-block per active-subset probe. Map block -> probe index in the current subset:
    // probe_index = block * subset_n + (frame_index % subset_n) (plan §4 round-robin). The
    // dispatch is sized to `DDGI_PROBE_COUNT / subset_n` blocks.
    uint phase = frame_index % subset_n;
    uint probe_index = gid.x * subset_n + phase;

    uint3 c = probe_coord(probe_index);
    float3 pw = probe_world_pos(c);
    uint R = min(rays_per_probe, GI_MAX_RAYS);

    // SDFDDGI I4: the temporal-blend state, read ONCE before any tile write. `blend_a` is the
    // hysteresis alpha (from grid_dims.w) ONLY when this probe was ACTIVE *and* CONVERGED last frame,
    // else 0 (a fresh write). The reset key is `ACTIVE & CONVERGED`, NOT `CONVERGED` alone: a probe
    // that was buried last frame keeps its CONVERGED bit but its atlas tile is stale (geometry moved
    // through it), and the resolve gates on the depth-mean sentinel — NOT this bit — so a stale tile
    // would leak (the re-activation ghost). Keying the reset on ACTIVE&CONVERGED forces a fresh write
    // on re-activation. Every texel thread reads the PRE-frame class (the CONVERGED bit is re-stamped
    // only at the end by lid.x==0).
    uint cls = Classification[probe_index];
    bool was_ac = (cls & (DDGI_CLASS_ACTIVE | DDGI_CLASS_CONVERGED))
                  == (DDGI_CLASS_ACTIVE | DDGI_CLASS_CONVERGED);
    float blend_a = was_ac ? asfloat(grid_dims.w) : 0.0;

    // Classification bit0 (active): re-evaluated each scheduled frame (geometry is dynamic).
    // `inside = field_distance(probe) < GI_INSIDE_EPS` ⇒ the probe is buried ⇒ skip it.
    bool inside = field_distance(pw) < GI_INSIDE_EPS;
    if (inside) {{
        if (lid.x == 0u) {{
            // Clear the active bit (keep the converged bit) so the resolve treats a
            // newly-buried probe as inactive.
            Classification[probe_index] = cls & DDGI_CLASS_CONVERGED;
        }}
        return;
    }}

    // (1) Cooperatively march the rays into groupshared (thread i marches rays i, i+64, ...).
    for (uint r = lid.x; r < R; r += 64u) {{
        // SDFDDGI I4: rotate the boot-static Fibonacci direction by this frame's smooth rotation so
        // the hysteresis EMA integrates a fuller sphere over frames. The ROTATED `rd` is what gets
        // cached in `gs_dir[r]`, so the octahedral blend weights against the same direction marched.
        float3 rd = rotate_ray(normalize(RayTable[r].xyz), frame_index);
        float hit_t;
        bool hit = probe_march(pw, rd, hit_t);
        // Shade the hit; a sky miss contributes zero radiance + GI_T_MAX depth.
        float3 L = float3(0.0, 0.0, 0.0);
        if (hit) {{
            float3 hit_pos = pw + rd * hit_t;
            // The TRUE SDF field normal at the hit (`sdf_normal` from `sdf_field.hlsli` — the SAME
            // eDSL-generated central-difference gradient the resolve uses, pinned by
            // `sdf_field_edsl_sync`; NOT the coarse `-rd`, P1-2). Its ~6 field taps/hit are a
            // real part of the `ddgi_probe_gi_cost` bench (cost-honesty).
            float3 n = sdf_normal(hit_pos);
            L = shade_hit(hit_pos, n);
            // SDFDDGI I4 firefly clamp: a lone bright hit would otherwise freeze into the EMA.
            L = min(L, float3(DDGI_MAX_RADIANCE, DDGI_MAX_RADIANCE, DDGI_MAX_RADIANCE));
        }} else {{
            hit_t = GI_T_MAX;
        }}
        gs_dir[r] = rd;
        gs_L[r] = L;
        gs_t[r] = hit_t;
    }}
    GroupMemoryBarrierWithGroupSync();

    // (2) Cooperatively gather the irradiance texels (6x6 valid) across the 64 threads. Each
    // thread owns texels `t = lid.x, lid.x+64, ...` over the DDGI_IRR_VALID_EXTENT^2 grid.
    uint3 irr_org = tile_origin(c, DDGI_IRR_TILE_EDGE);
    uint irr_valid = DDGI_IRR_VALID_EXTENT * DDGI_IRR_VALID_EXTENT;
    for (uint it = lid.x; it < irr_valid; it += 64u) {{
        uint tx = it % DDGI_IRR_VALID_EXTENT;
        uint ty = it / DDGI_IRR_VALID_EXTENT;
        float2 uv = texel_uv(tx, ty, DDGI_IRR_VALID_EXTENT);
        // texelUV -> texelDir. The tile-UV -> [-1,1]^2 remap lives HERE (outside oct_decode).
        float3 texelDir = oct_decode(uv);
        float sum_r = 0.0, sum_g = 0.0, sum_b = 0.0, sum_w = 0.0;
        for (uint r = 0u; r < R; ++r) {{
            float3 rayDir = gs_dir[r];
            float l_r = gs_L[r].x, l_g = gs_L[r].y, l_b = gs_L[r].z;
            // === GENERATED probe_blend BEGIN ===
{probe_blend}            // === GENERATED probe_blend END ===
        }}
        float3 irr = float3(sum_r, sum_g, sum_b) / max(sum_w, DDGI_MIN_SUM_WEIGHT);
        // Write the valid interior texel (offset past the 1-texel border).
        uint3 dst = uint3(irr_org.y + DDGI_TILE_BORDER + tx, irr_org.z + DDGI_TILE_BORDER + ty, irr_org.x);
        // SDFDDGI I4 hysteresis: EMA-blend the fresh irradiance against the persistent atlas texel
        // (a read-modify-write of the UAV — this dispatch has not yet written `dst`, so the read is
        // last frame's value; each interior texel is owned by exactly one thread, no aliasing).
        // `blend_a == 0` on the first-converged / re-activated write ⇒ a fresh (un-blended) write.
        float3 prev_irr = gIrrOut[dst].rgb;
        gIrrOut[dst] = float4(lerp(irr, prev_irr, blend_a), 1.0);
    }}

    // SDFDDGI I7: publish this dispatch's irradiance interior writes to the whole group before the
    // border-copy reads them — a border-copy thread may read an interior texel a DIFFERENT thread
    // just wrote in the loop above (a cross-thread UAV read-after-write within this threadgroup).
    DeviceMemoryBarrierWithGroupSync();

    // (2b) Cooperatively fill the irradiance tile's 1-texel border (SDFDDGI I7 — the octahedral
    // wrap-with-flip copy, `border_copy_index`). Without this the border stays at the boot-clear 0,
    // and the I3 resolve's LINEAR `SampleLevel` (`ddgi_irr_uv`) lands exactly on the border/interior
    // boundary at the oct-UV extremes (e == 0 or e == 1), blending 50% with that 0 — a real
    // darkened seam at every probe tile edge.
    for (uint ib = lid.x; ib < DDGI_IRR_BORDER_COUNT; ib += 64u) {{
        uint2 dst2, src2;
        border_copy_index(ib, DDGI_IRR_TILE_EDGE, DDGI_IRR_VALID_EXTENT, dst2, src2);
        uint3 dst_texel = uint3(irr_org.y + dst2.x, irr_org.z + dst2.y, irr_org.x);
        uint3 src_texel = uint3(irr_org.y + DDGI_TILE_BORDER + src2.x, irr_org.z + DDGI_TILE_BORDER + src2.y, irr_org.x);
        gIrrOut[dst_texel] = gIrrOut[src_texel];
    }}

    // (3) Cooperatively gather the depth texels (14x14 valid) — the two-moment tile.
    uint3 depth_org = tile_origin(c, DDGI_DEPTH_TILE_EDGE);
    uint depth_valid = DDGI_DEPTH_VALID_EXTENT * DDGI_DEPTH_VALID_EXTENT;
    for (uint dt = lid.x; dt < depth_valid; dt += 64u) {{
        uint tx = dt % DDGI_DEPTH_VALID_EXTENT;
        uint ty = dt / DDGI_DEPTH_VALID_EXTENT;
        float2 uv = texel_uv(tx, ty, DDGI_DEPTH_VALID_EXTENT);
        float3 texelDir = oct_decode(uv);
        float dmean = 0.0, dmean2 = 0.0, dw = 0.0;
        for (uint r = 0u; r < R; ++r) {{
            float3 rayDir = gs_dir[r];
            // SDFDDGI I4: clamp the hit distance to 1.5*spacing before the two-moment accumulate —
            // a sky-miss ray writes GI_T_MAX, which left raw blows up E[d^2] -> a huge Chebyshev
            // variance -> light leak. The clamp keeps the moments in the resolve's probe-query range.
            float t = min(gs_t[r], origin.w * GI_DEPTH_CLAMP_SCALE);
            // === GENERATED probe_depth_blend BEGIN ===
{probe_depth_blend}            // === GENERATED probe_depth_blend END ===
        }}
        float inv = 1.0 / max(dw, DDGI_MIN_SUM_WEIGHT);
        float2 moments = float2(dmean * inv, dmean2 * inv);
        uint3 dst = uint3(depth_org.y + DDGI_TILE_BORDER + tx, depth_org.z + DDGI_TILE_BORDER + ty, depth_org.x);
        // SDFDDGI I4 hysteresis: EMA-blend the two moments (a linear EMA of E[d] and E[d^2] converges
        // to the true moments, so the Chebyshev variance E[d^2]-E[d]^2 self-heals). Same `blend_a`.
        float2 prev_m = gDepthOut[dst];
        gDepthOut[dst] = lerp(moments, prev_m, blend_a);
    }}

    // SDFDDGI I7: publish this dispatch's depth interior writes before the border-copy reads them
    // (the same cross-thread UAV read-after-write hazard as the irradiance tile above).
    DeviceMemoryBarrierWithGroupSync();

    // (3b) Cooperatively fill the depth tile's 1-texel border (SDFDDGI I7 — the same octahedral
    // wrap-with-flip copy, parameterized by the depth tile geometry). Closes the SAME seam-bleed
    // hazard for the Chebyshev two-moment tap (`ddgi_depth_uv`) — an uncopied border would blend a
    // valid moment with the boot-clear (0,0), skewing `var`/`mean` at tile edges (a depth-leak, not
    // just a color darkening).
    for (uint db = lid.x; db < DDGI_DEPTH_BORDER_COUNT; db += 64u) {{
        uint2 dst2, src2;
        border_copy_index(db, DDGI_DEPTH_TILE_EDGE, DDGI_DEPTH_VALID_EXTENT, dst2, src2);
        uint3 dst_texel = uint3(depth_org.y + dst2.x, depth_org.z + dst2.y, depth_org.x);
        uint3 src_texel = uint3(depth_org.y + DDGI_TILE_BORDER + src2.x, depth_org.z + DDGI_TILE_BORDER + src2.y, depth_org.x);
        gDepthOut[dst_texel] = gDepthOut[src_texel];
    }}

    // (4) One thread stamps the converged-once bit (+ keeps active set): the tile is written.
    if (lid.x == 0u) {{
        Classification[probe_index] = DDGI_CLASS_ACTIVE | DDGI_CLASS_CONVERGED;
    }}
}}
"#,
        soft_shadow = SDF_SOFT_SHADOW_RANGED_COPY,
        oct_decode = oct_decode,
        probe_march = probe_march,
        probe_blend = probe_blend,
        probe_depth_blend = probe_depth_blend,
    )
}

/// The `sdf_soft_shadow_ranged` function COPIED VERBATIM from the committed
/// `crates/boyko_rhi_vulkan/shaders/sdf_shadow_leaves.hlsli` — pinned token-equal to it by the
/// `sdf_soft_shadow_ranged_copy_matches_resolve` sync test, which is where to re-copy from when
/// this constant has to be refreshed. The tuning symbols
/// (`SHADOW_MINT`/`MAX_IT`/`SHADOW_K`/`SHADOW_HIT_EPS`/`FIELD_LIPSCHITZ_L`/`SHADOW_MINT_STEP`)
/// are provided by `sdf_field.hlsli` + the shadow header this pass shares with the resolve.
///
/// It used to be copied from `deferred_pbr.hlsl`, which held the only hand-placed definition until
/// VB-SV0 rung S2 moved it into the shared leaf header (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §4.1) so
/// the three VB lit-producer tails could consume one definition instead of hand-copying it. The
/// copy's MEANING is unchanged — the probe-update march must equal the resolve's — and that is
/// independent of which file the resolve's copy is spelled in.
///
/// ⚠️ The EMITTED comment inside the string below still says "copied VERBATIM from
/// `deferred_pbr.hlsl`", and that is DELIBERATE, not an oversight — do NOT "fix" it. §4.1 scoped
/// S2's blast radius to exactly ten re-pinned `.spv`. Editing the emitted text forces a re-emit
/// and re-commit of the generated `sdf_probe_update.comp.hlsl`, and this repo's standing rule is
/// that a regenerated shader is re-DXC'd and its `.spv` re-pinned — pulling
/// `sdf_probe_update.comp.spv` into the rung for a comment. The pointer that has to be correct is
/// THIS doc, because this is what a future regeneration reads.
const SDF_SOFT_SHADOW_RANGED_COPY: &str = "\
// The multi-light SDF shadow marcher — copied VERBATIM from `deferred_pbr.hlsl` (pinned equal by\n\
// `sdf_soft_shadow_ranged_copy_matches_resolve`). `t_max` = the light reach.\n\
float sdf_soft_shadow_ranged(float3 p, float3 n, float3 L, float t_max) {\n\
    float res = 1.0;\n\
    float t = SHADOW_MINT;\n\
    [loop]\n\
    for (uint i = 0u; i < MAX_IT; ++i) {\n\
        float d = field_distance(p + L * t);\n\
        res = min(res, SHADOW_K * d / t);\n\
        if (d < SHADOW_HIT_EPS) {\n\
            return 0.0;\n\
        }\n\
        t = t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP);\n\
        if (t > t_max) {\n\
            break;\n\
        }\n\
    }\n\
    return clamp(res, 0.0, 1.0);\n\
}\n";
