// Deferred-shading SPLIT (increment 1): the fullscreen `deferred_pbr` compute RESOLVE.
//
// The marcher (`sdf_gbuffer_composite.hlsl`) no longer composites the lit color; it
// WRITES ATTRIBUTES into the G-buffer (gAlbedo = the unmultiplied base color, gMaterial =
// (r = vis, g = 0, b = mask, a = 1)). This pass reads those two attributes back and
// produces the final LIT image:
//
//     base = gAlbedo.Load(px).rgb;
//     vis  = gMaterial.Load(px).r;                    // clamp(shadow * ao) quantized to R8
//     mask = uint(gMaterial.Load(px).b * 255 + 0.5);  // 1 = SDF-LIT, 0 = mesh/bg/empty
//     lit  = (mask == 1u) ? (base * vis) : base;      // STRICT if/select on mask
//
// The strict `mask` branch is LOAD-BEARING: a vis = 0 SDF pass must black out ONLY the
// SDF-lit arms (mask = 1). Mesh / background / empty pixels carry mask = 0 (and vis = 1),
// so they pass `base` through BYTE-IDENTICALLY — the 0%-gate. A `lerp`/`max` form would
// let a vis = 0 lane darken a mesh/bg pixel; an `if` on mask cannot.
//
// This is a deliberate COMPOSITE split only: the BRDF (the Lambert+ambient base, the A1
// shadow march, the A2 AO march) STAYS in the marcher this increment — there is no
// Cook-Torrance, no material SSBO, no oct-normal, no textures here yet.
//
// # The resolve descriptor set (set 0 of the resolve pipeline — NOT the marcher's vocab)
//
//   binding 0 : RWTexture2D<float4> (STORAGE, rgba8) — gAlbedo (read via `.Load`).
//   binding 1 : RWTexture2D<float4> (STORAGE, rgba8) — gMaterial (read via `.Load`).
//   binding 2 : RWTexture2D<float4> (STORAGE, rgba8) — lit output (stored).
//
// EXACTLY 3 STORAGE bindings, NO sampler, NO UBO: the extent comes from
// `gLit.GetDimensions(w, h)` (the lit image is 1:1 the marched pixels), so the 1D
// dispatch index maps to (px, py) the SAME way the marcher does — no camera UBO needed.
//
// All three images are consumed in GENERAL — gAlbedo / gMaterial are loaded as storage
// images (the marcher's STORAGE views, kept in GENERAL after a memory-only COMPUTE→COMPUTE
// barrier) and `lit` is a storage store. `[[vk::image_format("rgba8")]]` pins each
// `OpTypeImage` to `Rgba8` so it matches the R8G8B8A8_UNORM views
// (shaderStorageImageWriteWithoutFormat is OFF) — the same discipline as the marcher.
//
// Dispatched 1D at the SAME group count as the marcher (`ceil(pixels / 64)`), so the
// resolve covers exactly the marched pixels 1:1.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 deferred_pbr.hlsl \
//       -Fo deferred_pbr.comp.spv
// Validated with:
//   C:\VulkanSDK\1.4.350.0\Bin\spirv-val.exe deferred_pbr.comp.spv

// bindings 0..2: the G-buffer storage images. `[[vk::image_format("rgba8")]]` pins each
// `OpTypeImage` to `Rgba8` so it matches the R8G8B8A8_UNORM views.
[[vk::image_format("rgba8")]] RWTexture2D<float4> gAlbedo   : register(u0);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gMaterial : register(u1);
[[vk::image_format("rgba8")]] RWTexture2D<float4> gLit      : register(u2);

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint idx = tid.x;

    // The extent comes from the lit image (1:1 the marched pixels) — no camera UBO. The 1D
    // index maps to (px, py) the SAME way the marcher's `idx % w` / `idx / w` does.
    uint w, h;
    gLit.GetDimensions(w, h);
    if (idx >= w * h) {
        return;
    }
    uint px = idx % w;
    uint py = idx / w;

    // `RWTexture2D.Load` (a UAV read) takes a 2D coord (no mip lane). Load the full float4
    // once per image, then swizzle, to avoid an implicit vector truncation.
    int2 coord = int2((int)px, (int)py);
    float4 albedo_texel = gAlbedo.Load(coord);
    float4 material_texel = gMaterial.Load(coord);
    float3 base = albedo_texel.rgb;
    float vis = material_texel.r;
    // BUG-PBR-F1: `mask` is stored in gMaterial.b as the FLOAT 1.0 (SDF-LIT) or 0.0
    // (mesh/bg/empty). An R8_UNORM store quantizes 1.0 → byte 255 (NOT 1) and 0.0 → byte 0,
    // and the UAV `.Load` reads the byte back NORMALIZED to [0,1] (255 → 1.0, 0 → 0.0). The
    // prior decode `uint(b * 255 + 0.5)` mapped the SDF-LIT flag to 255u, so the strict
    // `mask == 1u` test was ALWAYS false → every SDF-lit pixel wrongly took the `base`
    // pass-through (its `vis` was ignored), un-shadowing the crater self-shadow rim. The
    // flag is BINARY (only 1.0 or 0.0 is ever stored), so decode it as a threshold on the
    // normalized value: `> 0.5` is SDF-LIT. This matches the host `golden_deferred_resolve`'s
    // `attrs.mask == 1` (the decoded 0/1 flag) bit-for-bit and is robust to the R8 round-trip.
    bool is_sdf_lit = material_texel.b > 0.5;

    // STRICT if/select on the SDF-lit flag (NOT lerp/max): only SDF-lit pixels get the
    // base * vis composite; mesh / background / empty pass `base` through byte-identically
    // even when vis would be 0 — the 0%-gate.
    float3 lit = is_sdf_lit ? (base * vis) : base;

    gLit[uint2(px, py)] = float4(clamp(lit, 0.0, 1.0), 1.0);
}
