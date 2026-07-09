// GUI P5a UI-rect vertex shader (`ui_rect.vs.hlsl`): a vertexless unit quad whose
// corner positions come from SV_VertexID and whose per-instance transform is read
// from a StructuredBuffer<UiInstance> (set 0, binding 0, STORAGE, VERTEX|FRAGMENT)
// by SV_InstanceID — the combination proven by the Rung-0.5 GPU golden.
//
// The VS transforms the quad into NDC by a 2D pixel->NDC ortho push constant
// (top-left origin via a POSITIVE y scale, Vulkan y-down NDC — see UiOrtho), and
// forwards the physical-px fragment position + the instance index to the FS, which
// evaluates the rounded-box SDF + AA + uniform border + flag-gated clip.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 ui_rect.vs.hlsl -Fo ui_rect.vs.spv

// The std430 UiInstance record (mirrors the Rust #[repr(C, align(16))] UiInstance;
// offsets: min_px@0, size_px@8, clip@16, corner_radius@32, color@48, border_color@52,
// border_width@56, flags@60; stride 64). uint scalars hold packed RGBA8 / flags.
struct UiInstance {
    float2 min_px;        // @0
    float2 size_px;       // @8
    float4 clip;          // @16  (min.xy, max.xy; valid iff CLIP_PRESENT)
    float4 corner_radius; // @32  (tl, tr, br, bl)
    uint   color;         // @48  premultiplied RGBA8
    uint   border_color;  // @52  premultiplied RGBA8
    float  border_width;  // @56  uniform, physical px
    uint   flags;         // @60  bit0 BORDER_ANY, bit1 CLIP_PRESENT
};

[[vk::binding(0, 0)]] StructuredBuffer<UiInstance> g_instances : register(t0);

// pixel -> NDC ortho (UiOrtho): ndc = pos_px * scale + translate. 16 bytes.
struct Ortho {
    float2 scale;
    float2 translate;
};
[[vk::push_constant]] Ortho g_ortho;

static const float2 CORNERS[6] = {
    float2(0.0, 0.0),
    float2(1.0, 0.0),
    float2(0.0, 1.0),
    float2(0.0, 1.0),
    float2(1.0, 0.0),
    float2(1.0, 1.0),
};

struct VsOut {
    float4 position  : SV_Position;
    float2 pos_px    : TEXCOORD0;   // physical px, for the FS clip test
    float2 local_px  : TEXCOORD1;   // rect-centred px, for the SDF
    float2 local_uv  : TEXCOORD2;   // GUI P5b (Decision T4-B): the 0..1 quad corner;
                                    // the FS text branch lerps the glyph UV with it
    nointerpolation uint inst_index : INSTANCE;
};

VsOut main(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    UiInstance inst = g_instances[iid];                 // SSBO read in the VERTEX stage
    float2 corner = CORNERS[vid];
    float2 pos_px = inst.min_px + corner * inst.size_px;

    VsOut o;
    o.position   = float4(pos_px * g_ortho.scale + g_ortho.translate, 0.0, 1.0);
    o.pos_px     = pos_px;
    o.local_px   = pos_px - (inst.min_px + 0.5 * inst.size_px); // rect-centred
    o.local_uv   = corner;          // 0..1 within the quad (rect branch ignores it)
    o.inst_index = iid;
    return o;
}
