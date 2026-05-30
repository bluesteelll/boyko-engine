//! Per-entity GPU record uploaded to the instance vertex buffer (plan §5.1).
//!
//! Wave 2 uses the **16 B packed** layout (the plan's §5.1 "Alternative" /
//! Open Question 2). Rationale: it halves upload bandwidth and VRAM versus the
//! 32 B float layout (1M instances = 16 MB instead of 32 MB), and color as a
//! packed `RGBA8` `u32` is unpacked cheaply in the vertex shader. The layout has
//! NO implicit padding — verified by the const-asserts below.
//!
//! The 32 B float layout (`{ pos:[f32;2], size:f32, _pad0:f32, color:[f32;4] }`)
//! is the documented fallback if color banding becomes visible at the target N
//! (plan §5.1). Switching means changing the fields here, the const-asserts, the
//! `VertexBufferLayout` attributes in `mod.rs`, and the matching `@location`
//! inputs in `shader.wgsl`.

use bytemuck::{Pod, Zeroable};

use boyko_macros::Component;

/// One instanced quad on the GPU. `#[repr(C)]` pins field order so the WGSL
/// instance-buffer attribute offsets stay in lockstep with this layout.
///
/// Fields map to `shader.wgsl` instance attributes:
/// - `pos`   -> `@location(2)` (`vec2<f32>`), quad center in world space
/// - `scale` -> `@location(3)` (`f32`), half-extent of the square
/// - `color` -> `@location(4)` (`u32`), packed `RGBA8` (byte 0 = R … byte 3 = A;
///   unpacked component-wise in the shader)
///
/// Wave 3: this is also a boyko `Component`, stored in the sim archetype next to
/// `Position`/`Velocity`. The `Component` derive is a pure marker (it only
/// assigns a `ComponentId` — no fields, no layout change), so it coexists with
/// `bytemuck::Pod` and the column stays a valid GPU instance array uploaded
/// directly via `for_each_chunk` + `cast_slice` (the headline zero-copy path,
/// plan D2/D3/§9 G1).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuInstance {
    /// Quad center in world coordinates.
    pub pos: [f32; 2],
    /// Half-extent of the (square) quad in world units.
    pub scale: f32,
    /// Packed `RGBA8` color. Byte 0 = R, byte 1 = G, byte 2 = B, byte 3 = A.
    pub color: u32,
}

/// Expected size of [`GpuInstance`] in bytes (16 B packed layout).
pub const GPU_INSTANCE_SIZE: usize = 16;

// The whole instancing strategy depends on this exact size/alignment: the vertex
// buffer stride and every WGSL attribute offset are derived from it. A silent
// layout change (e.g. an added field, or padding from a non-Pod field) must fail
// the build, not corrupt the draw (plan §11.4).
const _: () = assert!(size_of::<GpuInstance>() == GPU_INSTANCE_SIZE);
const _: () = assert!(align_of::<GpuInstance>() == 4);

impl GpuInstance {
    /// Builds an instance from a world position, half-extent, and `RGBA8` color
    /// components. Packs color little-endian (R in the low byte) to match the
    /// shader's manual unpack.
    #[inline]
    pub const fn new(pos: [f32; 2], scale: f32, rgba: [u8; 4]) -> Self {
        let color = (rgba[0] as u32)
            | ((rgba[1] as u32) << 8)
            | ((rgba[2] as u32) << 16)
            | ((rgba[3] as u32) << 24);
        Self { pos, scale, color }
    }
}
