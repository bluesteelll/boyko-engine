// px0_probe_raster.fs -- render wave PX0 (H3 Scope 3 / 3b; px0/cut.md D-P2, PX0-6): the TEST-ONLY
// fragment shader of the float-format probe's raster paths (colour attachment, blend ONE + ONE,
// blend premultiplied over). Paired with the existing `fullscreen_sample.vs.spv`; dispatched only
// by `boyko_rhi_vulkan/tests/b10g11r11_rounding_probe.rs` (gpu-ignored). It computes NOTHING: the
// value it returns is the host-generated float4 of its pixel and layer, and the blend state does
// the rest, so the device sequence equals the host model's by construction.
//
// # Layout
//
//   set 0, binding 0: StructuredBuffer<float4> gIn -- one float4 (rgb = the source colour,
//                     a = the blend's source alpha) per texel per layer.
//   push: { width, height, layer, _pad }.
//
// # Compile (offline + hermetic; byte-gated by px0_probe_spv_sync.rs)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main -fspv-target-env=vulkan1.3 \
//       px0_probe_raster.fs.hlsl -Fo px0_probe_raster.fs.spv

[[vk::binding(0, 0)]] StructuredBuffer<float4> gIn : register(t0);

struct Px0RasterPush {
    uint width;
    uint height;
    uint layer;
    uint pad;
};
[[vk::push_constant]] Px0RasterPush pc;

float4 main(float4 pos : SV_Position) : SV_Target0 {
    uint2 p = uint2(pos.xy);
    uint i = pc.layer * pc.width * pc.height + p.y * pc.width + p.x;
    return gIn[i];
}
