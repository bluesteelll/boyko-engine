// env_ibl.hlsli -- IBL1 R0 + R1 (ibl1/cut.md C10, ruling 117): the ENVIRONMENT arm of the six lit
// producers' SKY branch, compiled ONLY into their `-D REFL=1` twins.
//
// The prefiltered environment is a bordered octahedral 7-level B10G11R11 chain (the procedural
// studio, baked offline by `boyko_shaderdsl::ibl`, committed as `boyko_render/assets/ibl/
// studio_256.envoct`). Its SKY row (`boyko_render::light::GpuLight::from_environment`) carries the
// kind flag `LIGHT_FLAG_ENV` and its parameters in the lanes `from_sky` leaves unused:
//   L.dir   = (intensity, cos yaw, sin yaw)     L.range     = L1 (levels - 1)
//   L.color = sky lane x intensity              L.cone_pack = the level-0 tile side
//   L.pos   = ground lane x intensity
// so `light_kind(L) == LIGHT_KIND_SKY` still dispatches it (the flag sits above the kind mask), the
// background and the hemisphere diffuse read a sky/ground pair consistent with the map, and only
// the REFL twins look at the flag.
//
// INCLUDE CONTRACT: the including producer declares, BEFORE this include and at ONE binding (a
// combined image sampler), `Texture2D<float4> gEnvMap` and `SamplerState gEnvSamp` (a
// `MipMode::Linear` sampler, maxLod unclamped -- a `maxLod = 0` sampler would read level 0 for every
// roughness), and has included `pbr_lighting.hlsli` (`Surface`) and `light_table.hlsli` (`LightElem`).
//
// The leaves between the GENERATED sentinels are traced from `boyko_shaderdsl::ibl` (the SAME
// bodies the bake and the host oracles instantiate over f32) by
// `boyko_shaderdsl::emit::emit_hlsl_env_ibl`; `boyko_rhi_vulkan/tests/env_ibl_edsl_sync.rs` re-runs
// the printer and re-DXCs the 20 REFL `.spv`. Do not hand-edit the generated span: change the leaf,
// run `cargo run -p boyko_shaderdsl --features emit --bin emit_env_ibl`, re-splice, re-DXC every
// REFL row (docs/SHADER-VARIANT-MANIFEST.md).
#ifndef ENV_IBL_HLSLI
#define ENV_IBL_HLSLI

// === GENERATED env_ibl BEGIN === (boyko_shaderdsl::emit::emit_hlsl_env_ibl)
static const uint LIGHT_FLAG_ENV = 0x1000000u;

bool light_env_armed(LightElem L) {
    return (L.kind & LIGHT_FLAG_ENV) != 0u;
}

float env_lod(float pr, float l1) {
    float t0 = 2.0 - pr;
    float t1 = pr * t0;
    float t2 = l1 * t1;
    return t2;
}

float2 oct_env_uv(float3 d, float tile) {
    float t0 = abs(d.x);
    float t1 = abs(d.y);
    float t2 = t0 + t1;
    float t3 = abs(d.z);
    float t4 = t2 + t3;
    float t5 = 1.0 / t4;
    float t6 = d.x * t5;
    float t7 = d.y * t5;
    float t8 = d.z * t5;
    float t9 = (t6 >= 0.0) ? 1.0 : -1.0;
    float t10 = (t8 >= 0.0) ? 1.0 : -1.0;
    float t11 = abs(t8);
    float t12 = 1.0 - t11;
    float t13 = t12 * t9;
    float t14 = abs(t6);
    float t15 = 1.0 - t14;
    float t16 = t15 * t10;
    float t17 = (t7 < 0.0) ? t13 : t6;
    float t18 = (t7 < 0.0) ? t16 : t8;
    float t19 = tile - 2.0;
    float t20 = t19 / tile;
    float t21 = 1.0 / tile;
    float t22 = t17 * 0.5;
    float t23 = t22 + 0.5;
    float t24 = t23 * t20;
    float t25 = t24 + t21;
    float t26 = t18 * 0.5;
    float t27 = t26 + 0.5;
    float t28 = t27 * t20;
    float t29 = t28 + t21;
    return float2(t25, t29);
}

float3 spec_dominant_dir(float3 n, float3 r, float pr) {
    float t0 = 1.0 - pr;
    float t1 = sqrt(t0);
    float t2 = t1 + pr;
    float t3 = t0 * t2;
    float t4 = lerp(n.x, r.x, t3);
    float t5 = lerp(n.y, r.y, t3);
    float t6 = lerp(n.z, r.z, t3);
    float t7 = t4 * t4;
    float t8 = t5 * t5;
    float t9 = t7 + t8;
    float t10 = t6 * t6;
    float t11 = t9 + t10;
    float t12 = sqrt(t11);
    float t13 = 1.0 / t12;
    float t14 = t4 * t13;
    float t15 = t5 * t13;
    float t16 = t6 * t13;
    return float3(t14, t15, t16);
}

float3 env_yaw_rotate(float3 d, float c, float s) {
    float t0 = c * d.x;
    float t1 = s * d.z;
    float t2 = t0 - t1;
    float t3 = s * d.x;
    float t4 = c * d.z;
    float t5 = t3 + t4;
    return float3(t2, d.y, t5);
}

float3 eval_pbr_ambient_env(Surface s, float2 dfg, float3 env_spec, float3 sky_color, float3 ground_color, float hemi, float ao_final, float spec_ao) {
    float t0 = lerp(ground_color.x, sky_color.x, hemi);
    float t1 = s.f0.x * dfg.x;
    float t2 = t1 + dfg.y;
    float t3 = t2 * env_spec.x;
    float t4 = t3 * s.energy_comp.x;
    float t5 = s.diffuse_color.x * t0;
    float t6 = t5 * ao_final;
    float t7 = t4 * spec_ao;
    float t8 = t6 + t7;
    float t9 = lerp(ground_color.y, sky_color.y, hemi);
    float t10 = s.f0.y * dfg.x;
    float t11 = t10 + dfg.y;
    float t12 = t11 * env_spec.y;
    float t13 = t12 * s.energy_comp.y;
    float t14 = s.diffuse_color.y * t9;
    float t15 = t14 * ao_final;
    float t16 = t13 * spec_ao;
    float t17 = t15 + t16;
    float t18 = lerp(ground_color.z, sky_color.z, hemi);
    float t19 = s.f0.z * dfg.x;
    float t20 = t19 + dfg.y;
    float t21 = t20 * env_spec.z;
    float t22 = t21 * s.energy_comp.z;
    float t23 = s.diffuse_color.z * t18;
    float t24 = t23 * ao_final;
    float t25 = t22 * spec_ao;
    float t26 = t24 + t25;
    return float3(t8, t17, t26);
}
// === GENERATED env_ibl END ===

// env_specular -- the prefiltered radiance along `dir` at perceptual roughness `pr` (hand-written:
// the texture fetch is not an eDSL op). The lod is the bake's own map (`env_lod`); levels
// floor(lod) and floor(lod) + 1 (clamped to L1) are each fetched BILINEARLY at THEIR OWN bordered UV
// (`oct_env_uv` with that level's side) and lerped: one trilinear fetch at one UV would misregister
// the coarser level by up to half a texel at the tile edges (the interior scale (N - 2)/N differs
// per level). The host model (`boyko_rhi_vulkan::goldens`) is the same two bilinear reads.
float3 env_specular(float3 dir, float pr, LightElem L) {
    float3 d = env_yaw_rotate(dir, L.dir.y, L.dir.z);
    float l1 = L.range;
    float lod = min(env_lod(pr, l1), l1);
    float k0 = floor(lod);
    float k1 = min(k0 + 1.0, l1);
    float f = lod - k0;
    uint tile = uint(L.cone_pack);
    float n0 = float(tile >> uint(k0));
    float n1 = float(tile >> uint(k1));
    float3 c0 = gEnvMap.SampleLevel(gEnvSamp, oct_env_uv(d, n0), k0).rgb;
    float3 c1 = gEnvMap.SampleLevel(gEnvSamp, oct_env_uv(d, n1), k1).rgb;
    return lerp(c0, c1, f) * L.dir.x;
}

#endif // ENV_IBL_HLSLI
