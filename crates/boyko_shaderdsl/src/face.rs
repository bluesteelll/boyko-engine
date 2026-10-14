//! RD6 / D-DS — the facing leaves every opaque mesh producer orients its normal through.
//!
//! glTF 2.0 "Double Sided": "When this value is true, back-face culling is disabled, triangles
//! are rendered from both sides, and the normal vectors of the back-facing triangles are
//! negated" — and "the determinant of the node's global transform defines the winding order".
//! A producer therefore negates its FINAL shading normal exactly when the material is
//! double-sided and the fragment shows the triangle's authored back:
//!
//! ```text
//! flip = DOUBLE_SIDED ∧ ¬(front_ccw ⊕ MIRRORED)
//! ```
//!
//! where `front_ccw` is the rasterizer's own facing under `VK_FRONT_FACE_COUNTER_CLOCKWISE`
//! (`SV_IsFrontFace` on the raster paths, [`vb_screen_front_ccw_body`] on the Visibility-Buffer
//! re-fetch) and `MIRRORED` is the instance's negative-determinant bit (a mirror reverses every
//! triangle's screen winding, so the authored front shows as back-facing).
//!
//! Both leaves are authored ONCE generic over [`Cf`] and instantiated two ways (the campaign
//! pattern): `EvalCf` — the host oracle [`mesh_face_flip`] / [`vb_screen_front_ccw`] the tests
//! pin; `EmitCf` — the HLSL the `emit_mesh_face` generator splices into the three producer
//! sources (`forward_opaque.fs.hlsl`, `gbuffer_mrt.fs.hlsl`, `vb_geom_fetch.hlsli`).
//!
//! The bit layout is one spelling: these constants equal `boyko_rhi::enums::MESH_FACE_*` (this
//! crate has no `boyko_rhi` edge; `boyko_rhi_vulkan/tests/mesh_face_bits_agreement.rs` pins the
//! three spellings — `boyko_rhi`, this module, the generated HLSL — equal).

use core::cell::Cell;

use crate::cf::{Cf, EvalCf, Flow};
use crate::scalar::FieldScalar;

/// The bit index of the double-sided face bit in a carrier word (`1 << 2`).
pub const MESH_FACE_DOUBLE_SIDED_SHIFT: u32 = 2;
/// The bit index of the mirrored face bit in a carrier word (`1 << 3`).
pub const MESH_FACE_MIRRORED_SHIFT: u32 = 3;
/// "This draw's material is glTF `doubleSided`" — `1 << MESH_FACE_DOUBLE_SIDED_SHIFT`.
pub const MESH_FACE_DOUBLE_SIDED: u32 = 1 << MESH_FACE_DOUBLE_SIDED_SHIFT;
/// "This instance's model transform has a negative determinant" — `1 << MESH_FACE_MIRRORED_SHIFT`.
pub const MESH_FACE_MIRRORED: u32 = 1 << MESH_FACE_MIRRORED_SHIFT;

/// `1u` when the fragment's shading normal must be negated, else `0u` — see the module doc.
///
/// `front_ccw` is `1u` / `0u` (the rasterizer's counter-clockwise facing); `face` is the
/// carrier word (the raster push word or `VbInstanceRow::flags`) — only its two face bits are
/// read, so the instanced-arm bit 0 and the VB-indirection bit 1 of the push word pass through.
///
/// ```text
/// uint mirrored = face >> MESH_FACE_MIRRORED_SHIFT;
/// uint hidden = front_ccw ^ mirrored ^ 1u;           // bit 0: the authored back shows
/// return face >> MESH_FACE_DOUBLE_SIDED_SHIFT & hidden & 1u;
/// ```
pub fn mesh_face_flip_body<C: Cf>(front_ccw: C::Uint, face: C::Uint, ret: &C::RetCell) -> Flow {
    let mirrored = C::temp_uint(
        "mirrored",
        C::shr_u(face, C::named_uint_val("MESH_FACE_MIRRORED_SHIFT", MESH_FACE_MIRRORED_SHIFT)),
    );
    let hidden = C::temp_uint("hidden", C::uxor(C::uxor(front_ccw, mirrored), C::uint_lit(1)));
    C::ret(
        ret,
        C::and_u(
            C::and_u(
                C::shr_u(face, C::named_uint_val("MESH_FACE_DOUBLE_SIDED_SHIFT", MESH_FACE_DOUBLE_SIDED_SHIFT)),
                hidden,
            ),
            C::uint_lit(1),
        ),
    )
}

/// `1u` when the screen-space triangle `(vx[i], vy[i])` (pixel coordinates, origin top-left,
/// y down — the positive-height viewport every raster path uses) is FRONT-facing under
/// `VK_FRONT_FACE_COUNTER_CLOCKWISE`, else `0u`.
///
/// The Vulkan rule: `a = −½ Σ (xᵢ·yᵢ₊₁ − xᵢ₊₁·yᵢ)`, front ⇔ `a > 0` — "any triangle which is not
/// front-facing is back-facing, including zero-area triangles". With `area2 = Σ (xᵢ·yᵢ₊₁ −
/// xᵢ₊₁·yᵢ) = −2a`, front ⇔ `area2 < 0`:
///
/// ```text
/// float area2 = vx.x * vy.y - vx.y * vy.x + (vx.y * vy.z - vx.z * vy.y) + (vx.z * vy.x - vx.x * vy.z);
/// return (area2 < 0.0) ? 1u : 0u;
/// ```
pub fn vb_screen_front_ccw_body<C: Cf>(vx: [C::Scalar; 3], vy: [C::Scalar; 3], ret: &C::RetCell) -> Flow {
    let e01 = vx[0].mul(vy[1]).sub(vx[1].mul(vy[0]));
    let e12 = vx[1].mul(vy[2]).sub(vx[2].mul(vy[1]));
    let e20 = vx[2].mul(vy[0]).sub(vx[0].mul(vy[2]));
    let area2 = C::temp_float("area2", e01.add(e12).add(e20));
    C::ret(ret, C::select_uint(area2.lt(C::Scalar::lit(0.0)), C::uint_lit(1), C::uint_lit(0)))
}

/// The host oracle of [`mesh_face_flip_body`] (the `EvalCf` instance).
#[inline]
pub fn mesh_face_flip(front_ccw: u32, face: u32) -> u32 {
    let cell = Cell::new(u32::MAX);
    let _ = mesh_face_flip_body::<EvalCf>(front_ccw, face, &cell);
    cell.get()
}

/// The host oracle of [`vb_screen_front_ccw_body`] (the `EvalCf` instance).
#[inline]
pub fn vb_screen_front_ccw(vx: [f32; 3], vy: [f32; 3]) -> u32 {
    let cell = Cell::new(u32::MAX);
    let _ = vb_screen_front_ccw_body::<EvalCf>(vx, vy, &cell);
    cell.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vb::vb_near_clip_body;

    /// H2: the eight-row truth table — flip ⇔ double-sided ∧ the authored back shows, where the
    /// authored back shows ⇔ ¬(front_ccw ⊕ mirrored); the push word's bits 0 and 1 are ignored.
    #[test]
    fn mesh_face_flip_truth_table() {
        for ds in [false, true] {
            for mirrored in [false, true] {
                for front in [0u32, 1] {
                    let face = u32::from(ds) * MESH_FACE_DOUBLE_SIDED + u32::from(mirrored) * MESH_FACE_MIRRORED;
                    let authored_front_visible = (front == 1) != mirrored;
                    let want = u32::from(ds && !authored_front_visible);
                    for low_bits in [0u32, 1, 2, 3] {
                        assert_eq!(
                            mesh_face_flip(front, face | low_bits),
                            want,
                            "ds={ds} mirrored={mirrored} front_ccw={front} low_bits={low_bits}"
                        );
                    }
                }
            }
        }
    }

    /// The Vulkan polygon area in f64 (the spec formula, independent of the leaf's grouping).
    fn spec_front(vx: [f32; 3], vy: [f32; 3]) -> u32 {
        let (x, y) = (vx.map(f64::from), vy.map(f64::from));
        let mut s = 0.0;
        for i in 0..3 {
            let j = (i + 1) % 3;
            s += x[i] * y[j] - x[j] * y[i];
        }
        u32::from(-0.5 * s > 0.0)
    }

    /// A tiny deterministic generator (xorshift32) — no third-party RNG.
    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 >> 8) as f32 / (1u32 << 24) as f32
        }
    }

    /// H2: the leaf agrees with the spec's area rule on 10⁴ seeded pixel-space triangles in both
    /// windings (each triangle and its reverse disagree), on zero-area triangles (never front),
    /// and on near-degenerate ones whose f32 area is still clearly signed.
    #[test]
    fn vb_screen_front_ccw_matches_the_spec_area_rule() {
        let mut rng = Rng(0x9E37_79B9);
        let mut fronts = 0;
        for _ in 0..10_000 {
            let vx = [rng.next() * 2560.0, rng.next() * 2560.0, rng.next() * 2560.0];
            let vy = [rng.next() * 1440.0, rng.next() * 1440.0, rng.next() * 1440.0];
            let (rx, ry) = ([vx[0], vx[2], vx[1]], [vy[0], vy[2], vy[1]]);
            let area = (f64::from(vx[1] - vx[0]) * f64::from(vy[2] - vy[0])
                - f64::from(vx[2] - vx[0]) * f64::from(vy[1] - vy[0]))
            .abs();
            if area < 1.0 {
                continue; // sub-pixel slivers: covered by the degenerate rows below
            }
            let f = vb_screen_front_ccw(vx, vy);
            assert_eq!(f, spec_front(vx, vy), "leaf vs spec on {vx:?} {vy:?}");
            assert_eq!(vb_screen_front_ccw(rx, ry), 1 - f, "the reversed winding flips the facing");
            fronts += f;
        }
        assert!((4_000..=6_000).contains(&fronts), "anti-vacuity: both windings sampled ({fronts} front)");
        // Zero area: collinear and coincident vertices are back-facing (spec).
        assert_eq!(vb_screen_front_ccw([0.0, 10.0, 20.0], [0.0, 10.0, 20.0]), 0);
        assert_eq!(vb_screen_front_ccw([5.0, 5.0, 5.0], [7.0, 7.0, 7.0]), 0);
        // Near-degenerate but clearly signed: a 1000-px base, 0.25-px height.
        assert_eq!(vb_screen_front_ccw([0.0, 1000.0, 500.0], [100.0, 100.0, 99.75]), spec_front([0.0, 1000.0, 500.0], [100.0, 100.0, 99.75]));
        assert_eq!(vb_screen_front_ccw([0.0, 500.0, 1000.0], [100.0, 99.75, 100.0]), spec_front([0.0, 500.0, 1000.0], [100.0, 99.75, 100.0]));
        // The orientation itself: (0,0) → (0,1) → (1,0) in y-down pixels is counter-clockwise on
        // screen, hence front (the spec's worked convention).
        assert_eq!(vb_screen_front_ccw([0.0, 0.0, 100.0], [0.0, 100.0, 0.0]), 1);
    }

    /// H2: a triangle crossing the near plane keeps its facing through `vb_near_clip` (the
    /// shrink-only clip moves a vertex along its edges): the leaf on the clipped triangle's pixel
    /// coordinates equals the facing of the same triangle placed wholly in front of the viewer.
    #[test]
    fn vb_screen_front_ccw_survives_the_near_clip() {
        let extent = [1920.0f32, 1080.0];
        // Clip-space vertices of one triangle; v2 dips behind the near plane (w <= epsilon).
        let tri = |w2: f32, flip: bool| -> [[f32; 4]; 3] {
            let a = [-0.4, -0.3, 0.5, 1.0];
            let b = [0.4, -0.3, 0.5, 1.0];
            let c = [0.0, 0.5, 0.5, w2];
            if flip { [a, c, b] } else { [a, b, c] }
        };
        let to_px = |v: [f32; 4]| -> (f32, f32) {
            ((v[0] / v[3] * 0.5 + 0.5) * extent[0], (v[1] / v[3] * 0.5 + 0.5) * extent[1])
        };
        for flip in [false, true] {
            let safe = tri(1.0, flip);
            let (s0, s1, s2) = (to_px(safe[0]), to_px(safe[1]), to_px(safe[2]));
            let want = vb_screen_front_ccw([s0.0, s1.0, s2.0], [s0.1, s1.1, s2.1]);
            let crossing = tri(-0.2, flip);
            let clipped = vb_near_clip_body::<f32>(crossing);
            let (c0, c1, c2) = (to_px(clipped[0]), to_px(clipped[1]), to_px(clipped[2]));
            assert_eq!(
                vb_screen_front_ccw([c0.0, c1.0, c2.0], [c0.1, c1.1, c2.1]),
                want,
                "flip={flip}: the near clip must not change the triangle's facing"
            );
        }
    }
}
