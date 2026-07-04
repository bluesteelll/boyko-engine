// SDFDDGI I3 — the DDGI resolve-side probe-irradiance SAMPLE (shared include).
//
// ONE source of truth for the deferred PBR resolve (`deferred_pbr.hlsl`) AND the GPU golden
// (`ddgi_probe_gi_resolve.comp.hlsl`) so both author BYTE-IDENTICAL math. This is the op-for-op
// HLSL mirror of the host oracle `boyko_rhi_vulkan::goldens::probe_sample` (+ `oct_encode` /
// `ddgi_texel_direction` inverse). The resolve path is R11G11B10F-no-gamma (Decision D6) so it
// stays transcendental-free — accepted op set ONLY: {+ - * / abs min max clamp/saturate floor
// sqrt normalize select}. NO `pow`. The op ORDER + associativity match the host oracle exactly
// (float add is non-associative) — the I3 GPU golden `probe_sample_gpu_eq_cpu_to_bits` certifies
// host↔GPU bit-equality.
//
// # Bindings this include CONSUMES (declared by the includer)
//
//   Texture2DArray<float4> gDdgiIrr   : register(t16);  SamplerState gDdgiIrrSamp   : register(s16);
//   Texture2DArray<float2> gDdgiDepth : register(t17);  SamplerState gDdgiDepthSamp : register(s17);
//   cbuffer ResolvedDdgi  : register(b18) { float4 gDdgiOrigin; float4 gDdgiInvSpacDims; ... };
//
// The includer declares those (the deferred resolve at bindings 16/17/18; the golden at its own
// registers) BEFORE `#include`ing this file. The tap helpers below read them.

#ifndef DDGI_RESOLVE_HLSLI
#define DDGI_RESOLVE_HLSLI

// ---- atlas + weight constants (mirror boyko_rhi_vulkan::goldens / ::ddgi) -----------------
static const uint  DDGI_R_IRR_TILE_EDGE     = 8u;   // irradiance tile edge (6x6 valid + 1 border)
static const uint  DDGI_R_IRR_VALID_EXTENT  = 6u;   // irradiance valid interior extent
static const uint  DDGI_R_DEPTH_TILE_EDGE   = 16u;  // depth tile edge (14x14 valid + 1 border)
static const uint  DDGI_R_DEPTH_VALID_EXTENT = 14u; // depth valid interior extent
static const uint  DDGI_R_TILE_BORDER       = 1u;   // the 1-texel border inset
static const float DDGI_R_WRAP_WEIGHT_BIAS  = 0.2;  // wrap/backface small bias
static const float DDGI_R_MIN_SUM_WEIGHT    = 1.0e-6; // epsilon (sky-fallback + Chebyshev guard)

// Irradiance atlas per-layer dims (dx*TILE = 16*8 = 128, dz*TILE = 16*8 = 128).
static const float DDGI_R_IRR_ATLAS_W = 128.0;
static const float DDGI_R_IRR_ATLAS_H = 128.0;
// Depth atlas per-layer dims (dx*TILE = 16*16 = 256, dz*TILE = 16*16 = 256).
static const float DDGI_R_DEPTH_ATLAS_W = 256.0;
static const float DDGI_R_DEPTH_ATLAS_H = 256.0;

// ---- octahedral ENCODE (op-for-op mirror of goldens::oct_encode) -------------------------
// L1-normalize by MULTIPLY-BY-RECIPROCAL (`inv = 1/s; n*inv`) EXACTLY as the host oracle
// `goldens::oct_encode` does — NOT `n / s` (the eDSL body divides; the host mirror multiplies,
// and the I3 GPU golden pins against the MULTIPLY host reference to bits). Result in [0,1]^2.
float2 ddgi_oct_encode(float3 n) {
    float inv_l1 = 1.0 / (abs(n.x) + abs(n.y) + abs(n.z));
    float nx = n.x * inv_l1;
    float ny = n.y * inv_l1;
    float nz = n.z * inv_l1;
    float ex = nx;
    float ey = ny;
    if (nz < 0.0) {
        float sx = (nx >= 0.0) ? 1.0 : -1.0;
        float sy = (ny >= 0.0) ? 1.0 : -1.0;
        ex = (1.0 - abs(ny)) * sx;
        ey = (1.0 - abs(nx)) * sy;
    }
    return float2(ex * 0.5 + 0.5, ey * 0.5 + 0.5);
}

// ---- direction -> irradiance-tile atlas UV (inverse of goldens::ddgi_texel_direction) ----
// The host WRITE places valid interior texel (tx,ty) in [0,VALID_EXTENT) at atlas pixel
// (ox + BORDER + tx, oy + BORDER + ty) where the tile origin is (ox, oy) = (x*TILE, z*TILE) and
// the array layer is `y` (Y-plane-major). `ddgi_texel_direction` maps texel CENTER through
// `u = (tx + 0.5)/VALID_EXTENT` then `oct_decode`. So the READ inverse is: `oct_encode(dir)`
// gives the continuous tile-UV `e` in [0,1]^2 (where e == (tx+0.5)/VALID_EXTENT at a texel
// center); scaling by VALID_EXTENT lands on the fractional valid-interior texel, and offsetting
// by (ox + BORDER) / atlas_dim gives the normalized atlas UV a LINEAR SampleLevel reads.
float3 ddgi_irr_uv(uint3 c, float3 dir) {
    float2 e = ddgi_oct_encode(dir);                         // continuous [0,1]^2 tile UV
    float ox = (float)(c.x * DDGI_R_IRR_TILE_EDGE);          // tile texel origin x (= x*TILE)
    float oy = (float)(c.z * DDGI_R_IRR_TILE_EDGE);          // tile texel origin y (= z*TILE)
    // Valid-interior texel (fractional) + the 1-texel border inset, in atlas TEXEL space.
    float px = ox + (float)DDGI_R_TILE_BORDER + e.x * (float)DDGI_R_IRR_VALID_EXTENT;
    float py = oy + (float)DDGI_R_TILE_BORDER + e.y * (float)DDGI_R_IRR_VALID_EXTENT;
    // Texel -> [0,1] atlas UV (pixel-center convention); array layer = y.
    return float3(px / DDGI_R_IRR_ATLAS_W, py / DDGI_R_IRR_ATLAS_H, (float)c.y);
}

// ---- direction -> depth-tile atlas UV (same inverse, the depth tile geometry) ------------
float3 ddgi_depth_uv(uint3 c, float3 dir) {
    float2 e = ddgi_oct_encode(dir);
    float ox = (float)(c.x * DDGI_R_DEPTH_TILE_EDGE);
    float oy = (float)(c.z * DDGI_R_DEPTH_TILE_EDGE);
    float px = ox + (float)DDGI_R_TILE_BORDER + e.x * (float)DDGI_R_DEPTH_VALID_EXTENT;
    float py = oy + (float)DDGI_R_TILE_BORDER + e.y * (float)DDGI_R_DEPTH_VALID_EXTENT;
    return float3(px / DDGI_R_DEPTH_ATLAS_W, py / DDGI_R_DEPTH_ATLAS_H, (float)c.y);
}

// ---- probe world position (mirror the host `probe_pos` closure / the update pass) --------
// origin + coord * spacing. `origin` is gDdgiOrigin.xyz; `spacing` = 1 / inv_spacing.
float3 ddgi_probe_pos(uint3 c, float3 origin, float spacing) {
    return origin + float3(c) * spacing;
}

// ---- the resolve sample (op-for-op mirror of goldens::probe_sample) -----------------------
// `p`/`n`: receiver world position + normal. `origin`/`inv_spacing`/`dims`: the grid (b18).
// `sky_ambient`: the no-coverage fallback (all corners out-of-bounds or unconverged). Returns
// the trilinearly-blended wrap- + Chebyshev-weighted indirect irradiance.
//
// The corner iteration order is FIXED (z outer, y, x inner) so the float accumulation order
// matches the host oracle exactly (float add is non-associative). Op order pinned to the host:
//   wrap  = ((dot + 1) * 0.5)^2 + bias, floored at 0        (host L4168-4169)
//   var   = max(mean2 - mean*mean, 0)                        (host L4174)
//   cheb  = (dist <= mean) ? 1 : var / max(var + delta*delta, eps)  (host L4177-4181)
//   w     = wx * wy * wz * wrap * cheb                       (host L4183)
//   sum_irr += w * irr ; sum_w += w                          (host L4184-4187)
float3 ddgi_probe_sample(
    float3 p, float3 n,
    float3 origin, float inv_spacing, uint3 dims,
    float3 sky_ambient,
    Texture2DArray<float4> irrTex, SamplerState irrSamp,
    Texture2DArray<float2> depthTex, SamplerState depthSamp)
{
    float spacing = 1.0 / inv_spacing;

    // world -> fractional probe coords, clamped so base and base+1 stay in [0, dims-1].
    float3 hi = float3((float)(max(dims.x, 1u) - 1u),
                       (float)(max(dims.y, 1u) - 1u),
                       (float)(max(dims.z, 1u) - 1u));
    float fx = clamp((p.x - origin.x) * inv_spacing, 0.0, hi.x);
    float fy = clamp((p.y - origin.y) * inv_spacing, 0.0, hi.y);
    float fz = clamp((p.z - origin.z) * inv_spacing, 0.0, hi.z);

    // Base cell + trilinear fractions. `floor` is exact here ONLY because the input was clamped
    // to [0, dims-1] >= 0 (where floor == trunc and HLSL floor matches the host bit-for-bit).
    float bxf = floor(fx);
    float byf = floor(fy);
    float bzf = floor(fz);
    float tx = fx - bxf;
    float ty = fy - byf;
    float tz = fz - bzf;
    uint bx = (uint)bxf;
    uint by = (uint)byf;
    uint bz = (uint)bzf;

    // `precise`: the host oracle (Rust f32) NEVER fuses `a*b+c` (two roundings), but DXC by
    // default CONTRACTS the blend MACs (`sum_irr += w*irr`, `facing*facing+bias`, `var+delta²`,
    // the written-out dot) into single-rounding FMAs — a 1-2 ULP divergence that only surfaces
    // when multiple converged probes accumulate. `precise` forbids contraction/reassociation and
    // propagates backward through the whole dependency tree feeding these accumulators, so every
    // contributing op is emitted as-written (separate OpFMul + OpFAdd) matching the host to bits.
    precise float3 sum_irr = float3(0.0, 0.0, 0.0);
    precise float sum_w = 0.0;

    uint mx = max(dims.x, 1u) - 1u;
    uint my = max(dims.y, 1u) - 1u;
    uint mz = max(dims.z, 1u) - 1u;

    // The 8 surrounding probes: z outer, y, x inner (the grid index order — matches the host).
    [unroll] for (uint cz = 0u; cz < 2u; ++cz) {
        float wz = (cz == 0u) ? (1.0 - tz) : tz;
        uint pz = min(bz + cz, mz);
        [unroll] for (uint cy = 0u; cy < 2u; ++cy) {
            float wy = (cy == 0u) ? (1.0 - ty) : ty;
            uint py = min(by + cy, my);
            [unroll] for (uint cx = 0u; cx < 2u; ++cx) {
                float wx = (cx == 0u) ? (1.0 - tx) : tx;
                uint px = min(bx + cx, mx);
                uint3 idx = uint3(px, py, pz);

                float3 ppos = ddgi_probe_pos(idx, origin, spacing);
                // Direction receiver -> probe (the wrap-weight axis). `normalize(0)` (receiver
                // AT the probe) matches the host `v_normalize` ZERO guard: dot 0 -> neutral wrap.
                float3 dvec = ppos - p;
                float dlen = sqrt(dvec.x * dvec.x + dvec.y * dvec.y + dvec.z * dvec.z);
                float3 to_probe = (dlen > 0.0) ? (dvec / dlen) : float3(0.0, 0.0, 0.0);

                // The depth read serves BOTH the converged sentinel and Chebyshev: boot-clear
                // sets depth == 0, and any updated probe writes mean >= GI_MINT > 0 (a hit) or
                // GI_T_MAX (sky), so `mean == 0` <=> unconverged (Decision: depth-mean sentinel,
                // NOT a classification binding — the resolve set is at its 19/19 cap).
                float3 duv = ddgi_depth_uv(idx, n);
                float2 mom = depthTex.SampleLevel(depthSamp, duv, 0.0).rg;
                float mean = mom.x;
                float mean2 = mom.y;
                if (mean <= 0.0) {
                    continue; // unconverged probe: zero weight (sky fallback until first write)
                }

                // Wrap / backface weight: ((dot + 1) * 0.5)^2 + bias, floored at 0. The dot is
                // WRITTEN OUT (not the `dot` intrinsic) so its mul/add order + associativity match
                // the host `v_dot` (x then y then z, no contraction) for bit-exactness.
                float ndot = to_probe.x * n.x + to_probe.y * n.y + to_probe.z * n.z;
                float facing = (ndot + 1.0) * 0.5;
                float wrap = max(facing * facing + DDGI_R_WRAP_WEIGHT_BIAS, 0.0);

                // Chebyshev two-moment visibility: var = E[d^2] - E[d]^2; unshadowed if nearer.
                float var = max(mean2 - mean * mean, 0.0);
                float dist = sqrt((p.x - ppos.x) * (p.x - ppos.x)
                                + (p.y - ppos.y) * (p.y - ppos.y)
                                + (p.z - ppos.z) * (p.z - ppos.z));
                float delta = max(dist - mean, 0.0);
                float cheb = (dist <= mean) ? 1.0 : (var / max(var + delta * delta, DDGI_R_MIN_SUM_WEIGHT));

                // The probe's octahedral irradiance in the receiver-normal direction (uniform
                // tiles in the golden -> the LINEAR SampleLevel returns the stored value exactly).
                float3 iuv = ddgi_irr_uv(idx, n);
                float3 irr = irrTex.SampleLevel(irrSamp, iuv, 0.0).rgb;

                float w = wx * wy * wz * wrap * cheb;
                sum_irr.x += w * irr.x;
                sum_irr.y += w * irr.y;
                sum_irr.z += w * irr.z;
                sum_w += w;
            }
        }
    }

    // Normalize by the summed weight; below the epsilon there was no coverage -> sky fallback.
    if (sum_w < DDGI_R_MIN_SUM_WEIGHT) {
        return sky_ambient;
    }
    float inv = 1.0 / sum_w;
    return float3(sum_irr.x * inv, sum_irr.y * inv, sum_irr.z * inv);
}

#endif // DDGI_RESOLVE_HLSLI
