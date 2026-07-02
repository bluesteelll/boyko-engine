//! Pillar B increment B1 — the interpolation-pair dense component
//! [`GpuTransform3D`] (the FIRST production `#[component(storage = "dense")]` type).
//!
//! One per-entity **previous → current** decomposed-TRS pair, packed byte-for-byte
//! into the 96-byte `TransformPair` the B2 interpolation compute pre-pass
//! (`interp_instances.comp.hlsl`) reads at binding 0. The B2 shader documents the
//! contract (its `Trs` / `TransformPair` structs); this is the HOST mirror the
//! shader header refers to ("The host `TransformPair` mirror … pins this layout with
//! const asserts").
//!
//! # Why dense storage
//!
//! Every interpolated instance contributes exactly one 96-byte pair to the compute
//! dispatch's input SSBO, regardless of its archetype. A dense component owns ONE
//! global column that holds every instance across all archetypes and never fragments
//! the archetype space (the Principle-0 "one contiguous buffer for all instances"
//! case — the same rationale as the solver-state / GPU-instance dense columns). The
//! per-substep pack walks that column; the pair-emitting gather scatters it into a
//! draw-ordered ring. A dense type is always [`ResidencyKind::Cpu`] — the pack + the
//! gather are host writers; the column → GPU upload is a separate `cast_slice` (the
//! `Gpu3dInstance` / `InstanceModelCol` discipline).
//!
//! [`ResidencyKind::Cpu`]: boyko_ecs::ecs::core::component::component_registry::ResidencyKind::Cpu
//!
//! # Principle 0
//!
//! The `GpuTransform3D` column IS the interpolation-pair source — there is NO
//! parallel `std::Vec` mirror. The source of truth is the decomposed
//! [`Transform`](boyko_scene::Transform) column;
//! [`pack_gpu_transforms`](crate::gpu_transform_pack::pack_gpu_transforms) is the
//! EXPLICIT per-substep pack (one `Transform` read + one prev-shuffle + one packed
//! write per row, alloc-free), symmetric with the demo's 2D `sync_gpu_instance`.

use boyko_macros::Component;
use boyko_scene::Transform;
use bytemuck::{Pod, Zeroable};

/// One decomposed transform, packed to byte-match the B2 shader's `Trs` struct.
///
/// All-`float4` packing (xyz + one pad lane per vector) keeps the record 16-byte
/// aligned and std430-straddle-free: `pos` at byte 0, `rot` at 16, `scale` at 32 — a
/// 48-byte TRS. The `.w` pad lane of `pos` / `scale` is UNUSED (written `0.0` by
/// [`GpuTransform3D::from_transform`], read by nothing — the shader indexes `.x/.y/.z`
/// / the quaternion `.xyzw`).
///
/// # Layout (the B2 contract)
///
/// * `pos` — `[f32; 4]` at byte 0: `xyz` = translation, `w` = pad.
/// * `rot` — `[f32; 4]` at byte 16: `xyzw` = unit quaternion (the engine's glTF/GPU
///   `(x, y, z, w)` convention — [`boyko_math::Quat`](boyko_math)'s field order, a
///   direct byte copy).
/// * `scale` — `[f32; 4]` at byte 32: `xyz` = per-axis scale, `w` = pad.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct TrsPacked {
    /// Translation in `xyz`; `w` is an unused pad lane (written `0.0`). Byte 0.
    pub pos: [f32; 4],
    /// Unit quaternion in `xyzw` (the engine's `(x, y, z, w)` convention). Byte 16.
    pub rot: [f32; 4],
    /// Per-axis scale in `xyz`; `w` is an unused pad lane (written `0.0`). Byte 32.
    pub scale: [f32; 4],
}

/// One previous → current decomposed-TRS pair — the interpolation datum the B2
/// compute pre-pass reads as a `TransformPair` (96 B: `prev` at byte 0, `curr` at
/// 48).
///
/// `#[component(storage = "dense")]` selects the dense (non-fragmenting) storage
/// backend: the component is EXCLUDED from every archetype signature and owns ONE
/// global column across all archetypes. `#[repr(C)]` pins the 96-byte stride the B2
/// input SSBO declares; the `bytemuck::Pod` derive makes the pair-emitting gather's
/// `cast_slice` scatter into the mapped SSBO ring sound.
///
/// # The prev-shuffle discipline (D3 — single shuffle site)
///
/// `curr` is this substep's pose; `prev` is the PRIOR substep's `curr`. The GPU lerp
/// `mix(prev, curr, alpha)` therefore always spans exactly one substep. The
/// [`pack_gpu_transforms`](crate::gpu_transform_pack::pack_gpu_transforms) system is
/// the SOLE per-substep writer of `prev` — it shuffles `prev = old curr` BEFORE
/// writing `curr = from(Transform)`. A spawn seeds `prev == curr` bitwise (the
/// no-teleport rule) via [`from_transform`](Self::from_transform).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
#[component(storage = "dense")]
pub struct GpuTransform3D {
    /// The PREVIOUS substep's packed pose — the lerp's rear endpoint. Byte 0.
    pub prev: TrsPacked,
    /// The CURRENT substep's packed pose — the lerp's front endpoint. Byte 48.
    pub curr: TrsPacked,
}

/// The byte size of one [`TrsPacked`] — the B2 shader's `Trs` stride (48 B).
pub const TRS_PACKED_BYTES: usize = 48;

/// The byte size of one [`GpuTransform3D`] — the B2 shader's `TransformPair` stride
/// (96 B). Equals the shader's `sizeof(TransformPair)`; the layout contract is
/// cross-crate, so the size is re-pinned here.
pub const GPU_TRANSFORM3D_BYTES: usize = 96;

// The whole B2 interpolation dispatch depends on these exact sizes / offsets: the
// input SSBO stride + every `prev`/`curr` field offset derive from them. A silent
// layout change (an added field, padding from a non-POD field, a reordered vector)
// must fail the build, not corrupt the interpolated pose — the `Gpu3dInstance` /
// `InstanceModelCol` pin discipline. The offsets mirror the shader's `Trs`
// (`pos`@0, `rot`@16, `scale`@32) and `TransformPair` (`prev`@0, `curr`@48).
const _: () = assert!(size_of::<TrsPacked>() == TRS_PACKED_BYTES);
const _: () = assert!(align_of::<TrsPacked>() == 4);
const _: () = assert!(core::mem::offset_of!(TrsPacked, pos) == 0);
const _: () = assert!(core::mem::offset_of!(TrsPacked, rot) == 16);
const _: () = assert!(core::mem::offset_of!(TrsPacked, scale) == 32);

const _: () = assert!(size_of::<GpuTransform3D>() == GPU_TRANSFORM3D_BYTES);
const _: () = assert!(align_of::<GpuTransform3D>() == 4);
const _: () = assert!(core::mem::offset_of!(GpuTransform3D, prev) == 0);
const _: () = assert!(core::mem::offset_of!(GpuTransform3D, curr) == 48);

impl TrsPacked {
    /// Packs a decomposed [`Transform`] into the B2 `Trs` layout: translation into
    /// `pos.xyz` (`pos.w = 0` pad), the unit quaternion into `rot.xyzw` (the engine's
    /// `(x, y, z, w)` order, a direct field copy), per-axis scale into `scale.xyz`
    /// (`scale.w = 0` pad).
    #[inline]
    pub fn from_transform(t: &Transform) -> Self {
        let p = t.translation;
        let q = t.rotation;
        let s = t.scale;
        Self {
            pos: [p.x, p.y, p.z, 0.0],
            rot: [q.x, q.y, q.z, q.w],
            scale: [s.x, s.y, s.z, 0.0],
        }
    }
}

impl GpuTransform3D {
    /// Builds a fresh pair from a decomposed [`Transform`], seeding `prev == curr`
    /// BITWISE — the no-teleport rule (D1 seed site).
    ///
    /// A row spawned this frame has no prior substep to interpolate from, so both
    /// endpoints are its current pose: `mix(prev, curr, alpha) == curr` for every
    /// `alpha`, so the interpolation is a no-op until the first
    /// [`pack_gpu_transforms`](crate::gpu_transform_pack::pack_gpu_transforms) shuffle
    /// gives `prev` a real prior pose. This mirrors the demo's `GpuInstance::new`
    /// seed (`prev_pos == pos`), lifted to the 3D decomposed pose.
    #[inline]
    pub fn from_transform(t: &Transform) -> Self {
        let packed = TrsPacked::from_transform(t);
        Self {
            prev: packed,
            curr: packed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use boyko_math::{Quat, Vec3};
    use boyko_scene::Transform;
    use bytemuck::bytes_of;

    /// A non-trivial decomposed transform whose every lane is distinct, so a
    /// mis-mapped field (a swapped quat lane, a translation-into-scale slip) is
    /// caught by value.
    fn sample_transform() -> Transform {
        Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            // A NON-identity, non-unit-length input so the (x, y, z, w) mapping is
            // observable per-lane (the packer does not normalize — a direct copy).
            rotation: Quat::new(0.1, 0.2, 0.3, 0.4),
            scale: Vec3::new(5.0, 6.0, 7.0),
        }
    }

    /// The byte-mirror of the B2 shader's `Trs` contract: reading the 48 packed bytes
    /// as `f32`s at the documented offsets recovers the source TRS, with the `.w` pad
    /// lanes of `pos` / `scale` zeroed. Offsets are the shader's `pos`@0, `rot`@16,
    /// `scale`@32.
    #[test]
    fn trs_packed_byte_mirrors_the_shader_offsets() {
        let t = sample_transform();
        let packed = TrsPacked::from_transform(&t);
        let bytes = bytes_of(&packed);
        assert_eq!(bytes.len(), TRS_PACKED_BYTES);

        let f = |off: usize| f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());

        // pos @ 0: xyz = translation, w = 0 pad.
        assert_eq!([f(0), f(4), f(8)], [1.0, 2.0, 3.0], "pos.xyz = translation");
        assert_eq!(f(12), 0.0, "pos.w is the unused pad lane (0)");
        // rot @ 16: xyzw = the quaternion in engine (x, y, z, w) order.
        assert_eq!([f(16), f(20), f(24), f(28)], [0.1, 0.2, 0.3, 0.4], "rot.xyzw");
        // scale @ 32: xyz = scale, w = 0 pad.
        assert_eq!([f(32), f(36), f(40)], [5.0, 6.0, 7.0], "scale.xyz");
        assert_eq!(f(44), 0.0, "scale.w is the unused pad lane (0)");
    }

    /// The pair's `prev` @ 0 and `curr` @ 48 byte-mirror the shader's `TransformPair`,
    /// and a fresh pair seeds `prev == curr` BITWISE — the no-teleport seed rule (D1).
    #[test]
    fn pair_seeds_prev_equal_curr_bitwise_and_mirrors_offsets() {
        let t = sample_transform();
        let g = GpuTransform3D::from_transform(&t);

        // Seed rule: prev and curr are the SAME 48 bytes (a still, freshly-spawned row
        // interpolates to `curr` at every alpha — no teleport).
        assert_eq!(
            bytes_of(&g.prev),
            bytes_of(&g.curr),
            "a freshly built pair must have prev == curr bitwise (the seed rule)"
        );

        // TransformPair offsets: prev @ 0, curr @ 48, each a full Trs.
        let bytes = bytes_of(&g);
        assert_eq!(bytes.len(), GPU_TRANSFORM3D_BYTES);
        let want = TrsPacked::from_transform(&t);
        assert_eq!(&bytes[0..48], bytes_of(&want), "prev @ 0 is the packed TRS");
        assert_eq!(&bytes[48..96], bytes_of(&want), "curr @ 48 is the packed TRS");
    }
}
