// px0_probe_store.comp -- render wave PX0 (H3 Scope 3 / 3b; px0/cut.md D-P2, PX0-6): the TEST-ONLY
// compute kernel of the float-format probe. No render path binds it; only
// `boyko_rhi_vulkan/tests/b10g11r11_rounding_probe.rs` (gpu-ignored) dispatches it. Hand-written
// test HLSL, not eDSL-owned: it computes nothing the model must mirror except A3's `lit * t + s`.
//
// Two variants of ONE source, one storage format each (docs/SHADER-VARIANT-MANIFEST.md):
//   -D FMT_R11G11B10F -> px0_probe_store_r11g11b10f.comp.spv (lit's B10G11R11 candidate)
//   -D FMT_RGBA16F    -> px0_probe_store_rgba16f.comp.spv    (the RGBA16F escape)
//
// # Layout (set 0)
//
//   binding 0: RWTexture2D<float4> gOut -- the target image, GENERAL.
//   binding 1: StructuredBuffer<float4> gIn -- the host-generated inputs, one float4 per texel
//              per layer (index = layer * width * height + y * width + x).
//
// Push: { width, height, mode, layer }. mode 0 = STORE (`gOut = gIn.xyz`, the storage-write
// path's classification), mode 1 = RMW (`gOut = gOut.xyz * gIn.w + gIn.xyz`, arm A3's
// `lit <- lit * t + s` with t in .w and s in .xyz -- one pass per dispatch, the host barriers
// between passes).
//
// # Compile (offline + hermetic; the committed .spv are byte-gated by px0_probe_spv_sync.rs)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       -D FMT_R11G11B10F px0_probe_store.comp.hlsl -Fo px0_probe_store_r11g11b10f.comp.spv
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       -D FMT_RGBA16F px0_probe_store.comp.hlsl -Fo px0_probe_store_rgba16f.comp.spv

#if defined(FMT_R11G11B10F)
[[vk::binding(0, 0)]] [[vk::image_format("r11g11b10f")]] RWTexture2D<float4> gOut : register(u0);
#elif defined(FMT_RGBA16F)
[[vk::binding(0, 0)]] [[vk::image_format("rgba16f")]] RWTexture2D<float4> gOut : register(u0);
#else
#error "define FMT_R11G11B10F or FMT_RGBA16F"
#endif

[[vk::binding(1, 0)]] StructuredBuffer<float4> gIn : register(t1);

struct Px0ProbePush {
    uint width;
    uint height;
    uint mode;
    uint layer;
};
[[vk::push_constant]] Px0ProbePush pc;

[numthreads(8, 8, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= pc.width || id.y >= pc.height) {
        return;
    }
    uint i = pc.layer * pc.width * pc.height + id.y * pc.width + id.x;
    float4 v = gIn[i];
    if (pc.mode == 0u) {
        gOut[id.xy] = float4(v.xyz, 1.0);
    } else {
        float4 d = gOut[id.xy];
        gOut[id.xy] = float4(d.xyz * v.w + v.xyz, 1.0);
    }
}
