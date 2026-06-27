// Shared light-table std430 decode (`light_table.hlsli`) — Lighting L0.
//
// The ONE source of truth for the GPU light table's std430 layout, included by the
// deferred resolve (`deferred_pbr.hlsl`) and (L1) the cluster cull (`cluster_cull.hlsl`),
// exactly like `ray_gen.hlsli` is the one ray-gen. Host mirror:
// `boyko_render::light::{GpuLight, LightHeaderGpu}` + `compute.rs::{GoldenLight,
// GoldenLightHeader}`; the const-asserted host fingerprints + the pins below keep them
// locked.
//
// # The word-indexed HEADER_BASE layout (Decision 8)
//
// The light SSBO is `StructuredBuffer<uint> LightBuf` indexed BY WORD (the proven
// edit-list HEADER_BASE idiom, `sdf_gbuffer_composite.hlsl:367`): the leading
// `LIGHT_HEADER_WORDS` (16) words are the `LightHeaderGpu`, then a flat `GpuLight[]`
// each `GPU_LIGHT_WORDS` (12) words, starting at `LIGHT_HEADER_BASE`. A `uint` buffer
// (not `StructuredBuffer<GpuLight>`) is used so the 16-word header and the 12-word
// elements coexist without a struct-stride alignment clash.
//
//   header word 0 : asuint -> light_count          (lane counts_exposure.x)
//   header word 1 : asfloat -> exposure            (lane counts_exposure.y, default 1.0)
//   header word 2 : asuint -> l0a_count            (lane counts_exposure.z; dir + sky)
//   header word 3 : asuint -> point_spot_count     (lane counts_exposure.w)
//   header word 4..6 : sky_diffuse.rgb (carried; the L0a ambient comes from sky entities)
//   header word 8..10: sky_spec.rgb    (carried; see above)
//   header word 12..15: cluster_params (zero in L0)
//
//   element i lane 0 (words +0..3) : dir_kind   (xyz dir, w = bitcast kind)
//   element i lane 1 (words +4..7) : pos_range  (xyz pos | ground, w = radius)
//   element i lane 2 (words +8..11): color_cone (rgb color×I | sky, w = packed cones)

#ifndef LIGHT_TABLE_HLSLI
#define LIGHT_TABLE_HLSLI

// Word counts — host pins: GPU_LIGHT_WORDS == 12, LIGHT_HEADER_WORDS == 16.
static const uint GPU_LIGHT_WORDS    = 12u;
static const uint LIGHT_HEADER_WORDS = 16u;
static const uint LIGHT_HEADER_BASE  = LIGHT_HEADER_WORDS; // GpuLight[] starts here

// Light kinds — mirror boyko_render::light::LIGHT_KIND_* + compute.rs GOLDEN_LIGHT_KIND_*.
static const uint LIGHT_KIND_DIRECTIONAL = 0u;
static const uint LIGHT_KIND_POINT       = 1u;
static const uint LIGHT_KIND_SPOT        = 2u;
static const uint LIGHT_KIND_SKY         = 3u;

// === P6 R1 — the per-light `casts_sdf_shadow` flag, packed into the kind word ===========
//
// `GpuLight`'s kind word (element lane-0 `.w`, decoded into `LightElem.kind`) carries the
// `LIGHT_KIND_*` enum in its LOW bits (0..3) and the P6 R1 `casts_sdf_shadow` flag in BIT
// 16. ADDITIVE helpers (the existing `load_light` / `LightElem.kind` decode is UNTOUCHED, so
// `cluster_cull.hlsl`'s `.spv` is unaffected — it never calls these; the helpers are defined
// AFTER `LightElem` below). The resolve uses `light_kind()` for the kind COMPARISONS (masking
// off the flag bits) and `light_casts_sdf_shadow()` to gate the per-light SDF shadow march.
// On every pre-P6 scene bit 16 is 0, so `light_kind() == e.kind` and the comparisons are
// byte-equivalent to today (the 0%-gate).
static const uint LIGHT_KIND_MASK         = 0xFFFFu; // the kind enum lives in the low 16 bits
static const uint LIGHT_FLAG_CASTS_SHADOW = 0x10000u; // bit 16: this light casts an SDF shadow

// === P6 R1 — the resolve shadow_mode, sourced from a SPARE header word =================
//
// Header word 7 (`counts_exposure`/`sky_diffuse`'s tail — `sky_diffuse.w`, NEVER read by the
// L0a sky ambient, which uses only words 4..6) carries the P6 R1 `shadow_mode`:
//   0 = legacy single-directional (the primary reads `gMaterial.r`, NO resolve march — the
//       BYTE-IDENTICAL 0%-gate; word 7 is 0.0 on every pre-P6 scene).
//   1 = multi-light: the primary directional KEEPS `gMaterial.r`; every EXTRA flagged caster
//       gets a `sdf_soft_shadow_ranged` march in the resolve's per-light loop.
// Sourced from a header word (not the marcher push) so the FROZEN marcher is untouched.
static const uint SHADOW_MODE_LEGACY     = 0u;
static const uint SHADOW_MODE_MULTI_LIGHT = 1u;

// The `shadow_mode` lives in BIT 0 of header word 7 (masked off so a contact-shadow-on scene,
// which sets BIT 1, never reads `shadow_mode == 3`). Arithmetically inert for every existing
// golden whose word 7 ∈ {0,1} (`x & 1 == x`).
uint load_shadow_mode(StructuredBuffer<uint> LightBuf) {
    return LightBuf[7] & 1u;
}

// === Render Shadow Phase 3 — the resolve `contact_shadow_mode`, packed in word 7 BIT 1 ========
//
// Screen-Space Contact Shadows (SSCS) are gated by BIT 1 of the SAME header word 7 that carries
// `shadow_mode` in BIT 0. The header is FULL (16 words / 4 vec4), so a spare BIT in an existing
// word is used rather than a new word (which would shift `LIGHT_HEADER_BASE` and re-encode every
// golden). On every pre-Phase-3 scene word 7 ∈ {0,1} → BIT 1 is 0 → `contact_shadow_mode == OFF`
// → the SSCS march block (a structural `if`) never runs → byte-identical to today (the 0%-gate).
static const uint CONTACT_SHADOW_MODE_OFF = 0u;
static const uint CONTACT_SHADOW_MODE_ON  = 1u;

uint load_contact_shadow_mode(StructuredBuffer<uint> LightBuf) {
    return (LightBuf[7] >> 1) & 1u;
}

// === Render P7 — the resolve ssao_mode, sourced from a SPARE header word ===============
//
// Header word 11 (`sky_spec.w` — NEVER read by the L0a sky ambient, which uses only
// words 8..10 for the spec hemisphere) carries the Render P7 `ssao_mode`, mirroring
// `load_shadow_mode` (word 7) EXACTLY (the same `LightBuf[N]` raw word accessor):
//   0 = SSAO OFF (the resolve combine is `ao_final == gMaterial.g` — the BYTE-IDENTICAL
//       0%-gate; word 11 is 0 on every pre-P7 scene).
//   1 = SSAO ON: the resolve arms the `ao_final = min(class_ao, gSsao)` cross-representation
//       combine (mesh pixels take pure SSAO; SDF pixels take the most-occluded of the exact
//       A2 march and SSAO).
// Sourced from a header word (not the marcher push) so the FROZEN marcher is untouched.
static const uint SSAO_MODE_OFF = 0u;
static const uint SSAO_MODE_ON  = 1u;

uint load_ssao_mode(StructuredBuffer<uint> LightBuf) {
    return LightBuf[11];
}

// The decoded header (read once per dispatch — a wave-uniform broadcast).
struct LightHeader {
    uint   light_count;
    float  exposure;
    uint   l0a_count;        // directionals + sky (the no-P front block)
    uint   point_spot_count; // the L0b block (needs gViewT/P)
    float3 sky_diffuse;      // carried; L0a ambient is driven by sky light entities
    float3 sky_spec;         // carried; see above
};

// One decoded light-table element.
struct LightElem {
    float3 dir;       // direction TO the light (directional/spot)
    uint   kind;      // LIGHT_KIND_*
    float3 pos;       // world pos (point/spot) | ground color (sky)
    float  range;     // cull radius (point/spot)
    float3 color;     // LINEAR color × baked intensity | sky color
    float  cone_pack; // packed spot cone cosines (asfloat) — SPOT only
};

// Decodes the table header from the leading region of the word-indexed light SSBO.
LightHeader load_light_header(StructuredBuffer<uint> LightBuf) {
    LightHeader h;
    h.light_count      = LightBuf[0];
    h.exposure         = asfloat(LightBuf[1]);
    h.l0a_count        = LightBuf[2];
    h.point_spot_count = LightBuf[3];
    h.sky_diffuse      = float3(asfloat(LightBuf[4]), asfloat(LightBuf[5]), asfloat(LightBuf[6]));
    h.sky_spec         = float3(asfloat(LightBuf[8]), asfloat(LightBuf[9]), asfloat(LightBuf[10]));
    return h;
}

// Decodes light-table element `i` (0-based into the GpuLight[] array after the header).
LightElem load_light(StructuredBuffer<uint> LightBuf, uint i) {
    uint b = LIGHT_HEADER_BASE + i * GPU_LIGHT_WORDS;
    LightElem e;
    e.dir       = float3(asfloat(LightBuf[b + 0]), asfloat(LightBuf[b + 1]), asfloat(LightBuf[b + 2]));
    e.kind      = LightBuf[b + 3];
    e.pos       = float3(asfloat(LightBuf[b + 4]), asfloat(LightBuf[b + 5]), asfloat(LightBuf[b + 6]));
    e.range     = asfloat(LightBuf[b + 7]);
    e.color     = float3(asfloat(LightBuf[b + 8]), asfloat(LightBuf[b + 9]), asfloat(LightBuf[b + 10]));
    e.cone_pack = asfloat(LightBuf[b + 11]);
    return e;
}

// P6 R1: the kind enum with the flag bits masked off — use this for every `kind ==`
// comparison (the resolve does; the cull keeps the raw `e.kind`, unaffected since a
// shadow-flagged point's `kind != LIGHT_KIND_POINT` correctly skips the cull until the
// follow-up rung masks it there too).
uint light_kind(LightElem e) {
    return e.kind & LIGHT_KIND_MASK;
}

// P6 R1: true iff this light is flagged a per-light SDF-shadow caster. Gated additionally by
// the header's `shadow_mode` in the resolve (a `shadow_mode==0` scene never marches).
bool light_casts_sdf_shadow(LightElem e) {
    return (e.kind & LIGHT_FLAG_CASTS_SHADOW) != 0u;
}

// Unpacks two f16 spot cone cosines (cos_inner in the low half, cos_outer in the high
// half) from a `color_cone.w` bit pattern (the inverse of the host `pack_cones`). SPOT
// only (L0b consumer).
float2 unpack_cones(float packed) {
    uint bits = asuint(packed);
    float cos_inner = f16tof32(bits & 0xFFFFu);
    float cos_outer = f16tof32((bits >> 16) & 0xFFFFu);
    return float2(cos_inner, cos_outer);
}

// === L1 cluster grid (Decision 6) — the shared cull-write / resolve-read source of truth ==
//
// The cluster grid is a `StructuredBuffer<uint2>` ClusterCell[CLUSTER_COUNT] ({offset,count}
// into the flat light-index list) + a `StructuredBuffer<uint>` LightIndexList. The header's
// `cluster_params` lane (LightBuf words 12..16) carries the exp-Z lookup factors:
//   word 12 : asfloat -> z_scale  (slice = ln(view_z) * z_scale + z_bias)
//   word 13 : asfloat -> z_bias
//   word 14 : asuint  -> packed_dims = dim_x | dim_y<<8 | dim_z<<16
//   word 15 : asuint  -> clusters_enabled (0 => L1 OFF, resolve loops the flat table)
// Host mirror: boyko_render::light::{ClusterConfig, LightHeaderGpu::new_clustered}.

// Decoded L1 cluster lookup params (read once per pixel in the resolve, per froxel in cull).
struct ClusterParams {
    float z_scale;        // exp-Z affine slice scale
    float z_bias;         // exp-Z affine slice bias
    uint  dim_x;          // froxel grid X
    uint  dim_y;          // froxel grid Y
    uint  dim_z;          // froxel grid Z (exp-Z slices)
    uint  clusters_enabled; // 0 => loop the flat table (L1 0%-gate == L0b)
};

// Decodes the L1 cluster params from the header's `cluster_params` lane.
ClusterParams load_cluster_params(StructuredBuffer<uint> LightBuf) {
    ClusterParams p;
    p.z_scale          = asfloat(LightBuf[12]);
    p.z_bias           = asfloat(LightBuf[13]);
    uint packed        = LightBuf[14];
    p.dim_x            = packed & 0xFFu;
    p.dim_y            = (packed >> 8) & 0xFFu;
    p.dim_z            = (packed >> 16) & 0xFFu;
    p.clusters_enabled = LightBuf[15];
    return p;
}

// Linearizes froxel (x,y,z) to its flat ClusterCell index. THE one source of truth: the
// cull-WRITE and the resolve-READ both call this, so they can never disagree (a mismatch
// silently maps a pixel to the wrong cluster). Mirrors host `light::cluster_index`:
// `(y * dim_x + x) * dim_z + z` (Z innermost so a depth walk is contiguous).
uint cluster_linear_index(uint x, uint y, uint z, uint dim_x, uint dim_z) {
    return (y * dim_x + x) * dim_z + z;
}

// Maps a view-space depth `view_z` to its exp-Z froxel slice, clamped to [0, dim_z-1]. The
// affine `ln(view_z) * z_scale + z_bias` inverts `view_z = near * (far/near)^(slice/dim_z)`.
// A `view_z <= 0` (behind the near plane / sentinel) clamps to slice 0.
uint cluster_z_slice(float view_z, ClusterParams p) {
    if (view_z <= 0.0) {
        return 0u;
    }
    float slice = log(view_z) * p.z_scale + p.z_bias;
    int si = (int)floor(slice);
    if (si < 0) { si = 0; }
    if (si > (int)p.dim_z - 1) { si = (int)p.dim_z - 1; }
    return (uint)si;
}

// Maps a pixel (px,py) at extent (w,h) to its froxel (x,y) tile, clamped to the grid. The
// tile is `px * dim_x / w` (integer, evenly spreads pixels across froxel columns).
uint2 cluster_xy_tile(uint px, uint py, uint w, uint h, ClusterParams p) {
    uint tx = (px * p.dim_x) / max(w, 1u);
    uint ty = (py * p.dim_y) / max(h, 1u);
    if (tx > p.dim_x - 1u) { tx = p.dim_x - 1u; }
    if (ty > p.dim_y - 1u) { ty = p.dim_y - 1u; }
    return uint2(tx, ty);
}

#endif // LIGHT_TABLE_HLSLI
