// Multi-paradigm render-path plan, rung R5 (ForwardPlus): the depth-only PRE-PASS fragment
// shader. `[[earlydepthstencil]]`-clean by construction: NO `SV_Depth` write, NO `discard`, NO
// UAV, and — the defining feature of this pass — NO `SV_Target` output at all. `RhiDevice::
// create_graphics_pipeline_forward_prepass` builds this pipeline with ZERO color-attachment
// formats (`GraphicsPipelineDesc::color_formats` empty), the SAME depth-only shape the CSM
// cascade / punctual-atlas shadow-map pipelines already use (`build_graphics_pipeline`'s
// `color_attachment_count == 0` path) — Vulkan permits a fragment shader with no color outputs
// inside a rendering scope that binds a depth attachment only. The RHI's `GraphicsPipelineDesc`
// requires a fragment module unconditionally (no null-FS pipeline shape exists in this engine),
// so this is the minimal, valid stand-in: an empty entry point.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 depth_prepass.fs.hlsl -Fo depth_prepass.fs.spv

void main() {
}
