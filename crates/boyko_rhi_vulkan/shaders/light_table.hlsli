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

// Unpacks two f16 spot cone cosines (cos_inner in the low half, cos_outer in the high
// half) from a `color_cone.w` bit pattern (the inverse of the host `pack_cones`). SPOT
// only (L0b consumer).
float2 unpack_cones(float packed) {
    uint bits = asuint(packed);
    float cos_inner = f16tof32(bits & 0xFFFFu);
    float cos_outer = f16tof32((bits >> 16) & 0xFFFFu);
    return float2(cos_inner, cos_outer);
}

#endif // LIGHT_TABLE_HLSLI
