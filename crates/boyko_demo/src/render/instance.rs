//! Per-entity GPU record uploaded to the instance vertex buffer (plan §5.1,
//! Phase 20.1 D1/D4).
//!
//! The layout is the Phase-20.1 **24 B interpolated** record: the original 16 B
//! packed fields (`pos`/`scale`/`color`) plus a trailing `prev_pos` so the
//! vertex shader can render `mix(prev_pos, pos, alpha)` — display-rate
//! smoothness from a 64 Hz fixed-step sim with zero CPU-side lerp work
//! (PHASE-20.1-PLAN D1). `prev_pos` is APPENDED so every pre-existing attribute
//! offset (locations 2/3/4) is byte-identical (D4). The layout has NO implicit
//! padding — verified by the const-asserts below.
//!
//! ## `prev_pos` writer discipline (D3 — load-bearing invariant)
//!
//! Exactly ONE site shuffles `prev_pos` per substep (`sync_gpu_instance`'s
//! pack, `sim/systems/common.rs`) and exactly ONE site seeds it
//! ([`GpuInstance::new`] at spawn). Every other per-substep writer must use
//! field-granular writes (`inst.color = ...`) or [`GpuInstance::with_prev`] —
//! a full-struct `*inst = GpuInstance::new(...)` inside a per-substep system
//! would silently reset `prev_pos = pos` every substep (snap-to-pos for those
//! rows). See the doc guard on [`GpuInstance::new`].

use bytemuck::{Pod, Zeroable};

use boyko_macros::Component;

/// One instanced quad on the GPU. `#[repr(C)]` pins field order so the WGSL
/// instance-buffer attribute offsets stay in lockstep with this layout.
///
/// Fields map to `shader.wgsl` instance attributes:
/// - `pos`      -> `@location(2)` (`vec2<f32>`), quad center after the LAST substep
/// - `scale`    -> `@location(3)` (`f32`), half-extent of the square
/// - `color`    -> `@location(4)` (`u32`), packed `RGBA8` (byte 0 = R … byte 3 = A;
///   unpacked component-wise in the shader)
/// - `prev_pos` -> `@location(5)` (`vec2<f32>`), quad center after the
///   SECOND-TO-LAST substep — the GPU lerp's other endpoint (Phase 20.1 D1/D4)
///
/// This is also a boyko `Component`, stored in the sim archetype next to
/// `Position`/`Velocity`. The `Component` derive is a pure marker (it only
/// assigns a `ComponentId` — no fields, no layout change), so it coexists with
/// `bytemuck::Pod` and the column stays a valid GPU instance array uploaded
/// directly via `for_each_chunk` + `cast_slice` (the headline zero-copy path,
/// plan D2/D3/§9 G1).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuInstance {
    /// Quad center in world coordinates — position after the LAST substep.
    pub pos: [f32; 2],
    /// Half-extent of the (square) quad in world units.
    pub scale: f32,
    /// Packed `RGBA8` color. Byte 0 = R, byte 1 = G, byte 2 = B, byte 3 = A.
    pub color: u32,
    /// Quad center after the SECOND-TO-LAST substep (Phase 20.1 D1/D2).
    ///
    /// INVARIANT (D3): written ONLY by `sync_gpu_instance`'s per-substep
    /// shuffle and the [`GpuInstance::new`] spawn seed. Downstream writers
    /// (`sync_ball_gpu`, `tint_collided`) use field-granular writes and never
    /// touch this field.
    pub prev_pos: [f32; 2],
}

/// Expected size of [`GpuInstance`] in bytes (24 B interpolated layout: 6 × 4 B,
/// no padding).
pub const GPU_INSTANCE_SIZE: usize = 24;

// The whole instancing strategy depends on this exact size/alignment: the vertex
// buffer stride and every WGSL attribute offset are derived from it. A silent
// layout change (e.g. an added field, or padding from a non-Pod field) must fail
// the build, not corrupt the draw (plan §11.4 / Phase 20.1 G5).
const _: () = assert!(size_of::<GpuInstance>() == GPU_INSTANCE_SIZE);
const _: () = assert!(align_of::<GpuInstance>() == 4);

impl GpuInstance {
    /// Builds an instance from a world position, half-extent, and `RGBA8` color
    /// components, seeding `prev_pos = pos` so a freshly spawned instance
    /// renders pinned at its spawn point under any interpolation alpha.
    ///
    /// **Spawn-seed only — never call inside a per-substep system** (Phase 20.1
    /// D3 / ★R1-1): a `*inst = GpuInstance::new(...)` in a per-substep writer
    /// would silently reset `prev_pos = pos` every substep, snapping those rows
    /// out of the GPU lerp. Per-substep writers use field writes
    /// (`inst.color = ...`) or [`Self::with_prev`].
    #[inline]
    pub const fn new(pos: [f32; 2], scale: f32, rgba: [u8; 4]) -> Self {
        Self {
            pos,
            scale,
            color: Self::pack_rgba8(rgba),
            prev_pos: pos,
        }
    }

    /// Builds an instance with an explicit interpolation pair — the pack
    /// shuffle's constructor (Phase 20.1 D2): `prev_pos` is the previous
    /// substep's packed position, `pos` the current one.
    #[inline]
    pub const fn with_prev(prev_pos: [f32; 2], pos: [f32; 2], scale: f32, rgba: [u8; 4]) -> Self {
        Self {
            pos,
            scale,
            color: Self::pack_rgba8(rgba),
            prev_pos,
        }
    }

    /// Packs `RGBA8` color components little-endian (R in the low byte) to
    /// match the shader's manual unpack. Field-granular color writers
    /// (`inst.color = GpuInstance::pack_rgba8(...)`, Phase 20.1 D3) share this
    /// with the constructors so the packing can never diverge.
    #[inline]
    pub const fn pack_rgba8(rgba: [u8; 4]) -> u32 {
        (rgba[0] as u32)
            | ((rgba[1] as u32) << 8)
            | ((rgba[2] as u32) << 16)
            | ((rgba[3] as u32) << 24)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T1 (Phase 20.1): `new()` seeds `prev_pos == pos` bitwise — the spawn
    /// funnel's "render pinned at the spawn point" guarantee (D1/D8).
    #[test]
    fn new_seeds_prev_pos_equal_to_pos() {
        let inst = GpuInstance::new([1.5, -2.25], 0.6, [10, 20, 30, 255]);
        assert_eq!(inst.pos[0].to_bits(), inst.prev_pos[0].to_bits());
        assert_eq!(inst.pos[1].to_bits(), inst.prev_pos[1].to_bits());
    }

    /// T1: `pack_rgba8` is exactly the packing `new()` applies.
    #[test]
    fn pack_rgba8_matches_new_packing() {
        let rgba = [0x12, 0x34, 0x56, 0x78];
        assert_eq!(GpuInstance::new([0.0, 0.0], 1.0, rgba).color, GpuInstance::pack_rgba8(rgba));
        assert_eq!(GpuInstance::pack_rgba8(rgba), 0x7856_3412);
        assert_eq!(GpuInstance::pack_rgba8([255, 0, 0, 0]), 0x0000_00ff);
    }

    /// T1: `with_prev` places each argument in the right field (prev and pos
    /// are NOT swapped — the lerp endpoints depend on it).
    #[test]
    fn with_prev_field_placement() {
        let inst = GpuInstance::with_prev([1.0, 2.0], [3.0, 4.0], 0.5, [9, 8, 7, 6]);
        assert_eq!(inst.prev_pos, [1.0, 2.0]);
        assert_eq!(inst.pos, [3.0, 4.0]);
        assert_eq!(inst.scale, 0.5);
        assert_eq!(inst.color, GpuInstance::pack_rgba8([9, 8, 7, 6]));
    }
}
