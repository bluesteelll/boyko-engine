// GUI P5a UI-rect fragment shader (`ui_rect.fs.hlsl`): a rounded-box SDF + AA +
// UNIFORM border + flag-gated in-shader clip, output PREMULTIPLIED (matches the
// engine's src=ONE premultiplied-alpha blend).
//
// The record is re-read from the same StructuredBuffer<UiInstance> by the
// interpolated SV_InstanceID (the FRAGMENT-stage SSBO read proven by Rung 0.5).
// All math is in physical px (the VS forwards rect-centred local_px), so fwidth(d)
// is one device-pixel wide AA — DPI-correct under the Decision-9 full-extent
// contract (the UI viewport spans the same pixels the ortho denominator uses).
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 ui_rect.fs.hlsl -Fo ui_rect.fs.spv

struct UiInstance {
    float2 min_px;
    float2 size_px;
    float4 clip;
    float4 corner_radius;
    uint   color;
    uint   border_color;
    float  border_width;
    uint   flags;
};

[[vk::binding(0, 0)]] StructuredBuffer<UiInstance> g_instances : register(t0);

struct VsOut {
    float4 position  : SV_Position;
    float2 pos_px    : TEXCOORD0;
    float2 local_px  : TEXCOORD1;
    nointerpolation uint inst_index : INSTANCE;
};

static const uint FLAG_BORDER_ANY   = 1u << 0;
static const uint FLAG_CLIP_PRESENT = 1u << 1;

// Unpack a premultiplied RGBA8 (byte0=R..byte3=A) to a float4 in [0,1].
float4 unpack_rgba8(uint c) {
    return float4(
        float(c & 0xFFu),
        float((c >> 8) & 0xFFu),
        float((c >> 16) & 0xFFu),
        float((c >> 24) & 0xFFu)
    ) * (1.0 / 255.0);
}

// Quilez/Bevy per-corner rounded-box SDF. `p` is rect-centred; `half_size` is half
// the rect; `r` is (tl, tr, br, bl). Selects the corner radius by the quadrant of p.
float sd_rounded_box(float2 p, float2 half_size, float4 r) {
    // x = right side (p.x > 0) ? right radii : left radii.
    float2 rx = (p.x > 0.0) ? r.yz : r.xw; // (tr,br) : (tl,bl)
    float  rr = (p.y > 0.0) ? rx.y : rx.x; // pick top vs bottom within that side
    float2 q = abs(p) - half_size + rr;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rr;
}

// Anti-aliased coverage of a finite clip AABB at physical-px `pos`. 1 inside, 0
// outside, a ~1px AA band at the edges (so a clipped edge is as crisp as a rect
// edge). `clip` is (min.xy, max.xy), always finite when CLIP_PRESENT is set.
float clip_coverage(float2 pos, float4 clip, float fw) {
    float2 inside_min = smoothstep(clip.xy - fw, clip.xy + fw, pos);          // 1 past min
    float2 inside_max = smoothstep(clip.zw - fw, clip.zw + fw, pos);          // 1 past max
    float2 cov = inside_min * (1.0 - inside_max);
    return cov.x * cov.y;
}

float4 main(VsOut input) : SV_Target0 {
    UiInstance inst = g_instances[input.inst_index];     // SSBO read in the FRAGMENT stage
    float2 half_size = 0.5 * inst.size_px;

    float d  = sd_rounded_box(input.local_px, half_size, inst.corner_radius);
    float fw = max(fwidth(d), 1e-5);                      // resolution-independent AA
    float fill_cov = 1.0 - smoothstep(-fw, fw, d);       // outer-shape coverage

    // `inst.color` is PREMULTIPLIED RGBA8. unpack_rgba8 yields a premultiplied
    // float4 (rgb already carries the * a factor); `result` accumulates the final
    // premultiplied pixel.
    float4 result = unpack_rgba8(inst.color) * fill_cov;     // fill, area-weighted

    // UNIFORM border (P5a exact): the inner shape is the same rounded box inset by
    // the border width; the ring between outer and inner is the border color, drawn
    // OVER the fill in PREMULTIPLIED space.
    if ((inst.flags & FLAG_BORDER_ANY) != 0u) {
        float bw = inst.border_width;
        float4 inner_r = max(inst.corner_radius - bw, 0.0);
        float d_inner = sd_rounded_box(input.local_px, half_size - bw, inner_r);
        float inner_cov = 1.0 - smoothstep(-fw, fw, d_inner);
        float border_cov = saturate(fill_cov - inner_cov);    // exact ring (uniform)

        // Premultiplied "over": composite the border ring (premultiplied color
        // `bc`, area-weighted by `border_cov`) ON TOP of the fill. The fill is
        // restricted to the inner shape (`inner_cov`); where the border ring is
        // translucent (border alpha < 1), the fill shows through correctly via the
        // (1 - src_a) term — `lerp` of two premultiplied colors would mis-weight
        // that fall-through. `src` = border * border_cov; its effective alpha is
        // `bc.a * border_cov`. `dst` = fill restricted to the inner shape.
        //   result = src + dst * (1 - src_alpha)
        float4 bc  = unpack_rgba8(inst.border_color);
        float4 src = bc * border_cov;                         // premultiplied, weighted
        float4 dst = unpack_rgba8(inst.color) * inner_cov;    // fill under the ring
        result = src + dst * (1.0 - src.a);                   // premultiplied OVER
    }

    if ((inst.flags & FLAG_CLIP_PRESENT) != 0u) {        // well-predicted uniform branch
        result *= clip_coverage(input.pos_px, inst.clip, fw);
    }

    // PREMULTIPLIED output (rgb already carries * a; coverage already folded in).
    return result;
}
