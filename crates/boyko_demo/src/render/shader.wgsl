// Instanced quad shader for boyko_demo (plan §5.3, Phase 20.1 D1/D7).
//
// Slot 0 (per-vertex): a unit quad centered at the origin, [-0.5, 0.5]^2.
// Slot 1 (per-instance): GpuInstance { pos: vec2, scale: f32,
//   color: u32 (packed RGBA8), prev_pos: vec2 } — 24 B stride.
//
// Vertex transform: quad_corner * instance.scale +
// mix(instance.prev_pos, instance.pos, camera.alpha) — the GPU-side fixed-step
// interpolation (Phase 20.1 D1) — then through the camera's world->NDC matrix.
// The packed color is unpacked component-wise.
// Fragment: outputs the instance color; quads are rounded into dots by discarding
// fragments outside the inscribed circle (the plan's "round dot" note).

struct Camera {
    view_proj: mat4x4<f32>,
    // Interpolation alpha in [0, 1) (1.0 = snap). The struct size rounds to 80
    // (uniform align(16)); the host mirror pads explicitly (camera.rs).
    alpha: f32,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    // Per-vertex unit-quad corner in [-0.5, 0.5]^2 (slot 0).
    @location(0) corner: vec2<f32>,
    // Per-instance fields (slot 1). location(1) is intentionally unused so the
    // instance attributes start at a distinct base from the quad's location(0).
    @location(2) inst_pos: vec2<f32>,
    @location(3) inst_scale: f32,
    @location(4) inst_color: u32,
    // Position after the second-to-last substep (offset 16, Phase 20.1 D4).
    @location(5) inst_prev_pos: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Local quad coordinate in [-0.5, 0.5]^2, used for the round-dot discard.
    @location(1) local: vec2<f32>,
};

fn unpack_rgba8(packed: u32) -> vec4<f32> {
    let r = f32(packed & 0xffu) / 255.0;
    let g = f32((packed >> 8u) & 0xffu) / 255.0;
    let b = f32((packed >> 16u) & 0xffu) / 255.0;
    let a = f32((packed >> 24u) & 0xffu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // GPU-side fixed-step interpolation (Phase 20.1 D1): render the instance
    // between its last two substep positions at the frame's alpha.
    let world = in.corner * in.inst_scale + mix(in.inst_prev_pos, in.inst_pos, camera.alpha);
    out.clip_pos = camera.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.color = unpack_rgba8(in.inst_color);
    out.local = in.corner;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Round the quad into a dot: discard fragments outside the inscribed circle.
    // `local` is in [-0.5, 0.5]^2, so the circle radius is 0.5 (radius^2 = 0.25).
    let dist_sq = dot(in.local, in.local);
    if (dist_sq > 0.25) {
        discard;
    }
    return in.color;
}
