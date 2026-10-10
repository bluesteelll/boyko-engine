// Multi-paradigm render-path plan, rung R4b-b (code-review follow-up): the Forward v1 sky
// BACKGROUND fragment shader — fills every `lit` pixel with the SAME analytic sky/ground
// gradient + visible sun disc the deferred resolve paints for `mask == 0` (background) pixels
// (`deferred_pbr.hlsl:1369-1414`), so a Forward frame's background matches a Deferred frame's
// exactly instead of staying flat-clear-color/black. Drawn FIRST inside `forward_opaque`'s SAME
// `begin_rendering` scope via its OWN pipeline (depth test/write OFF — see this file's "Depth"
// note), so opaque mesh geometry then draws over it at the pixels it actually covers.
//
// # What this replicates (TOKEN-FOR-TOKEN, `deferred_pbr.hlsl:1369-1414`)
//
// The deferred resolve's background branch: reconstruct the per-pixel view ray
// (`ray_gen.hlsli::generate_ray`), scan the light table's L0a block (`[0, l0a_count)` —
// directionals + sky, the SAME "no-P front block" `forward_opaque.fs.hlsl` scans) for a SKY
// light entry (`sky_color`/`ground_color`, upper/lower hemisphere) and accumulate a visible sun
// disc per DIRECTIONAL light (`SKY_SUN_EXPONENT`-power cosine kernel — the SAME duplicated-pure-
// helper idiom `forward_opaque.fs.hlsl`'s own header documents for `env_brdf_approx`/
// `sun_kernel`), lerp ground→sky along the ray's `LIGHT_UP` alignment, add the sun disc, apply
// exposure, THEN the SAME tonemap + manual gamma OETF the lit path applies
// (`pbr_lighting.hlsli::tonemap_select` + `OETF_GAMMA_EXP`). A scene with NO `SkyLight` entry
// keeps the flat background base color (`deferred_pbr.hlsl`'s `base = albedo_texel.rgb` reads
// the marcher/raster's OWN background-clear constant for an unwritten pixel — BYTE-IDENTICAL to
// `forward.rs::FORWARD_LIT_CLEAR`'s rgb, so this shader hardcodes that SAME constant instead of
// reading a nonexistent gAlbedo texel, the zero-behavior-change equivalent).
//
// # Ray reconstruction under Forward (the ONE adaptation from the compute-dispatch original)
//
// The deferred resolve is a COMPUTE dispatch (`px, py` from `SV_DispatchThreadID`'s linear
// index, `w, h` from `img_w()`/`img_h()`); this is a RASTERIZED full-screen triangle, so `px, py`
// come from the fragment's `SV_Position` (Vulkan's pixel-center convention, `SV_Position.xy -
// 0.5` gives the integer pixel index) instead — `generate_ray`'s math is otherwise IDENTICAL
// (same `cbuffer Camera` shape, same `ray_gen.hlsli` call). `w, h` still come from
// `img_w()`/`img_h()` (VERBATIM copies of `deferred_pbr.hlsl`'s own fallback-to-default helpers,
// duplicated per this codebase's "raster shaders duplicate tiny math" idiom —
// `forward_opaque.vs.hlsl`'s `inverse3x3`/`det3x3` doc — rather than a new shared header for a
// two-line function pair).
//
// # Depth (Decision 4)
//
// NO `SV_Depth`, NO depth attachment referenced by this pipeline at all (`depth_format: None` at
// boot — `VulkanContext::create_graphics_pipeline_forward`'s sibling `create_graphics_pipeline`
// call in `GpuSceneBundles::boot`): Vulkan permits a pipeline with `depthAttachmentFormat ==
// VK_FORMAT_UNDEFINED` to be recorded inside a dynamic-rendering scope that DOES bind a depth
// attachment — the pipeline simply neither tests nor writes it. `forward_opaque`'s OWN pipeline
// (drawn immediately after this one, same scope) keeps its real `VK_COMPARE_OP_GREATER`
// depth-write pass untouched, so mesh geometry still self-occludes and draws over exactly the
// pixels it covers, leaving this pass's color everywhere else.
//
// Set 0 (reused from `forward_opaque`'s OWN Set-0 layout + bind group — `forward_opaque.fs.hlsl`'s
// doc — this FS only references TWO of its five bindings; the rest are bound-but-unread, the SAME
// "a shader references a subset of what its layout/set declares" idiom `forward_opaque.vs.hlsl`'s
// doc states for Set 0's VS-only bindings 0/1): `Camera` @2, `LightBuf` @3.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 forward_sky.fs.hlsl -Fo forward_sky.fs.spv

// `pbr_lighting.hlsli`'s INCLUDE CONTRACT precondition: `PI` + `LIGHT_UP` in scope first (the
// SAME precondition `forward_opaque.fs.hlsl` satisfies before its own `#include`).
static const float PI = 3.14159265358979323846;
static const float3 LIGHT_UP = float3(0.0, 1.0, 0.0);

#include "pbr_lighting.hlsli"
#include "light_table.hlsli"
#include "ray_gen.hlsli"

[[vk::binding(2, 0)]] cbuffer Camera {
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
};
[[vk::binding(3, 0)]] StructuredBuffer<uint> LightBuf;

// VERBATIM copies of `deferred_pbr.hlsl::img_w`/`img_h` (the codebase-wide "0 => a deterministic
// default" idiom every compute shader reading this SAME Camera UBO shape duplicates).
static const uint IMG_W_DEFAULT = 64u;
static const uint IMG_H_DEFAULT = 64u;
uint img_w() { return (img_w_raw != 0u) ? img_w_raw : IMG_W_DEFAULT; }
uint img_h() { return (img_h_raw != 0u) ? img_h_raw : IMG_H_DEFAULT; }

// The flat background-clear color (`forward.rs::FORWARD_LIT_CLEAR`'s rgb; byte-identical to the
// marcher/raster's own `BACKGROUND` clear constant `sdf_gbuffer_composite.hlsl` uses) — the "no
// SkyLight in this scene" fallback, VERBATIM equivalent of `deferred_pbr.hlsl`'s `base =
// albedo_texel.rgb` (which reads that SAME constant back from an unwritten `gAlbedo` texel).
static const float3 BACKGROUND_BASE = float3(0.05, 0.05, 0.1);

// PBR P1 sun-disc kernel exponent — VERBATIM copy of `deferred_pbr.hlsl::SKY_SUN_EXPONENT`
// (a FIXED, moderate cosine-power exponent — the background is a flat environment element, not a
// BRDF lobe, so this is deliberately NOT `sun_kernel_exponent`'s roughness-driven value).
static const float SKY_SUN_EXPONENT = 512.0;

float4 main(float4 sv_position : SV_Position) : SV_Target0 {
    // Vulkan's pixel-center convention: `SV_Position.xy` is `(px + 0.5, py + 0.5)` in framebuffer
    // space — subtracting 0.5 before truncating recovers the integer pixel index, the SAME `px`/
    // `py` a compute dispatch's `SV_DispatchThreadID` would have supplied to `generate_ray`.
    uint px = uint(sv_position.x - 0.5);
    uint py = uint(sv_position.y - 0.5);
    uint w = img_w();
    uint h = img_h();

    float3 ro_bg, rd_bg;
    generate_ray(px, py, w, h, camera_mode, cam_eye.xyz, cam_forward, cam_right, cam_up.xyz, ro_bg, rd_bg);

    LightHeader H_bg = load_light_header(LightBuf);
    bool has_sky = false;
    float3 sky_color = float3(0.0, 0.0, 0.0);
    float3 ground_color = float3(0.0, 0.0, 0.0);
    float3 sun_disc = float3(0.0, 0.0, 0.0);
    for (uint bi = 0u; bi < H_bg.l0a_count; ++bi) {
        LightElem BL = load_light(LightBuf, bi);
        if (light_kind(BL) == LIGHT_KIND_SKY) {
            has_sky = true;
            sky_color = BL.color;      // upper hemisphere (L.color)
            ground_color = BL.pos;     // lower hemisphere (packed in the pos lane)
        } else if (light_kind(BL) == LIGHT_KIND_DIRECTIONAL) {
            // A visible sun disc for every directional light — the SAME `pow`-kernel shape the
            // metal's own sun-disc term uses (`sun_kernel`), but a FIXED exponent.
            float3 bl = normalize(BL.dir);
            sun_disc += BL.color * pow(saturate(dot(rd_bg, bl)), SKY_SUN_EXPONENT);
        }
    }

    float3 lit;
    if (has_sky) {
        float3 sky = lerp(ground_color, sky_color, saturate(dot(rd_bg, LIGHT_UP) * 0.5 + 0.5));
        sky += sun_disc;
        // O3: exposure is the FINAL multiply on the linear radiance, THEN the SAME tonemap +
        // manual gamma OETF the lit path applies.
        sky *= H_bg.exposure;
        sky = tonemap_select(sky, load_tonemap_mode(LightBuf));
        sky = pow(sky, OETF_GAMMA_EXP);
        lit = sky;
    } else {
        // No SkyLight in this scene's table — keep the flat background base color.
        lit = BACKGROUND_BASE;
    }

    return float4(clamp(lit, 0.0, 1.0), 1.0);
}
