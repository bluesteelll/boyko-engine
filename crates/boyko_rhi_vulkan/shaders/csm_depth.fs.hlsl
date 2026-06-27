// CSM Increment 1b — Rung A: the cascade DEPTH-PASS fragment shader.
//
// EMPTY (depth-only). The cascade depth pass declares NO color attachment; the only
// output is the rasterizer's interpolated `SV_Position.z`, which the fixed-function
// depth test writes into the cascade's D32 layer. This fragment stage exists ONLY so
// the graphics pipeline is complete (a vertex+fragment pair); it does no work and
// writes nothing. The matching vertex shader is `csm_depth.vs.hlsl`.
//
// Mirrors the depth-only-prepass precedent (`gbuffer.fs.hlsl` was likewise a stand-in
// before the MRT upgrade); here the fragment truly produces nothing because the pass
// is depth-only by design (no `SV_Target`, no `SV_Depth`).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 csm_depth.fs.hlsl -Fo csm_depth.fs.spv

void main() {
}
