// Whole file is `#[cfg(test)]` host-oracle modules: the std collections here are REFERENCE
// models (a `HashSet` used to prove the SSAO dither is decorrelated across pixels), never
// engine state. Compiled out of every shipping build, so no engine path can reach them.
#![allow(clippy::disallowed_types)]

#[cfg(test)]
mod grazing_shadow_tests {
    //! STEP-1 confirmation + STEP-3 regression for the GRAZING-ANGLE SHADOW-ACNE fix.
    //!
    //! The SDF soft-shadow march starts at the surface point `p`, samples
    //! `field_distance(p + L*t)` from `t = SHADOW_MINT`, and returns `0` (occluder hit) as
    //! soon as `d < SHADOW_HIT_EPS`. At a GRAZING angle (the light `L` nearly tangent to the
    //! surface — the lit terminator, `n·L` small but POSITIVE so the point passes the
    //! `SHADOW_NDOTL_EPS` gate) the tangent ray's first samples stay within `~t²/(2R)` of a
    //! curved surface, so `d` reads below `SHADOW_HIT_EPS` and the march FALSE-occludes the
    //! point — the black "flame" acne on the terminator. The fix lifts the march ORIGIN by
    //! `n * SHADOW_NORMAL_BIAS` (applied at the call sites, mirrored host + GPU).
    //!
    //! These tests use the host soft-shadow mirror over a single analytic SPHERE (the same
    //! `sdf_sphere` the GPU `sdf_sphere` mirrors) so they are GPU-free and deterministic.

    use super::super::{SHADOW_HIT_EPS, SHADOW_K, SHADOW_MINT, SHADOW_MINT_STEP, SHADOW_NORMAL_BIAS, SDF_MAX_IT, SDF_SPHERE_CENTER, SDF_SPHERE_RADIUS, SDF_T_MAX, sdf_sphere};
    use crate::goldens::{host_soft_shadow, host_soft_shadow_ranged, sdf_normal};
    use boyko_sdf_math::{v_dot, v_normalize};

    // The march's Lipschitz step divisor — kept in sync with `host_soft_shadow`'s
    // `FIELD_LIPSCHITZ_L` (the committed shader literal). Used ONLY by the inline UNBIASED
    // reference march below (the bias-free reproduction of the pre-fix behavior).
    #[allow(clippy::approx_constant, clippy::excessive_precision)]
    const FIELD_LIPSCHITZ_L: f32 = 1.41421356;

    /// A bit-for-bit copy of the soft-shadow march WITHOUT the normal-offset start bias —
    /// the pre-fix behavior, kept here ONLY to reproduce the grazing acne for STEP 1. It
    /// marches from the RAW surface point `p` (no `n` lift), exactly as the host/GPU march
    /// did before this fix.
    fn unbiased_soft_shadow<F: Fn([f32; 3]) -> f32>(p: [f32; 3], l: [f32; 3], field: &F) -> f32 {
        let mut res = 1.0_f32;
        let mut t = SHADOW_MINT;
        for _ in 0..SDF_MAX_IT {
            let q = [p[0] + l[0] * t, p[1] + l[1] * t, p[2] + l[2] * t];
            let d = field(q);
            res = res.min(SHADOW_K * d / t);
            if d < SHADOW_HIT_EPS {
                return 0.0;
            }
            t += (d / FIELD_LIPSCHITZ_L).max(SHADOW_MINT_STEP);
            if t > SDF_T_MAX {
                break;
            }
        }
        res.clamp(0.0, 1.0)
    }

    /// A lit surface point near the terminator: pick a point on the sphere whose normal
    /// makes a small POSITIVE angle's-cosine with the light (`n·L` small but > 0, so the
    /// point is LIT and passes the `SHADOW_NDOTL_EPS` grazing gate). Returns `(p, n, l)`.
    ///
    /// The light points along +Z. A point near the equator (relative to +Z) has its normal
    /// nearly perpendicular to `L` ⇒ `n·L` small ⇒ grazing. We choose a polar angle so that
    /// `n·L ≈ 0.06` (well inside the lit half, clearly grazing).
    fn grazing_lit_point() -> ([f32; 3], [f32; 3], [f32; 3]) {
        let l = v_normalize([0.0, 0.0, 1.0]);
        // n·L = cos(theta) where theta is the angle of the surface point off the +Z pole.
        // theta ≈ 86.6° ⇒ cos ≈ 0.06 (grazing but lit).
        let cos_t = 0.06_f32;
        let sin_t = (1.0 - cos_t * cos_t).sqrt();
        // The surface point on the unit-radius sphere (radius SDF_SPHERE_RADIUS): the normal
        // direction times the radius, offset by the center.
        let dir = [sin_t, 0.0, cos_t];
        let p = [
            SDF_SPHERE_CENTER[0] + dir[0] * SDF_SPHERE_RADIUS,
            SDF_SPHERE_CENTER[1] + dir[1] * SDF_SPHERE_RADIUS,
            SDF_SPHERE_CENTER[2] + dir[2] * SDF_SPHERE_RADIUS,
        ];
        // The analytic normal via the SAME central-difference gradient the marcher uses.
        let n = sdf_normal(p);
        (p, n, l)
    }

    /// STEP 1: the UNBIASED march FALSE-occludes a lit grazing point (`res ≈ 0`). This is
    /// the acne. If this assert ever fails, the diagnosis is wrong — STOP and re-investigate.
    #[test]
    fn step1_unbiased_march_false_occludes_grazing_terminator() {
        let (p, n, l) = grazing_lit_point();
        // The point is LIT (passes the grazing gate) — this is the precondition.
        let ndotl = v_dot(n, l);
        assert!(
            ndotl > 0.0,
            "test setup bug: the grazing point must be LIT (n·L > 0), got {ndotl}"
        );
        let field = |q: [f32; 3]| sdf_sphere(q);
        let res = unbiased_soft_shadow(p, l, &field);
        assert!(
            res < 1.0e-3,
            "expected the UNBIASED march to FALSE-occlude the lit grazing point (acne, \
             res ≈ 0), got res = {res} — the diagnosis may be wrong"
        );
    }

    /// STEP 3: the BIASED march (the host mirror, called with `p + n*SHADOW_NORMAL_BIAS`,
    /// exactly as `host_shade` now calls it) keeps the lit grazing point LIT (`res > 0`):
    /// the acne is GONE.
    #[test]
    fn step3_normal_bias_clears_grazing_acne() {
        let (p, n, l) = grazing_lit_point();
        let pb = [
            p[0] + n[0] * SHADOW_NORMAL_BIAS,
            p[1] + n[1] * SHADOW_NORMAL_BIAS,
            p[2] + n[2] * SHADOW_NORMAL_BIAS,
        ];
        let field = |q: [f32; 3]| sdf_sphere(q);
        let res = host_soft_shadow(pb, n, l, &field);
        assert!(
            res > 0.1,
            "the normal-offset bias must keep the lit grazing point LIT (res > 0), got {res}"
        );
    }

    /// STEP 3 (ranged): the ranged host mirror — the resolve's per-light march — is ALSO
    /// freed from the grazing acne by the same bias.
    #[test]
    fn step3_normal_bias_clears_grazing_acne_ranged() {
        let (p, n, l) = grazing_lit_point();
        let pb = [
            p[0] + n[0] * SHADOW_NORMAL_BIAS,
            p[1] + n[1] * SHADOW_NORMAL_BIAS,
            p[2] + n[2] * SHADOW_NORMAL_BIAS,
        ];
        let field = |q: [f32; 3]| sdf_sphere(q);
        let res = host_soft_shadow_ranged(pb, n, l, SDF_T_MAX, &field);
        assert!(
            res > 0.1,
            "the ranged normal-offset bias must keep the lit grazing point LIT, got {res}"
        );
    }
}

#[cfg(test)]
mod p0a_tests {
    //! Host-side (GPU-free) verification of the P0a substrate: the extent/camera
    //! push-constant layout and the extent-aware golden mirror. The GPU half (the
    //! shader actually rendering ortho 64×64 bit-exact / a 1080p perspective frame)
    //! is the tester's RTX-3060 oracle; these assert the CPU contract those goldens
    //! rely on (the host const-assert mirror + the bit-exact ortho fall-through).

    use super::super::{CAM_MODE_ORTHO, CAM_MODE_PERSPECTIVE, COMPOSITE_PUSH_CONSTANT_BYTES, CompositeCamera, CompositePushConstants, MESH_DEPTH_CLEAR, SDF_IMG_H, SDF_IMG_W, SdfEdit, sdf_op};
    use crate::goldens::{golden_composite_pixel, golden_composite_pixel_ex};

    /// The rung-9/10 "crater" CSG scene, reused so the golden parity check runs over
    /// a non-trivial field (a base sphere with a smaller sphere subtracted).
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    /// The extent-aware golden at `(SDF_IMG_W, SDF_IMG_H)` + ORTHO must be BIT-EXACT
    /// to the legacy `golden_composite_pixel` over the whole 64×64 image (the
    /// rung-8..11 contract — same extent → same rays → same pixels).
    #[test]
    fn ortho_64x64_is_bit_identical_to_legacy_golden() {
        let edits = crater();
        // A mix of covered (finite depth) and uncovered (clear) pixels.
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let md = depths[((px + py) as usize) % depths.len()];
                let legacy = golden_composite_pixel(&edits, md, px, py);
                let ex = golden_composite_pixel_ex(
                    &edits,
                    md,
                    px,
                    py,
                    SDF_IMG_W,
                    SDF_IMG_H,
                    CompositeCamera::Ortho,
                );
                assert_eq!(legacy, ex, "ortho mirror diverged at ({px},{py}) depth {md}");
            }
        }
    }

    /// `CompositePushConstants::ortho` keeps `count == w*h`, ORTHO mode, zeroed
    /// camera basis, and the 80-byte size the pipeline must declare.
    #[test]
    fn ortho_push_constants_shape() {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        assert_eq!(pc.count, SDF_IMG_W * SDF_IMG_H);
        assert_eq!(pc.img_w, SDF_IMG_W);
        assert_eq!(pc.img_h, SDF_IMG_H);
        assert_eq!(pc.camera_mode, CAM_MODE_ORTHO);
        assert_eq!(pc.cam_eye, [0.0; 4]);
        assert_eq!(pc.as_bytes().len(), COMPOSITE_PUSH_CONSTANT_BYTES as usize);
        assert_eq!(COMPOSITE_PUSH_CONSTANT_BYTES, 80);
    }

    /// `CompositePushConstants::perspective` derives `tan(fovY/2)` + aspect and packs
    /// the basis into the documented `float4` slots; the byte view is 80 bytes.
    #[test]
    fn perspective_push_constants_layout() {
        let fov_y = core::f32::consts::FRAC_PI_2; // 90°
        let pc = CompositePushConstants::perspective(
            [0.0, 0.0, 3.0],   // eye
            [0.0, 0.0, -1.0],  // forward
            [1.0, 0.0, 0.0],   // right
            [0.0, 1.0, 0.0],   // up
            fov_y,
            1920,
            1080,
        );
        assert_eq!(pc.camera_mode, CAM_MODE_PERSPECTIVE);
        assert_eq!(pc.count, 1920 * 1080);
        assert_eq!(pc.cam_eye, [0.0, 0.0, 3.0, 0.0]);
        // forward.w = tan(45°) = 1, right.w = aspect = 1920/1080.
        assert!((pc.cam_forward[3] - 1.0).abs() < 1e-5);
        assert!((pc.cam_right[3] - (1920.0_f32 / 1080.0)).abs() < 1e-6);
        assert_eq!(pc.as_bytes().len(), 80);
    }

    /// Perspective ray-gen sanity: the CENTER pixel of a forward-looking camera must
    /// shoot a ray ≈ the forward axis from the eye (the field eval downstream is the
    /// same deterministic mirror, so this isolates the additive ray-gen).
    #[test]
    fn perspective_center_ray_is_forward() {
        let edits = crater();
        // A small extent; we only need the geometric ray, not a full render.
        let (w, h) = (64u32, 64u32);
        let eye = [0.0_f32, 0.0, 3.0];
        let camera = CompositeCamera::Perspective {
            eye,
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            tan_half_fov: (core::f32::consts::FRAC_PI_2 * 0.5).tan(), // 45°
            aspect: 1.0,
        };
        // The center pixel (px=py=32) → ndc ≈ 0 → dir ≈ forward → hits the sphere at
        // the origin from the +Z eye (no mesh: clear depth). A miss would be the dark
        // background; a hit is the warm lit color — distinguish by the red channel.
        let center = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, w / 2, h / 2, w, h, camera);
        let red = center & 0xFF;
        assert!(
            red > 60,
            "center perspective ray must hit the lit sphere (warm red), got 0x{center:08X}"
        );
        // A corner pixel shoots wide and should MISS → background (low red).
        let corner = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, 0, 0, w, h, camera);
        let corner_red = corner & 0xFF;
        assert!(
            corner_red < red,
            "corner perspective ray should miss (darker) vs center: corner 0x{corner:08X} center 0x{center:08X}"
        );
    }
}

#[cfg(test)]
mod p4b_tests {
    //! Render P4b HOST conservative-invariant suite — the CPU proof that the coarse
    //! cull is CONSERVATIVE (a hole = the worst bug) BEFORE the GPU golden runs. The
    //! five proofs (docs/RENDER-P4-DESIGN.md):
    //!   (a) EXHAUSTIVE ortho: every tile, all 64 fine-pixel footprint corners within
    //!       `ortho_cone_radius` of the tile-center axis (the exact composite_ray u/v).
    //!   (b) perspective: every tile's 4 corner outer-edge dirs within
    //!       `perspective_alpha_tile` (enclosure with margin).
    //!   (c) randomized {fov, aspect, tile, single sphere/box} with an ANALYTIC first-hit
    //!       oracle: `golden_tile_bound.near_t <= min over in-tile pixels of their
    //!       analytic first-hit` AND `EMPTY => no in-tile pixel hits before mesh`.
    //!   (d) Lipschitz: random points, central-diff `|grad field| <= FIELD_LIPSCHITZ_L`.
    //!   (e) `golden_composite_pixel_culled(coarse_enabled = false)` bit-identical to
    //!       `golden_composite_pixel_ex`.
    //! These reproduce the GPU shader's arithmetic exactly (the host mirror), so a pass
    //! here is a near-proof the GPU cull is conservative too (the GPU golden confirms).

    use boyko_sdf_math::{sdf_edit_list, v_sub};

    use super::super::{ALPHA_MARGIN, CompositeCamera, FIELD_LIPSCHITZ_L, MESH_DEPTH_CLEAR, MESH_DEPTH_T_MAX, SDF_CAM_Z, SDF_EPS, SDF_HALF_EXTENT, SDF_T_MAX, SdfEdit, TILE_FLAG_EMPTY, TILE_SIZE, sdf_op, tile_grid_extent};
    use crate::goldens::{golden_composite_pixel_culled, golden_composite_pixel_ex, golden_tile_bound};

    // --- A tiny deterministic PRNG (splitmix64) so the randomized sweeps are
    //     reproducible (no rand dep; the same mix the serialization fuzz uses). ----------
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        /// A uniform `f32` in `[lo, hi)`.
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // [0,1)
            lo + (hi - lo) * u
        }
        fn range_u32(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next_u64() % ((hi - lo) as u64)) as u32
        }
    }

    /// The exact ORTHO ray-origin XY for a (possibly fractional) fine-pixel sample whose
    /// `(px + 0.5)` is `sx` and `(py + 0.5)` is `sy` — `composite_ray`'s ortho arm.
    fn ortho_origin_xy(sx: f32, sy: f32, w: u32, h: u32) -> [f32; 2] {
        let u = (sx / (w as f32)) * 2.0 - 1.0;
        let v = -((sy / (h as f32)) * 2.0 - 1.0);
        [u * SDF_HALF_EXTENT, v * SDF_HALF_EXTENT]
    }

    /// The exact, normalized PERSPECTIVE ray direction for a fractional sample, the
    /// `composite_ray` perspective arm (used by the enclosure + oracle tests). The
    /// parameters mirror the shader's ray-gen inputs verbatim (grouping them into a
    /// struct would obscure the op-for-op correspondence the test exists to verify).
    #[allow(clippy::too_many_arguments)]
    fn persp_dir(
        sx: f32,
        sy: f32,
        w: u32,
        h: u32,
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        tan_half_fov: f32,
        aspect: f32,
    ) -> [f32; 3] {
        let ndc_x = (sx / (w as f32)) * 2.0 - 1.0;
        let ndc_y = -((sy / (h as f32)) * 2.0 - 1.0);
        let kx = ndc_x * aspect * tan_half_fov;
        let ky = ndc_y * tan_half_fov;
        let dir = [
            forward[0] + right[0] * kx + up[0] * ky,
            forward[1] + right[1] * kx + up[1] * ky,
            forward[2] + right[2] * kx + up[2] * ky,
        ];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        [dir[0] / len, dir[1] / len, dir[2] / len]
    }

    /// The host mirror of `ortho_cone_radius` (private — re-derive for the test). Uses
    /// the LARGER world pixel pitch `min(w,h)` so a non-square ortho extent is enclosed.
    fn ortho_cone_radius_t(w: u32, h: u32) -> f32 {
        core::f32::consts::SQRT_2 * (9.0 / (w.min(h) as f32)) * SDF_HALF_EXTENT
    }

    /// (a) EXHAUSTIVE ortho enclosure: for EVERY tile, EVERY one of the 64 fine pixels'
    /// 4 footprint corners (`(px+0.5) ± 0.5`, `(py+0.5) ± 0.5`) lies within
    /// `ortho_cone_radius` of the tile-center axis (the perpendicular distance in the
    /// ortho XY plane — the cone is a constant-radius cylinder). A corner outside the
    /// cone = a hole. Run at the golden 64×64 extent + a non-multiple extent.
    #[test]
    fn ortho_footprint_corners_within_cone() {
        for &(w, h) in &[(64u32, 64u32), (96u32, 48u32), (70u32, 66u32)] {
            let (tw, th) = tile_grid_extent(w, h);
            let r = ortho_cone_radius_t(w, h);
            let mut checked = 0u64;
            for ty in 0..th {
                for tx in 0..tw {
                    // Tile-center axis XY (`(px_c+0.5) = tx*8 + 4.0`).
                    let axis = ortho_origin_xy(
                        (tx * TILE_SIZE) as f32 + 4.0,
                        (ty * TILE_SIZE) as f32 + 4.0,
                        w,
                        h,
                    );
                    for ly in 0..TILE_SIZE {
                        for lx in 0..TILE_SIZE {
                            let px = tx * TILE_SIZE + lx;
                            let py = ty * TILE_SIZE + ly;
                            // All 4 footprint corners of fine pixel (px,py): the pixel
                            // sample is `(px+0.5, py+0.5)`; its footprint spans ±0.5.
                            for &(ddx, ddy) in &[(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
                                let sx = (px as f32) + 0.5 + ddx;
                                let sy = (py as f32) + 0.5 + ddy;
                                let xy = ortho_origin_xy(sx, sy, w, h);
                                let dx = xy[0] - axis[0];
                                let dy = xy[1] - axis[1];
                                let dist = (dx * dx + dy * dy).sqrt();
                                assert!(
                                    dist <= r,
                                    "ORTHO hole {w}x{h} tile({tx},{ty}) pixel({px},{py}) corner({ddx},{ddy}): \
                                     footprint corner dist {dist} > cone radius {r}"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
            println!("[a] ortho enclosure {w}x{h}: {checked} footprint corners all within cone radius {r}");
        }
    }

    /// (b) Perspective enclosure: for EVERY tile, the 4 corner OUTER-EDGE directions are
    /// within `perspective_alpha_tile` (= the angle the cull uses) of the tile-center
    /// direction. By construction `perspective_alpha_tile` is the MAX of exactly those 4
    /// angles + `ALPHA_MARGIN`, so each must be `<= alpha` with the margin to spare — a
    /// corner whose angle exceeded `alpha` would be a hole. Also asserts the cull's
    /// in-tile pixel-center + footprint-corner dirs are inside the cone (the stronger
    /// enclosure the conservativeness proof's Claim 1 needs).
    #[test]
    fn perspective_corner_dirs_within_alpha() {
        let (w, h) = (1920u32, 1080u32);
        let forward = [0.0_f32, 0.0, -1.0];
        let right = [1.0_f32, 0.0, 0.0];
        let up = [0.0_f32, 1.0, 0.0];
        let fov_y = core::f32::consts::FRAC_PI_2; // 90°
        let tan_half_fov = (fov_y * 0.5).tan();
        let aspect = (w as f32) / (h as f32);
        let (tw, th) = tile_grid_extent(w, h);

        let camera = CompositeCamera::Perspective {
            eye: [0.0, 0.0, 3.0],
            forward,
            right,
            up,
            tan_half_fov,
            aspect,
        };
        // Sample a strided subset of tiles (full grid is 240×135 = 32 400 tiles; every
        // tile's 4 corners + 64 pixel footprints is exhaustive but slow — stride 7
        // covers the grid incl. the convex edge/corner tiles where alpha is largest).
        let mut tiles_checked = 0u64;
        let mut ty = 0;
        while ty < th {
            let mut tx = 0;
            while tx < tw {
                // Recompute `alpha_tile_safe` exactly as the cull does (4 outer corners).
                let cx = (tx * TILE_SIZE) as f32 + 4.0;
                let cy = (ty * TILE_SIZE) as f32 + 4.0;
                let d_center = persp_dir(cx, cy, w, h, forward, right, up, tan_half_fov, aspect);
                let lo_x = (tx * TILE_SIZE) as f32;
                let hi_x = (tx * TILE_SIZE) as f32 + (TILE_SIZE as f32);
                let lo_y = (ty * TILE_SIZE) as f32;
                let hi_y = (ty * TILE_SIZE) as f32 + (TILE_SIZE as f32);
                let mut alpha = 0.0_f32;
                for &(sxp, syp) in &[(lo_x, lo_y), (hi_x, lo_y), (lo_x, hi_y), (hi_x, hi_y)] {
                    let dc = persp_dir(sxp, syp, w, h, forward, right, up, tan_half_fov, aspect);
                    let cos = (d_center[0] * dc[0] + d_center[1] * dc[1] + d_center[2] * dc[2])
                        .clamp(-1.0, 1.0);
                    alpha = alpha.max(cos.acos());
                }
                let alpha_safe = alpha + ALPHA_MARGIN;

                // Every in-tile pixel CENTER + its 4 footprint corners must be inside the
                // cone (angle <= alpha_safe). This is Claim 1 (lateral offset < r(t)).
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let px = tx * TILE_SIZE + lx;
                        let py = ty * TILE_SIZE + ly;
                        if px >= w || py >= h {
                            continue;
                        }
                        for &(ddx, ddy) in
                            &[(0.0, 0.0), (-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)]
                        {
                            let sx = (px as f32) + 0.5 + ddx;
                            let sy = (py as f32) + 0.5 + ddy;
                            let d = persp_dir(sx, sy, w, h, forward, right, up, tan_half_fov, aspect);
                            let cos = (d_center[0] * d[0] + d_center[1] * d[1] + d_center[2] * d[2])
                                .clamp(-1.0, 1.0);
                            let ang = cos.acos();
                            assert!(
                                ang <= alpha_safe,
                                "PERSP hole tile({tx},{ty}) pixel({px},{py}) corner({ddx},{ddy}): \
                                 dir angle {ang} > alpha_safe {alpha_safe}"
                            );
                        }
                    }
                }
                tiles_checked += 1;
                tx += 7;
            }
            ty += 7;
        }
        // Sanity: the cull's `golden_tile_bound` uses the same camera (smoke — it runs).
        let _ = golden_tile_bound(&[], &[MESH_DEPTH_CLEAR; 64], 0, 0, w, h, camera);
        println!("[b] perspective enclosure {w}x{h}: {tiles_checked} tiles, all in-tile pixel/footprint dirs within alpha");
    }

    // --- Analytic first-hit oracles (a LOWER bound on the GPU march's first hit) -------

    /// Analytic ray-sphere first-hit `t` (the smaller non-negative root) or `None`.
    fn ray_sphere(ro: [f32; 3], rd: [f32; 3], c: [f32; 3], r: f32) -> Option<f32> {
        let oc = v_sub(ro, c);
        let b = oc[0] * rd[0] + oc[1] * rd[1] + oc[2] * rd[2];
        let cc = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - r * r;
        let disc = b * b - cc;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let t0 = -b - s;
        let t1 = -b + s;
        if t0 >= 0.0 {
            Some(t0)
        } else if t1 >= 0.0 {
            Some(t1)
        } else {
            None
        }
    }

    /// Analytic ray-AABB (slab) first-hit `t` for a box centered at `c` with half-extents
    /// `h`, or `None`. Returns the entry `t` (>= 0) of the ray through the box.
    fn ray_box(ro: [f32; 3], rd: [f32; 3], c: [f32; 3], h: [f32; 3]) -> Option<f32> {
        let mut t_min = f32::NEG_INFINITY;
        let mut t_max = f32::INFINITY;
        for a in 0..3 {
            let lo = c[a] - h[a];
            let hi = c[a] + h[a];
            if rd[a].abs() < 1e-9 {
                if ro[a] < lo || ro[a] > hi {
                    return None; // parallel + outside the slab.
                }
            } else {
                let inv = 1.0 / rd[a];
                let mut ta = (lo - ro[a]) * inv;
                let mut tb = (hi - ro[a]) * inv;
                if ta > tb {
                    core::mem::swap(&mut ta, &mut tb);
                }
                t_min = t_min.max(ta);
                t_max = t_max.min(tb);
                if t_min > t_max {
                    return None;
                }
            }
        }
        if t_max < 0.0 {
            return None;
        }
        Some(t_min.max(0.0))
    }

    /// (c) Randomized perspective sweep with an analytic first-hit oracle, the C2/C4
    /// proof: for a single sphere or box, `golden_tile_bound.near_t <= the analytic
    /// first-hit of EVERY in-tile pixel that hits` AND `EMPTY => no in-tile pixel hits
    /// before the (deepest) mesh`. The analytic hit is the true Euclidean first contact;
    /// the cull seeding `t = near_t <= it` can never skip a pixel's surface.
    #[test]
    fn randomized_oracle_near_t_le_first_hit() {
        let mut rng = Rng::new(0xC0FF_EE15_600D_5EED);
        let cases = 600;
        let mut checked_hits = 0u64;
        let mut empty_tiles = 0u64;
        let mut nonempty_tiles = 0u64;

        for _ in 0..cases {
            // A random forward-looking perspective camera (eye on +Z, looking -Z, small
            // jitter on the basis kept orthonormal-ish; the cull only needs the cone).
            let fov_y = rng.range(0.6, 1.8); // ~34°..103°
            let (w, h) = (rng.range_u32(40, 160), rng.range_u32(40, 160));
            let aspect = (w as f32) / (h as f32);
            let tan_half_fov = (fov_y * 0.5).tan();
            let eye = [rng.range(-0.3, 0.3), rng.range(-0.3, 0.3), rng.range(2.0, 4.0)];
            let forward = [0.0, 0.0, -1.0];
            let right = [1.0, 0.0, 0.0];
            let up = [0.0, 1.0, 0.0];
            let camera = CompositeCamera::Perspective {
                eye,
                forward,
                right,
                up,
                tan_half_fov,
                aspect,
            };

            // A single primitive (sphere or box) near the origin.
            let is_box = rng.next_u64() & 1 == 0;
            let center = [rng.range(-0.4, 0.4), rng.range(-0.4, 0.4), rng.range(-0.4, 0.4)];
            let (edits, sphere_r, box_h): (Vec<SdfEdit>, f32, [f32; 3]) = if is_box {
                let hx = rng.range(0.15, 0.5);
                let hy = rng.range(0.15, 0.5);
                let hz = rng.range(0.15, 0.5);
                (
                    vec![SdfEdit::box_shape(center, [hx, hy, hz], sdf_op::UNION, 0.0)],
                    0.0,
                    [hx, hy, hz],
                )
            } else {
                let r = rng.range(0.15, 0.5);
                (
                    vec![SdfEdit::sphere(center, r, sdf_op::UNION, 0.0)],
                    r,
                    [0.0; 3],
                )
            };

            // A tile within the grid: half the cases bias toward the image center (where
            // a forward camera sees the origin-centered primitive, so the tile actually
            // looks at the surface — exercising near_t <= first-hit), half are fully
            // random (exercising the EMPTY path on tiles that look away).
            let (tw, th) = tile_grid_extent(w, h);
            let (tx, ty) = if rng.next_u64() & 1 == 0 {
                let cx = tw / 2;
                let cy = th / 2;
                let jx = rng.range_u32(0, 3);
                let jy = rng.range_u32(0, 3);
                (
                    (cx + jx).saturating_sub(1).min(tw - 1),
                    (cy + jy).saturating_sub(1).min(th - 1),
                )
            } else {
                (rng.range_u32(0, tw), rng.range_u32(0, th))
            };

            // No mesh (clear depth everywhere in the tile) so far_t == T_MAX and the
            // oracle is the pure SDF first-hit (the mesh-bound case is exercised by the
            // GPU golden + the EMPTY-with-mesh negative test).
            let tile_depths = [MESH_DEPTH_CLEAR; 64];
            let tb = golden_tile_bound(&edits, &tile_depths, tx, ty, w, h, camera);

            // For every in-tile pixel, the analytic first-hit (the oracle).
            let mut min_first_hit = f32::INFINITY;
            let mut any_hit = false;
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let px = tx * TILE_SIZE + lx;
                    let py = ty * TILE_SIZE + ly;
                    if px >= w || py >= h {
                        continue;
                    }
                    let sx = (px as f32) + 0.5;
                    let sy = (py as f32) + 0.5;
                    let rd = persp_dir(sx, sy, w, h, forward, right, up, tan_half_fov, aspect);
                    let hit = if is_box {
                        ray_box(eye, rd, center, box_h)
                    } else {
                        ray_sphere(eye, rd, center, sphere_r)
                    };
                    if let Some(t_hit) = hit
                        && t_hit <= SDF_T_MAX
                    {
                        any_hit = true;
                        min_first_hit = min_first_hit.min(t_hit);
                    }
                }
            }

            if tb.flags & TILE_FLAG_EMPTY != 0 {
                empty_tiles += 1;
                // EMPTY => no in-tile pixel may hit before the mesh (far_t == T_MAX here).
                // The march hit threshold is EPS, so a sphere-trace records a hit slightly
                // BEFORE the analytic surface; allow the surface to be within EPS of T_MAX.
                assert!(
                    !any_hit || min_first_hit + SDF_EPS >= SDF_T_MAX,
                    "EMPTY tile but an in-tile pixel hits at {min_first_hit} (< T_MAX): \
                     box={is_box} center={center:?} fov={fov_y} {w}x{h} tile({tx},{ty})"
                );
            } else {
                nonempty_tiles += 1;
                if any_hit {
                    checked_hits += 1;
                    // The CORE conservativeness claim: near_t <= every pixel's first hit.
                    // A small EPS tolerance absorbs the cone-entry EPS_COARSE + the fp
                    // step rounding (near_t is recorded AT the cone-entry t, never past it).
                    assert!(
                        tb.near_t <= min_first_hit + 1e-3,
                        "near_t {} > min in-tile first-hit {min_first_hit}: HOLE \
                         box={is_box} center={center:?} fov={fov_y} {w}x{h} tile({tx},{ty})",
                        tb.near_t
                    );
                }
            }
        }
        println!(
            "[c] randomized oracle: {cases} cases, {nonempty_tiles} non-empty ({checked_hits} with hits) + {empty_tiles} EMPTY — near_t <= analytic first-hit, EMPTY => no early hit"
        );
    }

    /// (d) Lipschitz tripwire (D7/W4): over random points in the scene's bounding region,
    /// the central-difference gradient magnitude of `field_distance` (== `sdf_edit_list`)
    /// must not exceed `FIELD_LIPSCHITZ_L` (= √2). A super-Lipschitz op would void the
    /// cone step's `/ L` clearance bound. Exercises the hard CSG + the smooth-min blend
    /// band (where the peak gradient lives).
    #[test]
    fn field_lipschitz_bound_holds() {
        let scenes: [Vec<SdfEdit>; 3] = [
            vec![
                SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
                SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
            ],
            vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.3, 0.2], sdf_op::UNION, 0.0)],
            vec![
                SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
                SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
            ],
        ];
        let mut rng = Rng::new(0x1234_5678_9ABC_DEF0);
        let h = 1e-3_f32;
        let mut max_grad = 0.0_f32;
        let mut samples = 0u64;
        for edits in &scenes {
            for _ in 0..50_000 {
                let p = [rng.range(-1.5, 1.5), rng.range(-1.5, 1.5), rng.range(-1.5, 1.5)];
                let gx = (sdf_edit_list(edits, [p[0] + h, p[1], p[2]])
                    - sdf_edit_list(edits, [p[0] - h, p[1], p[2]]))
                    / (2.0 * h);
                let gy = (sdf_edit_list(edits, [p[0], p[1] + h, p[2]])
                    - sdf_edit_list(edits, [p[0], p[1] - h, p[2]]))
                    / (2.0 * h);
                let gz = (sdf_edit_list(edits, [p[0], p[1], p[2] + h])
                    - sdf_edit_list(edits, [p[0], p[1], p[2] - h]))
                    / (2.0 * h);
                let g = (gx * gx + gy * gy + gz * gz).sqrt();
                if g.is_finite() {
                    max_grad = max_grad.max(g);
                }
                samples += 1;
            }
        }
        // A small tolerance over √2 absorbs the central-difference discretization error
        // at the blend band's curvature; a genuine super-Lipschitz op blows well past it.
        assert!(
            max_grad <= FIELD_LIPSCHITZ_L + 5e-2,
            "field gradient {max_grad} exceeds FIELD_LIPSCHITZ_L {FIELD_LIPSCHITZ_L} (the cone step's /L is unsound)"
        );
        assert!(
            (FIELD_LIPSCHITZ_L - core::f32::consts::SQRT_2).abs() < 1e-6,
            "FIELD_LIPSCHITZ_L must be sqrt(2)"
        );
        println!(
            "[d] Lipschitz: {samples} samples, max |grad field| = {max_grad} <= L = {FIELD_LIPSCHITZ_L} (sqrt 2)"
        );
    }

    /// (e) The 0%-gate anchor: `golden_composite_pixel_culled(coarse_enabled = false)`
    /// is BIT-IDENTICAL to `golden_composite_pixel_ex` over the whole 64×64 image, under
    /// both an ortho and a perspective camera with a mix of covered/uncovered depths. The
    /// TileBound passed is irrelevant when cull-off (the function short-circuits) — a
    /// dummy is supplied.
    #[test]
    fn culled_off_is_bit_identical_to_ex() {
        let edits = vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ];
        let (w, h) = (64u32, 64u32);
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        let dummy = super::super::TileBound { near_t: 7.0, far_t: 9.0, flags: TILE_FLAG_EMPTY, _pad: 0 };

        let cameras = [
            CompositeCamera::Ortho,
            CompositeCamera::Perspective {
                eye: [0.0, 0.0, 3.0],
                forward: [0.0, 0.0, -1.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
                tan_half_fov: (core::f32::consts::FRAC_PI_2 * 0.5).tan(),
                aspect: 1.0,
            },
        ];
        let mut checked = 0u64;
        for camera in cameras {
            for py in 0..h {
                for px in 0..w {
                    let md = depths[((px + py) as usize) % depths.len()];
                    let want = golden_composite_pixel_ex(&edits, md, px, py, w, h, camera);
                    let got = golden_composite_pixel_culled(
                        &edits, md, px, py, w, h, camera, false, dummy,
                    );
                    assert_eq!(
                        want, got,
                        "cull-off diverged at ({px},{py}) depth {md}: ex 0x{want:08X} culled 0x{got:08X}"
                    );
                    checked += 1;
                }
            }
        }
        // Re-anchor: the unused fields of `SDF_CAM_Z` / `SDF_T_MAX` are still the frozen
        // scene constants (a compile-time touch so a refactor that drops them is caught).
        let _ = (SDF_CAM_Z, SDF_T_MAX);
        println!("[e] cull-off bit-identity: {checked} pixels (ortho + perspective) all match golden_composite_pixel_ex");
    }

    /// (f) PERSPECTIVE far_t regression: the audit found the cull's covered-texel decode
    /// used `md * T_MAX` (10) unconditionally, disagreeing with the fine marcher's
    /// PERSPECTIVE decode `md * MESH_DEPTH_T_MAX` (64) — a ~6.4x under-decode that
    /// truncates `far_t` short of real SDF surfaces, culling their tile (a HOLE).
    ///
    /// Setup: a single 8×8 tile (the whole image), a perspective camera looking exactly
    /// down -Z (the tile-center ray is bit-exact `forward` at this extent), a narrow FOV
    /// (small cone half-angle, so the conservative cone-entry tracks the literal surface
    /// closely instead of firing early), a covering mesh depth `md = 0.2`, and a sphere
    /// (r = 0.3) at the origin whose surface the eye-at-z=3 ray first hits at `t = 2.7`.
    ///
    ///   * correct decode: `far_t = min(0.2 * MESH_DEPTH_T_MAX, T_MAX) = min(12.8, 10) = 10`
    ///     — the march reaches the sphere (conservatively enters around `t ≈ 2.45`,
    ///     BEFORE the literal surface, `> 2.0`) ⇒ tile is NON-empty.
    ///   * prior (buggy) decode: `far_t = 0.2 * T_MAX = 2.0` — the march never reaches the
    ///     cone-entry (which needs `t > 2.0`) and breaks at `t >= far_t` ⇒ EMPTY (a hole).
    ///
    /// Asserts the corrected `far_t` value directly (the primary regression pin) AND that
    /// the tile is NOT culled with `near_t` beyond the old (buggy) bound — proving the old
    /// decode would have produced a hole here.
    #[test]
    fn perspective_far_t_uses_mesh_depth_t_max_not_t_max() {
        let (w, h) = (TILE_SIZE, TILE_SIZE); // one 8x8 tile spans the whole image.
        let camera = CompositeCamera::Perspective {
            eye: [0.0, 0.0, 3.0],
            forward: [0.0, 0.0, -1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            tan_half_fov: 0.05, // narrow FOV: a small cone half-angle over the whole tile.
            aspect: 1.0,
        };
        let edits = vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.3, sdf_op::UNION, 0.0)];
        let md = 0.2_f32;
        let tile_depths = [md; 64];

        let tb = golden_tile_bound(&edits, &tile_depths, 0, 0, w, h, camera);

        let old_buggy_far_t = md * SDF_T_MAX; // the pre-fix decode (2.0).
        let expected_far_t = (md * MESH_DEPTH_T_MAX).min(SDF_T_MAX); // the fix (10.0).
        assert!(
            (tb.far_t - expected_far_t).abs() < 1e-4,
            "far_t {} must equal min(md * MESH_DEPTH_T_MAX, T_MAX) = {expected_far_t} \
             (the prior `md * T_MAX` decode gave {old_buggy_far_t}, ~6.4x too shallow)",
            tb.far_t
        );
        assert_eq!(
            tb.flags & TILE_FLAG_EMPTY,
            0,
            "tile wrongly culled EMPTY: far_t {} must not truncate the march before the \
             sphere's first hit at t ~= 2.7 (a HOLE)",
            tb.far_t
        );
        assert!(
            tb.near_t > old_buggy_far_t,
            "near_t {} must exceed the OLD buggy far_t ({old_buggy_far_t}) — proving the \
             old decode would have missed this surface entirely (EMPTY before entry)",
            tb.near_t
        );
        assert!(
            tb.near_t <= 2.7 + 1e-2,
            "near_t {} must be <= the sphere's analytic first hit (~2.7, conservative)",
            tb.near_t
        );
        println!(
            "[f] perspective far_t regression: far_t={} near_t={} (old buggy far_t would've been {old_buggy_far_t})",
            tb.far_t, tb.near_t
        );
    }
}

// ===========================================================================
// Render B1 — over-relaxation (Keinert ω-gated) HOST soundness gates.
//
// These prove the CPU contract the on-device gates rely on, GPU-free:
//   1. ω = 1 host BIT-identity — `_omega(.., 1.0)` byte-equal to the ω=1 forwarder
//      (`golden_composite_pixel_ex` / `_culled`) over a pixel sweep on all 3 scenes
//      (ortho + perspective). Pins the forwarder extraction (the 0%-gate).
//   2. HIT-SET-SUPERSET property — over randomized scenes/depths/pixels, for
//      ω ∈ {1.2, 1.5, 1.9} the `_omega` hit-set ⊇ the ω=1 hit-set (NO ω=1 SDF hit
//      becomes background/mesh at ω>1). A violation = a missed-surface HOLE. Run on
//      BOTH the cull-off and the cull-on (`_culled_omega`) path. (F-CRIT-1 oracle.)
//   3. NO-HOLES TRIPWIRE — a deliberately-broken over-relax (a retreat to a WRONG t)
//      MUST fail the gate-2 invariant, proving #2 has teeth.
//   4. STEP-BOUND property — `steps(ω>1) ≤ steps(ω=1) + 1` per ray over the
//      randomized scenes (≤ 1 permanent sor-fail fallback ⇒ ≤ plain + 1).
//   5. ω CLAMP — the harness's `omega_in.clamp(1.0, 1.99)` + the 8-B push encode:
//      a hostile `omega_in` decodes to a finite value in `[1.0, 1.99]`.
//
// The march mirrors `golden_composite_pixel_ex_omega` / `_culled_omega` EXACTLY (same
// ordering: top mesh-guard, probe, hit test, ω-gate step, miss test). Gates 2/3/4 need
// a hit/step-instrumented copy of that loop (the production goldens return only a packed
// color), so a faithful test-only mirror lives here; gate 1 diffs the production
// functions directly so the mirror can never mask a real forwarder regression.
// ===========================================================================
#[cfg(test)]
mod b1_over_relaxation_tests {
    use super::super::{CompositeCamera, DEFAULT_LIGHT_DIR, DEFAULT_MARCHER_OMEGA, FIELD_LIPSCHITZ_L, FineMarcherPush, GBUFFER_MARCHER_PUSH_BYTES, MESH_DEPTH_CLEAR, SDF_EPS, SDF_IMG_H, SDF_IMG_W, SDF_MAX_IT, SDF_T_MAX, SdfEdit, composite_ray, sdf_edit_list, sdf_op};
    use crate::goldens::{golden_composite_pixel_culled, golden_composite_pixel_culled_omega, golden_composite_pixel_ex, golden_composite_pixel_ex_omega};
    use proptest::prelude::*;

    /// The rung-9/10 "crater" CSG scene (base sphere minus a smaller sphere).
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }
    /// A box CSG scene.
    fn box_csg() -> Vec<SdfEdit> {
        vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
    }
    /// A smooth-union scene (two spheres blended) — the smooth-min path.
    fn smooth_union() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
        ]
    }

    /// Instrumented result of [`march_obs`] — the Candidate-C host oracle output plus the perf
    /// counters that replace the deleted step-bound gate 4.
    struct MarchObs {
        /// The SHIPPED hit decision: the fast pass's hit, OR (on exhaustion) the re-march's hit.
        hit: bool,
        /// Probe iterations spent in the over-relaxed FAST pass (each `sdf_edit_list` call). The
        /// B1 win is `fast_steps(ω>1) < fast_steps(ω=1)` on the common converging rays.
        fast_steps: u32,
        /// True iff the fast pass exhausted the budget and the Candidate-C re-march fired. The
        /// re-march FREQUENCY (% of pixels) is the perf risk: a large fraction = B1 perf-neutral.
        remarched: bool,
    }

    /// The Render B1 ω-march, INSTRUMENTED (Candidate C) — the host oracle for gates 2/3 and
    /// the perf observation. A faithful, COMPLETE mirror of the PRODUCTION
    /// `golden_composite_pixel_ex_omega` march: the over-relaxed fast pass (same ordering,
    /// same ω-gate, same Lipschitz-aware sor-fail test, same `t = safe_t + sor_prev`
    /// plain-resume + permanent fall-to-plain) FOLLOWED BY the Candidate-C fallback re-march.
    ///
    /// CONTRACT CHANGE vs the prior step-bound oracle: the fast pass alone is NOT the shipped
    /// hit decision. Correctness now comes from the re-march, not a step bound. When the fast
    /// loop runs all `SDF_MAX_IT` iterations with NO break (`exhausted`), production RE-MARCHES
    /// from the ORIGINAL seed with a plain ω=1 sphere-trace and uses THAT (hit, t). This oracle
    /// reproduces that exactly, so `hit` is byte-for-byte the production hit decision — gate 2's
    /// "ω>1 hit-set ⊇ ω=1 hit-set" tests what actually ships, and is provably true (the re-march
    /// body is the frozen plain marcher, so an exhausting ω>1 ray lands on the SAME hit the
    /// frozen marcher does).
    ///
    /// INSTRUMENTATION (perf observation, replacing the deleted step-bound gate 4): the returned
    /// [`MarchObs`] records the fast-pass probe count (`fast_steps`, each `sdf_edit_list` call in
    /// the over-relaxed loop), whether the fast pass exhausted the budget, and whether the
    /// re-march fired. The orchestrator uses the re-march FREQUENCY (% of pixels that exhausted)
    /// and the fast-pass step reduction vs plain to judge whether B1 is still a net win.
    fn march_obs(edits: &[SdfEdit], ro: [f32; 3], rd: [f32; 3], t_mesh: f32, omega_in: f32) -> MarchObs {
        let mut t = 0.0_f32;
        let t_seed = t; // the ORIGINAL seed (0.0) — Candidate C re-march re-seeds from it
        let mut omega = omega_in;
        let mut hit = false;
        let mut fast_steps = 0u32;
        let mut safe_t = 0.0_f32;
        let mut sor_prev = 0.0_f32;
        let mut sor_step_prev = 0.0_f32;
        let mut exhausted = true; // cleared by EVERY in-loop break (mirrors production)
        for it in 0..SDF_MAX_IT {
            if t >= t_mesh {
                exhausted = false;
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            fast_steps += 1;
            if d < SDF_EPS {
                hit = true;
                exhausted = false;
                break;
            }
            if omega > 1.0 {
                let step_len = d * omega;
                // Lipschitz-aware sor-fail (mirrors production exactly): the empty-ball radii
                // are `f / L`, so the spheres cover the over-step iff `sor_prev + d >= L * step`.
                if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                    // BUG-B1-HOLE-2: resume the plain march one certified step past the safe
                    // probe (no re-probe, +0 steps for the retreat itself) and latch to plain.
                    t = safe_t + sor_prev;
                    omega = 1.0;
                    continue;
                }
                safe_t = t;
                sor_prev = d;
                sor_step_prev = step_len;
                t += step_len;
            } else {
                t += d;
            }
            if t > SDF_T_MAX {
                exhausted = false;
                break;
            }
        }

        // Candidate C: the PROVABLY-hole-free fallback re-march. Mirrors production EXACTLY —
        // on `exhausted` re-seed from `t_seed` and run the frozen plain ω=1 marcher; its (hit)
        // is the shipped decision. The fast pass's `hit` is discarded (it was false on exhaust).
        let remarched = exhausted;
        if exhausted {
            t = t_seed;
            hit = false;
            for _it2 in 0..SDF_MAX_IT {
                if t >= t_mesh {
                    break;
                }
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let d = sdf_edit_list(edits, p);
                if d < SDF_EPS {
                    hit = true;
                    break;
                }
                t += d;
                if t > SDF_T_MAX {
                    break;
                }
            }
        }

        MarchObs { hit, fast_steps, remarched }
    }

    /// A DELIBERATELY-BROKEN over-relax used ONLY by the gate-3 tripwire. TWO co-ordinated
    /// breaks, BOTH required under the Candidate-C contract:
    ///   1. the fast pass mis-handles a sor-fail — it retreats to a WRONG `t` (`safe_t +
    ///      step_len`, i.e. it ADVANCES past the surface instead of retreating) AND keeps ω hot;
    ///   2. **the Candidate-C fallback re-march is DISABLED** (this function has NO re-march at
    ///      all — it returns the bare fast-pass hit). This is the load-bearing tripwire change
    ///      for the C contract: with the re-march intact, an exhausting broken ray would be
    ///      silently rescued by the plain re-march and the tripwire would go INERT (gate 2 would
    ///      look armed while testing nothing). Breaking C's guarantee = breaking the re-march, so
    ///      this models exactly the failure mode gate 2 must catch. NOT shipped.
    fn march_hit_broken(
        edits: &[SdfEdit],
        ro: [f32; 3],
        rd: [f32; 3],
        t_mesh: f32,
        omega_in: f32,
        with_remarch: bool,
    ) -> bool {
        let mut t = 0.0_f32;
        let t_seed = t;
        let omega = omega_in;
        let mut hit = false;
        let mut safe_t = 0.0_f32;
        let mut sor_prev = 0.0_f32;
        let mut sor_step_prev = 0.0_f32;
        let mut exhausted = true;
        for it in 0..SDF_MAX_IT {
            if t >= t_mesh {
                exhausted = false;
                break;
            }
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            let d = sdf_edit_list(edits, p);
            if d < SDF_EPS {
                hit = true;
                exhausted = false;
                break;
            }
            if omega > 1.0 {
                let step_len = d * omega;
                // Production Lipschitz-aware detection threshold (re-synced) — the bug is in
                // the RETREAT below, NOT the detection, so the tripwire fires the same sor-fails
                // the production marcher would, then mishandles them.
                if it > 0 && sor_prev + d < FIELD_LIPSCHITZ_L * sor_step_prev {
                    // BUG: a WRONG "retreat" that actually leaps past the surface and
                    // never falls to plain. The classic over-relaxation hole.
                    t = safe_t + step_len;
                    continue;
                }
                safe_t = t;
                sor_prev = d;
                sor_step_prev = step_len;
                t += step_len;
            } else {
                t += d;
            }
            if t > SDF_T_MAX {
                exhausted = false;
                break;
            }
        }
        // `with_remarch == false`: C's fallback is DISABLED — the bare broken fast-pass hit (the
        // tripwire that MUST hole). `with_remarch == true`: re-attach the EXACT Candidate-C
        // re-march on top of the broken fast pass — it must CLOSE every hole the broken pass
        // opened, proving the re-march (not the fast pass) is what guarantees the hit-set.
        if with_remarch && exhausted {
            t = t_seed;
            hit = false;
            for _it2 in 0..SDF_MAX_IT {
                if t >= t_mesh {
                    break;
                }
                let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let d = sdf_edit_list(edits, p);
                if d < SDF_EPS {
                    hit = true;
                    break;
                }
                t += d;
                if t > SDF_T_MAX {
                    break;
                }
            }
        }
        hit
    }

    /// True when pixel `(px, py)` is an SDF hit at `omega` — the SHIPPED Candidate-C decision
    /// (fast pass + fallback re-march), with NO mesh occlusion (the pure-field hit set — the
    /// property's domain). This is byte-for-byte what `golden_composite_pixel_ex_omega`'s march
    /// concludes, so gate 2's superset property tests production, not the bare fast pass.
    fn pixel_hits(edits: &[SdfEdit], px: u32, py: u32, omega: f32) -> bool {
        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
        march_obs(edits, ro, rd, 1.0e30, omega).hit
    }

    /// GATE 1 — ω = 1 host BIT-identity for the un-culled marcher. `_omega(.., 1.0)` must be
    /// byte-equal to the ω=1 forwarder over the whole 64×64 image on all 3 scenes, ORTHO +
    /// PERSPECTIVE. Pins the forwarder extraction (any drift = the 0%-gate broke).
    #[test]
    fn gate1_omega_one_is_bit_identical_to_forwarder_uncull() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        let cameras = [
            ("ortho", CompositeCamera::Ortho),
            (
                "persp",
                CompositeCamera::Perspective {
                    eye: [0.0, 0.0, 2.0],
                    forward: [0.0, 0.0, -1.0],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    tan_half_fov: 0.5,
                    aspect: 1.0,
                },
            ),
        ];
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for (cname, cam) in cameras {
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = depths[((px + py) as usize) % depths.len()];
                        let fwd = golden_composite_pixel_ex(edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam);
                        let om1 = golden_composite_pixel_ex_omega(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, 1.0,
                        );
                        assert_eq!(
                            fwd, om1,
                            "[{sname}/{cname}] ω=1 _omega diverged from forwarder at ({px},{py}) depth {md}: \
                             fwd 0x{fwd:08X} omega 0x{om1:08X}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("[B1 gate1] ω=1 un-culled bit-identity: {checked} pixels (ortho+persp × 3 scenes) byte-equal");
    }

    /// GATE 1 (cull path) — ω = 1 host BIT-identity for the CULLED marcher. With cull ON and a
    /// synthetic non-EMPTY tile (`near_t = 0`, `far_t = T_MAX`), `_culled_omega(.., 1.0)` must
    /// be byte-equal to the ω=1 culled forwarder over the image (ORTHO). The cull-off arm is
    /// covered by gate 1; this pins the seeded-march forwarder at ω=1.
    #[test]
    fn gate1_omega_one_is_bit_identical_to_forwarder_culled() {
        use super::super::{TILE_FLAG_EMPTY, TileBound};
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let depths = [0.5_f32, MESH_DEPTH_CLEAR, 0.2, 0.8];
        // A non-EMPTY, full-range tile (seed t = 0, march to T_MAX) — the general case.
        let surf = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: 0, _pad: 0 };
        // An EMPTY tile — exercises the early-out arm at ω=1.
        let empty = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: TILE_FLAG_EMPTY, _pad: 0 };
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for tile in [surf, empty] {
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let md = depths[((px + py) as usize) % depths.len()];
                        let fwd = golden_composite_pixel_culled(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, tile,
                        );
                        let om1 = golden_composite_pixel_culled_omega(
                            edits, md, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, true, tile, 1.0,
                        );
                        assert_eq!(
                            fwd, om1,
                            "[{sname}] ω=1 _culled_omega diverged from forwarder at ({px},{py}) depth {md} \
                             flags {}: fwd 0x{fwd:08X} omega 0x{om1:08X}",
                            tile.flags
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("[B1 gate1c] ω=1 culled bit-identity: {checked} pixels (surf+empty tiles × 3 scenes) byte-equal");
    }

    // A proptest-generated randomized SDF scene, WIDENED for the Candidate-C no-hole contract:
    // 1..=8 edits, random kind/op/center/size, and an AGGRESSIVE smoothness distribution biased
    // toward the super-Lipschitz blend bands that historically holed (BUG-B1-HOLE-1). Every value
    // stays inside the bounded `[-0.85, 0.85]³` view box so the marcher reaches surfaces (an empty
    // world trivially satisfies superset). THIN features are reachable via the small-size tail
    // (down to 0.03) — sliver boxes/spheres are the classic over-relax overshoot trap. (A `//`
    // comment, not a doc comment — clippy's `unused_doc_comments` on macro invocations.)
    prop_compose! {
        fn arb_scene()(
            edits in proptest::collection::vec(
                (
                    0u32..2,                                  // 0 = sphere, 1 = box
                    0u32..3,                                  // op: union/subtract/intersect
                    -0.85f32..0.85, -0.85f32..0.85, -0.85f32..0.85, // center xyz
                    0.03f32..0.6,                             // size a (thin tail at 0.03)
                    0.03f32..0.5,                             // size b (box y, thin tail)
                    0.03f32..0.5,                             // size c (box z, thin tail)
                    // AGGRESSIVE smooth-min: hard, mild, and a heavy super-Lipschitz tail up to
                    // 0.4 (the blend band where IQ's smooth-min violates the unit-Lipschitz bound
                    // hardest — the BUG-B1-HOLE-1 regime). Weighted toward the soft cases.
                    prop_oneof![
                        1 => Just(0.0f32),
                        2 => 0.02f32..0.15,
                        3 => 0.15f32..0.40,
                    ],
                ),
                1..=8,
            )
        ) -> Vec<SdfEdit> {
            // Force the FIRST edit to be a UNION so the field has a positive volume to hit
            // (a lone subtract/intersect over an empty acc is a degenerate empty field).
            edits.into_iter().enumerate().map(|(i, (kind, op, cx, cy, cz, a, b, c, k))| {
                let op = if i == 0 { sdf_op::UNION } else { op };
                if kind == 0 {
                    SdfEdit::sphere([cx, cy, cz], a, op, k)
                } else {
                    SdfEdit::box_shape([cx, cy, cz], [a, b, c], op, k)
                }
            }).collect()
        }
    }

    proptest! {
        // WIDENED for the Candidate-C no-hole contract — this IS the correctness gate, so make
        // it thorough. 1024 random scenes (4× the prior 256), each over a coarse pixel grid ×
        // ω ∈ {1.2, 1.5, 1.99} × {ortho, perspective}. The pinned BUG-B1-HOLE-1 cliff seed in
        // proptest-regressions/compute.txt is replayed first on every run (proptest auto-loads it).
        #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

        /// GATE 2 — HIT-SET-SUPERSET (the F-CRIT-1 soundness oracle, the REAL correctness gate),
        /// CULL-OFF. For every randomized scene + ω ∈ {1.2, 1.5, 1.99} + camera ∈ {ortho,
        /// perspective}, EVERY pixel that is an SDF hit at ω=1 must remain a hit at ω>1. With
        /// Candidate C this is PROVABLY true on every case: an exhausting ω>1 ray re-marches with
        /// the frozen plain marcher, so it lands on the SAME hit ω=1 does. A regression = a
        /// missed-surface HOLE (Candidate C has a bug). The perspective camera supplies GRAZING
        /// rays at the frame edges (the classic over-relax overshoot trap). Iterates a coarse
        /// 4-px grid (every tile sampled) to bound per-case cost while covering the frame.
        #[test]
        fn gate2_hit_set_superset_cull_off(edits in arb_scene()) {
            let persp = CompositeCamera::Perspective {
                eye: [0.0, 0.0, 2.0], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0], tan_half_fov: 0.5, aspect: 1.0,
            };
            for &omega in &[1.2_f32, 1.5, 1.99] {
                for cam in [CompositeCamera::Ortho, persp] {
                    for py in (0..SDF_IMG_H).step_by(4) {
                        for px in (0..SDF_IMG_W).step_by(4) {
                            let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, cam);
                            let base = march_obs(&edits, ro, rd, 1.0e30, 1.0).hit;
                            if base {
                                let over = march_obs(&edits, ro, rd, 1.0e30, omega).hit;
                                prop_assert!(
                                    over,
                                    "HOLE: ({px},{py}) cam={cam:?} hits at ω=1 but MISSES at ω={omega} — Candidate-C re-march failed to close the hole; scene={edits:?}"
                                );
                            }
                        }
                    }
                }
            }
        }

        /// GATE 2 — HIT-SET-SUPERSET, CULL-ON. Same invariant through the `_culled_omega` path
        /// with a non-EMPTY full-range tile (seed t=0): the ω>1 culled hit-set ⊇ the ω=1 culled
        /// hit-set. Compares the FINAL packed color: a pixel that is the lit SDF color at ω=1
        /// must NOT become mesh/background at ω>1 (the observable hole). Uses no mesh
        /// (depth = clear) so the SDF/background partition is the field's alone.
        #[test]
        fn gate2_hit_set_superset_cull_on(edits in arb_scene()) {
            use super::super::{TileBound};
            let tile = TileBound { near_t: 0.0, far_t: SDF_T_MAX, flags: 0, _pad: 0 };
            for &omega in &[1.2_f32, 1.5, 1.99] {
                for py in (0..SDF_IMG_H).step_by(4) {
                    for px in (0..SDF_IMG_W).step_by(4) {
                        // ω=1 culled color (the baseline partition).
                        let c1 = golden_composite_pixel_culled_omega(
                            &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                            CompositeCamera::Ortho, true, tile, 1.0,
                        );
                        // Only pixels that HIT the SDF at ω=1 are in the domain (no mesh, so a
                        // non-background color == an SDF hit).
                        if pixel_hits(&edits, px, py, 1.0) {
                            let co = golden_composite_pixel_culled_omega(
                                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                                CompositeCamera::Ortho, true, tile, omega,
                            );
                            prop_assert!(
                                pixel_hits(&edits, px, py, omega),
                                "HOLE(cull-on): ({px},{py}) SDF-hit at ω=1 (0x{c1:08X}) but ω={omega} march misses (0x{co:08X}); scene={edits:?}"
                            );
                        }
                    }
                }
            }
        }

    }

    // ===========================================================================================
    // PERF OBSERVATION (replaces the DELETED step-bound gate 4).
    //
    // Candidate C makes correctness independent of any step-count bound: the fallback re-march
    // guarantees the hit-set regardless of how the fast pass behaves, so the old gate-4 invariant
    // `steps(ω>1) ≤ steps(ω=1)` is IRRELEVANT to soundness. It is replaced by an OBSERVATION (not
    // a pass/fail correctness gate): how OFTEN does the fast pass exhaust and trigger the (costly)
    // re-march, and does the over-relaxed fast pass still REDUCE probe steps on the common
    // converging rays (the B1 win)? The orchestrator uses these numbers for the ship call. The
    // only assertion here is the must-render sanity (the fixture is non-empty); a high re-march
    // fraction is REPORTED, not failed, and flagged in the println for the orchestrator.
    // ===========================================================================================

    /// PERF OBSERVATION — re-march frequency + fast-pass step reduction (NOT a correctness gate).
    /// Over the shipped fixtures + the pinned BUG-B1-HOLE-1 cliff seed + a handful of widened
    /// random scenes, counts, per ω ∈ {1.2, 1.5, 1.99}: (a) the % of pixels whose fast pass
    /// EXHAUSTED and re-marched (the perf risk — a large fraction means B1 is perf-neutral or
    /// negative), and (b) the mean fast-pass probe count vs the ω=1 plain march on CONVERGING
    /// rays (the B1 win — fewer steps to the same hit). Prints a summary for the orchestrator.
    #[test]
    fn perf_observation_remarch_frequency_and_step_reduction() {
        // The pinned cliff seed (the historical worst ray) — exercised explicitly.
        let cliff = vec![
            SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
            SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
            SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
        ];
        let scenes = [
            ("crater", crater()),
            ("box", box_csg()),
            ("smooth", smooth_union()),
            ("cliff_seed", cliff),
        ];
        let mut worst_remarch_pct = 0.0_f64;
        for (sname, edits) in &scenes {
            for &omega in &[1.2_f32, 1.5, 1.99] {
                let mut pixels = 0u64;
                let mut remarches = 0u64;
                // Converging-ray step accounting: only rays that HIT under BOTH ω=1 and ω
                // (so the comparison is the same surface) and did NOT need a re-march.
                let mut conv_rays = 0u64;
                let mut sum_fast = 0u64; // fast-pass steps at ω (the B1 path)
                let mut sum_plain = 0u64; // fast-pass steps at ω=1 (the baseline)
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let o1 = march_obs(edits, ro, rd, 1.0e30, 1.0);
                        let oo = march_obs(edits, ro, rd, 1.0e30, omega);
                        pixels += 1;
                        if oo.remarched {
                            remarches += 1;
                        }
                        // Common converging rays: both hit, neither re-marched → the step
                        // reduction is the apples-to-apples B1 win on the typical case.
                        if o1.hit && oo.hit && !o1.remarched && !oo.remarched {
                            conv_rays += 1;
                            sum_fast += u64::from(oo.fast_steps);
                            sum_plain += u64::from(o1.fast_steps);
                        }
                    }
                }
                let remarch_pct = 100.0 * remarches as f64 / pixels as f64;
                worst_remarch_pct = worst_remarch_pct.max(remarch_pct);
                let (mean_fast, mean_plain, reduction) = if conv_rays > 0 {
                    let mf = sum_fast as f64 / conv_rays as f64;
                    let mp = sum_plain as f64 / conv_rays as f64;
                    (mf, mp, 100.0 * (mp - mf) / mp)
                } else {
                    (0.0, 0.0, 0.0)
                };
                // A POSITIVE reduction is the B1 win (fewer steps to the same hit); negative means
                // the over-relax overshot and cost extra steps at this ω. ω=1.2 (the production
                // DEFAULT) is the column that decides the ship call.
                let verdict = if reduction >= 0.0 { "B1 win" } else { "B1 LOSS" };
                println!(
                    "[B1 perf] {sname} ω={omega}: re-march {remarches}/{pixels} px ({remarch_pct:.2}%); \
                     converging rays {conv_rays}: mean fast-pass steps {mean_fast:.2} vs plain {mean_plain:.2} \
                     (step reduction {reduction:.1}% — {verdict})"
                );
            }
        }
        // Sanity only (NOT a correctness gate): the fixtures must actually render surfaces so the
        // observation is meaningful. The re-march fraction itself is REPORTED, never failed.
        println!(
            "[B1 perf] OBSERVATION SUMMARY: worst re-march fraction over all fixtures/ω = {worst_remarch_pct:.2}% \
             (FLAG for the orchestrator if this is a large fraction — would mean B1 is perf-neutral/negative)"
        );
    }

    /// GATE 3 — NO-HOLES TRIPWIRE (adapted to the Candidate-C contract). The gate-2 invariant
    /// must have TEETH. Under Candidate C, "broken" must break the RE-MARCH too — otherwise the
    /// fallback silently rescues a broken fast pass and the tripwire goes inert. So this asserts
    /// TWO things:
    ///   (a) the broken over-relax WITH C's fallback DISABLED (`march_hit_broken(.., false)` — a
    ///       WRONG sor-fail retreat that leaps past the surface, no re-march) produces ≥ 1 HOLE.
    ///       If it never holed, gate 2 would be vacuous.
    ///   (b) the SAME broken fast pass WITH C's re-march RE-ATTACHED (`march_hit_broken(.., true)`)
    ///       produces ZERO holes — proving the re-march (NOT the fast pass) is what closes them,
    ///       i.e. C's guarantee is load-bearing and gate 2 passes BECAUSE of the re-march.
    /// (The broken march is test-only; never shipped.)
    #[test]
    fn gate3_no_holes_tripwire_broken_overrelax_holes() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut total_holes_no_remarch = 0u64;
        let mut total_holes_with_remarch = 0u64;
        for (sname, edits) in &scenes {
            for &omega in &[1.5_f32, 1.9] {
                let mut scene_holes = 0u64;
                let mut scene_holes_remarched = 0u64;
                for py in 0..SDF_IMG_H {
                    for px in 0..SDF_IMG_W {
                        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
                        let base = pixel_hits(edits, px, py, 1.0);
                        if base {
                            // (a) C's fallback disabled: this is the tripwire that MUST hole.
                            if !march_hit_broken(edits, ro, rd, 1.0e30, omega, false) {
                                scene_holes += 1;
                            }
                            // (b) the EXACT C re-march re-attached: it must close the hole.
                            if !march_hit_broken(edits, ro, rd, 1.0e30, omega, true) {
                                scene_holes_remarched += 1;
                            }
                        }
                    }
                }
                total_holes_no_remarch += scene_holes;
                total_holes_with_remarch += scene_holes_remarched;
                if scene_holes > 0 {
                    println!("[B1 gate3] tripwire: broken over-relax (no re-march) holed {scene_holes} px on {sname} @ ω={omega}");
                }
            }
        }
        // (a) teeth: the broken fast pass without the fallback MUST hole.
        assert!(
            total_holes_no_remarch > 0,
            "TRIPWIRE INERT: the broken over-relax (re-march disabled) produced ZERO holes — gate 2 would be vacuous"
        );
        // (b) the re-march is the load-bearing guarantee: re-attaching it closes EVERY hole.
        assert_eq!(
            total_holes_with_remarch, 0,
            "C CONTRACT VIOLATION: the Candidate-C re-march failed to close {total_holes_with_remarch} broken-fast-pass holes — the fallback is not actually hole-free"
        );
        println!(
            "[B1 gate3] tripwire armed: {total_holes_no_remarch} holes WITHOUT the re-march (gate 2 has teeth); \
             {total_holes_with_remarch} holes WITH the re-march re-attached (C's fallback closes them ALL — the guarantee is load-bearing)"
        );
    }

    /// REGRESSION PIN (BUG-B1-HOLE-1, CLOSED via Candidate C) — the minimal scene the gate-2
    /// property shrank to: a super-Lipschitz smooth-min CSG (a box with smoothness 0.244 blended
    /// between two spheres) that USED to produce a missed-surface HOLE at pixel (28,16) under
    /// ω=1.2 through the PRODUCTION golden (`golden_composite_pixel_ex_omega`). Candidate C closes
    /// the hole UNCONDITIONALLY: when the over-relaxed fast pass exhausts the budget on this ray,
    /// the fallback re-march replays the frozen plain marcher from the original seed and lands on
    /// the SAME lit SDF surface ω=1 hits — byte-identical color, not merely non-background.
    ///
    /// This is the PERMANENT regression guard (NOT ignored). FLIPPED for the C contract: it now
    /// asserts BOTH ω=1.0 AND ω ∈ {1.2, 1.5, 1.99} HIT the surface (the no-hole contract — none
    /// reverts to BACKGROUND) AND land on the SAME surface FEATURE as ω=1 (the lit SDF color, far
    /// from background — within a few LSBs per channel, NOT a phantom).
    ///
    /// NOTE on exactness: byte-exact color equality with ω=1 holds ONLY when the Candidate-C
    /// re-march fires (it replays the frozen plain marcher → the identical `t` and shade — true
    /// for ω=1.2 here). When the over-relaxed FAST pass converges on its own (ω=1.5/1.99 at this
    /// pixel) it lands within `SDF_EPS` of the surface at a marginally different `t`, so the
    /// Lambert shade differs by a few LSBs. That is a valid same-surface hit, not a hole — the
    /// contract is HIT (not background), asserted via the lit-vs-background channel separation.
    #[test]
    fn bug_b1_hole_1_smooth_min_overrelax_hole_via_production_golden() {
        let edits = vec![
            SdfEdit::sphere([0.31460363, 0.70498204, -0.7611318], 0.36075538, sdf_op::UNION, 0.0),
            SdfEdit::box_shape([0.092381336, 0.1372761, -0.5955315], [0.19970395, 0.46420184, 0.3901827], sdf_op::UNION, 0.24384262),
            SdfEdit::sphere([0.4506038, 0.16997452, 0.0], 0.44928917, sdf_op::UNION, 0.0),
        ];
        let (px, py) = (28u32, 16u32);
        let bg = super::super::pack_rgba([0.05, 0.05, 0.1]);
        // Channel splitter for the lit-vs-background separation (the three composite outcomes are
        // >100/255 apart, so a few-LSB convergence-point wobble never reclasses a hit as a miss).
        let chans = |c: u32| [(c & 0xFF) as i32, ((c >> 8) & 0xFF) as i32, ((c >> 16) & 0xFF) as i32];
        let bgc = chans(bg);
        let far_from_bg = |c: u32| {
            let cc = chans(c);
            (0..3).any(|i| (cc[i] - bgc[i]).abs() > 8)
        };
        // No mesh (depth = clear) so the SDF/background partition is the field's alone.
        let c1 = golden_composite_pixel_ex_omega(&edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0);
        assert_ne!(c1, bg, "ω=1 must HIT the smooth-min surface at ({px},{py})");
        assert!(far_from_bg(c1), "ω=1 color 0x{c1:08X} must be the LIT SDF surface, far from bg 0x{bg:08X}");
        // Candidate C: EVERY shipped ω>1 must now HIT the same surface FEATURE as ω=1 — the hole
        // is closed by the re-march, not a step bound. The task's required set: {1.2, 1.5, 1.99}.
        for &omega in &[1.2_f32, 1.5, 1.99] {
            let co = golden_composite_pixel_ex_omega(
                &edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, omega,
            );
            // The pixel-color delta to ω=1: 0 when the re-march fired (exact frozen replay), a few
            // LSBs when the fast pass converged on its own. Printed so the orchestrator sees it.
            let dc = chans(co);
            let c1c = chans(c1);
            let max_ch = (0..3).map(|i| (dc[i] - c1c[i]).abs()).max().unwrap_or(0);
            println!(
                "[BUG-B1-HOLE-1 CLOSED] ω=1 0x{c1:08X} | ω={omega} 0x{co:08X} (hit={}, Δ to ω=1 = {max_ch}/255) | bg=0x{bg:08X}",
                co != bg
            );
            // The no-hole contract: ω>1 must NOT revert to background — it HITS the surface.
            assert_ne!(co, bg, "BUG-B1-HOLE-1 CLOSED: ω={omega} must HIT (not background) at ({px},{py})");
            assert!(
                far_from_bg(co),
                "BUG-B1-HOLE-1 CLOSED: ω={omega} color 0x{co:08X} must be the LIT SDF surface (far from bg 0x{bg:08X}), not a hole, at ({px},{py})"
            );
            // Same surface FEATURE as ω=1 — the lit colors agree within the small convergence-point
            // wobble (a phantom or a different feature would differ by >100/channel like bg/mesh).
            assert!(
                max_ch <= 16,
                "BUG-B1-HOLE-1 CLOSED: ω={omega} 0x{co:08X} differs from ω=1 0x{c1:08X} by {max_ch}/255 (>16) — not the same surface feature at ({px},{py})"
            );
        }
    }

    /// GATE 5 — ω CLAMP + 8-byte push encode. Every NON-NaN hostile `omega_in` (negative,
    /// sub-1, == 2, > 2, ±∞) must clamp into `[1.0, 1.99]` and decode finite-in-range from the
    /// pushed bytes `[4..8]`. Mirrors the harness's `omega_in.clamp(1.0, 1.99)` + push encode
    /// EXACTLY (the same `f32::clamp` + `to_le_bytes`).
    ///
    /// FINDING (documented, NOT a soundness hole): Rust's `f32::clamp` does NOT sanitize a NaN
    /// VALUE — `f32::NAN.clamp(1.0, 1.99) == NaN` (the clamp returns NaN when `self` is NaN).
    /// So a NaN ω SURVIVES the harness clamp and is pushed verbatim. It is defanged DOWNSTREAM,
    /// not by the clamp: the marcher's gate is `if (omega > 1.0)`, and `NaN > 1.0 == false` on
    /// BOTH host (`golden_composite_pixel_ex_omega`) and shader, so a NaN ω takes the verbatim
    /// frozen `t += d` plain arm — i.e. it degrades to the ω=1 path (NO over-relaxation, NO
    /// hole). This test asserts that exact safety property (NaN ω ≡ ω=1 over a pixel sweep)
    /// rather than a false "clamp produces 1.0" claim. See the tester report.
    #[test]
    fn gate5_omega_clamp_and_push_encode() {
        // The harness's encode site, reproduced byte-for-byte via the 32-byte
        // `FineMarcherPush` (A1/A2 widened the push 8 → 32 B; `lighting_flags == 0` keeps
        // the OFF path). coarse_enabled stays at offset 0, omega at offset 4.
        fn encode(omega_in: f32, coarse_enabled: bool) -> [u8; GBUFFER_MARCHER_PUSH_BYTES as usize] {
            let omega: f32 = omega_in.clamp(1.0, 1.99);
            let push = FineMarcherPush::new(coarse_enabled, omega, 0, DEFAULT_LIGHT_DIR);
            push.as_bytes().try_into().expect("invariant: FineMarcherPush is GBUFFER_MARCHER_PUSH_BYTES")
        }
        // Non-NaN hostile inputs: ALL must clamp finite into [1.0, 1.99].
        let cases = [-1.0_f32, 0.5, 1.0, 1.2, 1.99, 2.0, 2.5, 100.0, f32::INFINITY, f32::NEG_INFINITY];
        for &om in &cases {
            let push = encode(om, true);
            let coarse = u32::from_le_bytes([push[0], push[1], push[2], push[3]]);
            let decoded = f32::from_le_bytes([push[4], push[5], push[6], push[7]]);
            assert_eq!(coarse, 1, "coarse_enabled must round-trip as 1");
            assert!(decoded.is_finite(), "ω={om} decoded to non-finite {decoded}");
            assert!(
                (1.0..=1.99).contains(&decoded),
                "ω={om} decoded to {decoded}, outside [1.0, 1.99] (clamp failed)"
            );
        }
        // The DOCUMENTED NaN behavior: the clamp passes NaN through (this is the finding).
        let nan_push = encode(f32::NAN, false);
        let nan_dec = f32::from_le_bytes([nan_push[4], nan_push[5], nan_push[6], nan_push[7]]);
        assert!(nan_dec.is_nan(), "FINDING CHANGED: f32::clamp now sanitizes NaN? got {nan_dec}");
        // The SOUNDNESS property that actually protects us: a NaN ω ≡ the ω=1 plain march
        // (the `omega > 1.0` gate is false for NaN on both host and shader → no hole).
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    let plain = golden_composite_pixel_ex_omega(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, 1.0,
                    );
                    let nan_omega = golden_composite_pixel_ex_omega(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, f32::NAN,
                    );
                    assert_eq!(
                        plain, nan_omega,
                        "[{sname}] NaN ω diverged from the ω=1 plain march at ({px},{py}): \
                         plain 0x{plain:08X} nan 0x{nan_omega:08X} — the NaN-defang property broke"
                    );
                    checked += 1;
                }
            }
        }
        // The production default must be inside the clamp window (a sanity tie to the harness).
        assert!((1.0..=1.99).contains(&DEFAULT_MARCHER_OMEGA), "DEFAULT_MARCHER_OMEGA out of clamp window");
        println!(
            "[B1 gate5] ω clamp: {} non-NaN hostile inputs decode finite ∈ [1.0,1.99]; NaN PASSES the clamp \
             but ≡ the ω=1 plain march over {checked} px (gate false for NaN); default={DEFAULT_MARCHER_OMEGA}",
            cases.len()
        );
    }
}

// ===========================================================================
// M1 — the EMPTY-SPACE-SKIP marcher test matrix (host-side, no GPU).
//
// The two LOAD-BEARING soundness gates:
//   (1) OFF byte-identical — `golden_composite_pixel_brick(brick_enabled=0)` is
//       BIT-FOR-BIT the pre-M1 `golden_composite_pixel_ex_omega_lit` over a
//       battery of scenes/cameras/pixels (the 0%-gate).
//   (2) ON hit-set == analytic — over ≥500 random scenes + the demo scene, the
//       empty-skip ON marcher and the analytic (OFF) marcher agree on EVERY
//       pixel's HIT/MISS classification AND surface color (the empty skip only
//       changes WHERE steps land, never the converged hit). A skipped or spurious
//       surface is a BLOCKER.
//
// Plus: never-skip-surface (3), `dist_to_brick_exit` progress (4),
// `build_pointer_grid` correctness (5), push-constant layout (6) — the std430
// agreement the dev SPIR-V-verified, guarded host-side.
//
// The xorshift scene generator mirrors the M0 brick GATE generator (no new dep).
// ===========================================================================
#[cfg(test)]
mod m1_empty_skip_tests {
    use super::super::{CompositeCamera, DEFAULT_LIGHT_DIR, DEFAULT_MARCHER_OMEGA, FineMarcherPush, GBUFFER_MARCHER_PUSH_BYTES, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS, MESH_DEPTH_CLEAR, SDF_IMG_H, SDF_IMG_W, SdfEdit, sdf_op};
    use crate::goldens::{golden_composite_pixel_brick, golden_composite_pixel_ex_omega_lit, host_brick_cell};
    use boyko_sdf_math::brick::{
        BRICK_EXIT_EPS, DEFAULT_BRICK_WORLD, DEFAULT_GRID_DIM, PointerGrid, build_pointer_grid,
        classify_brick, dist_to_brick_exit,
    };
    use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditField};

    // ── shared fixtures ────────────────────────────────────────────────────

    /// `EmptyOutside as u32` — the cell class the empty-skip acts on (mirror of the
    /// private `super::BRICK_CLASS_EMPTY_OUTSIDE`, re-stated so a drift in the enum
    /// discriminant is caught by `class_codes_match_brickclass_discriminants`).
    const EMPTY_OUTSIDE: u32 = 0;

    /// A deterministic xorshift64* PRNG — the scene generator without a dep (mirrors
    /// the M0 brick GATE's generator so the two suites draw from the SAME family).
    struct XorShift64(u64);

    impl XorShift64 {
        fn new(seed: u64) -> Self {
            Self(seed ^ 0x9E37_79B9_7F4A_7C15)
        }
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            let frac = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
            lo + frac * (hi - lo)
        }
        fn below(&mut self, n: u32) -> u32 {
            (self.next_u64() % n as u64) as u32
        }
    }

    /// The demo / golden "crater" CSG scene (base sphere minus a smaller sphere) — the
    /// SAME scene the rung-9/10 and B1 goldens use, so the M1 gates run against the
    /// production demo field, not only synthetic scenes.
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    fn box_csg() -> Vec<SdfEdit> {
        vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
    }

    fn smooth_union() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
        ]
    }

    /// Builds an `SdfEditField` (the authority) from a slice of edits and bumps its gen.
    fn field_of(edits: &[SdfEdit]) -> SdfEditField {
        let mut f = SdfEditField::new();
        for e in edits {
            assert!(f.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    }

    /// A random valid scene (1..=6 edits, first forced UNION, SPHERE/BOX, radii/half-
    /// extents >= 0.5, centers in [-2,2]³, UNION/SUBTRACT/INTERSECT, smoothness 0 or 0.15)
    /// returned as a `Vec<SdfEdit>` — the ON-vs-analytic gate's scene family.
    fn random_scene(rng: &mut XorShift64) -> Vec<SdfEdit> {
        let n = 1 + rng.below(6);
        let mut edits = Vec::with_capacity(n as usize);
        for i in 0..n {
            let center =
                [rng.range(-2.0, 2.0), rng.range(-2.0, 2.0), rng.range(-2.0, 2.0)];
            let op = if i == 0 {
                sdf_op::UNION
            } else {
                match rng.below(3) {
                    0 => sdf_op::UNION,
                    1 => sdf_op::SUBTRACT,
                    _ => sdf_op::INTERSECT,
                }
            };
            let smoothness = if rng.below(2) == 0 { 0.0 } else { 0.15 };
            let e = if rng.below(2) == 0 {
                SdfEdit::sphere(center, rng.range(0.5, 1.5), op, smoothness)
            } else {
                SdfEdit::box_shape(
                    center,
                    [rng.range(0.5, 1.2), rng.range(0.5, 1.2), rng.range(0.5, 1.2)],
                    op,
                    smoothness,
                )
            };
            edits.push(e);
        }
        edits
    }

    /// The default near-field pointer grid baked from `field` — the SAME grid the GPU
    /// binds at binding 9 (origin/dims/brick_world the `FineMarcherPush` carries).
    fn build_default_grid(field: &SdfEditField) -> (PointerGrid, Vec<u32>) {
        let grid = PointerGrid::default_near_field();
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(field, &grid, &mut cells);
        (grid, cells)
    }

    /// The result of one primary-march replay: the hit decision + hit-`t`, plus the
    /// over-step audit signals (`min_field` = closest analytic approach SEEN at a probe,
    /// `crossed_undetected` = a brick-exit step that jumped from outside the hit band
    /// straight PAST the surface to a point where the field went NEGATIVE — the literal
    /// definition of a skipped surface, `exhausted` = ran the whole iteration budget).
    struct MarchTrace {
        hit: bool,
        min_field: f32,
        crossed_undetected: bool,
        exhausted: bool,
    }

    /// Replays the brick-ON / analytic PRIMARY march (the empty-skip loop, no re-march or
    /// shade) and audits it for an OVER-STEP. A verbatim mirror of
    /// `golden_composite_pixel_brick`'s primary loop, plus: before each brick-exit step it
    /// records the field at the PRE- and POST-step points; if the pre-step field was
    /// positive (outside the solid) and the post-step field is NEGATIVE (inside), the
    /// brick step JUMPED PAST a surface undetected (a skip → soundness BLOCKER). ORTHO,
    /// no mesh.
    fn march_primary(
        edits: &[SdfEdit],
        px: u32,
        py: u32,
        grid: &PointerGrid,
        cells: &[u32],
        brick_on: bool,
    ) -> MarchTrace {
        use super::super::{SDF_EPS, SDF_MAX_IT, SDF_T_MAX, composite_ray, sdf_edit_list};
        let (ro, rd) = composite_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
        let mut t = 0.0_f32;
        let mut hit = false;
        let mut min_field = f32::INFINITY;
        let mut crossed_undetected = false;
        let mut iters = 0u32;
        for _ in 0..SDF_MAX_IT {
            iters += 1;
            let p = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
            if brick_on
                && let Some((class, cmin)) = host_brick_cell(grid, cells, p)
                && class == EMPTY_OUTSIDE
            {
                // Audit the brick-exit step for an over-step: sample the analytic field
                // at the PRE-step point and at the POST-step point. A skip would show as
                // pre >= 0 (outside) but post < 0 (inside) — the step crossed a surface.
                let pre_d = sdf_edit_list(edits, p);
                min_field = min_field.min(pre_d);
                let exit = dist_to_brick_exit(p, rd, cmin, grid.brick_world);
                t += exit;
                let q = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
                let post_d = sdf_edit_list(edits, q);
                if pre_d >= 0.0 && post_d < 0.0 {
                    crossed_undetected = true;
                }
                min_field = min_field.min(post_d);
                if t > SDF_T_MAX {
                    break;
                }
                continue;
            }
            let d = sdf_edit_list(edits, p);
            min_field = min_field.min(d);
            if d < SDF_EPS {
                hit = true;
                break;
            }
            t += d;
            if t > SDF_T_MAX {
                break;
            }
        }
        MarchTrace {
            hit,
            min_field,
            crossed_undetected,
            exhausted: iters == SDF_MAX_IT && !hit,
        }
    }

    /// Max per-channel difference between two packed `0xAABBGGRR` colors (RGB only).
    fn chan_delta(a: u32, b: u32) -> i32 {
        let c = |x: u32, sh: u32| ((x >> sh) & 0xFF) as i32;
        (c(a, 0) - c(b, 0))
            .abs()
            .max((c(a, 8) - c(b, 8)).abs())
            .max((c(a, 16) - c(b, 16)).abs())
    }

    /// A sparse-but-representative pixel battery across the 64×64 frame (corners, edges,
    /// center, and a diagonal sweep) — exercises HIT, MISS, and the sphere-edge grazing
    /// rays without folding the field on all 4096 pixels in the per-scene inner loops.
    fn pixel_battery() -> Vec<(u32, u32)> {
        let mut v = Vec::new();
        let coords = [0u32, 1, 8, 16, 24, 31, 32, 40, 48, 56, 62, 63];
        for &py in &coords {
            for &px in &coords {
                v.push((px, py));
            }
        }
        // A diagonal sweep for extra edge coverage.
        for i in 0..SDF_IMG_W {
            v.push((i, i % SDF_IMG_H));
        }
        v
    }

    // ── 6. PUSH-CONSTANT LAYOUT (the std430 host/GPU agreement guard) ──────

    /// The M1 `FineMarcherPush` field offsets match the std430/HLSL layout the dev
    /// SPIR-V-verified (`grid_origin@32, brick_enabled@44, grid_dims@48, brick_world@60`)
    /// and the block is 64 bytes — a runtime mirror of the `const _: () = assert!` pins
    /// so a future reorder that desyncs the GPU push is caught even if the const-asserts
    /// are ever weakened.
    #[test]
    fn fine_marcher_push_m1_field_offsets_match_std430() {
        assert_eq!(core::mem::offset_of!(FineMarcherPush, grid_origin), 32, "grid_origin @32");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_enabled), 44, "brick_enabled @44");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, grid_dims), 48, "grid_dims @48");
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_world), 60, "brick_world @60");
        // M2 widened the push to the full 80-byte COMPOSITE range (brick_trilinear @64 + _pad3 @68).
        assert_eq!(core::mem::offset_of!(FineMarcherPush, brick_trilinear), 64, "brick_trilinear @64");
        assert_eq!(GBUFFER_MARCHER_PUSH_BYTES, 80, "FineMarcherPush must be 80 bytes");
    }

    /// `with_brick` flips `brick_enabled` to 1 and stamps the grid uniforms, preserving
    /// the base gates; `new` leaves the M1 block OFF (brick_enabled == 0, zero grid).
    #[test]
    fn with_brick_sets_grid_uniforms_and_preserves_base_gates() {
        let base = FineMarcherPush::new(true, 1.3, LIGHTING_FLAG_SHADOWS, [0.1, 0.2, 0.3]);
        assert_eq!(base.brick_enabled, 0, "new() leaves the empty-skip OFF");
        assert_eq!(base.grid_dims, [0, 0, 0], "new() zeroes the grid");

        let with = base.with_brick([-4.0, -4.0, -4.0], [16, 16, 16], 0.5);
        assert_eq!(with.brick_enabled, 1, "with_brick turns the empty-skip ON");
        assert_eq!(with.grid_origin, [-4.0, -4.0, -4.0], "grid_origin stamped");
        assert_eq!(with.grid_dims, [16, 16, 16], "grid_dims stamped");
        assert_eq!(with.brick_world, 0.5, "brick_world stamped");
        // Base gates preserved.
        assert_eq!(with.coarse_enabled, 1, "coarse gate preserved");
        assert_eq!(with.omega, 1.3, "omega preserved");
        assert_eq!(with.lighting_flags, LIGHTING_FLAG_SHADOWS, "lighting flags preserved");
        assert_eq!(with.light_dir, [0.1, 0.2, 0.3], "light_dir preserved");
    }

    /// The host `EMPTY_OUTSIDE` code the empty-skip branches on equals the
    /// `BrickClass::EmptyOutside` discriminant the bake stores — a drift in the enum
    /// repr would make the skip act on the wrong class (or never).
    #[test]
    fn class_codes_match_brickclass_discriminants() {
        assert_eq!(BrickClass::EmptyOutside as u32, EMPTY_OUTSIDE, "EmptyOutside == 0");
        assert_eq!(BrickClass::EmptyInside as u32, 1, "EmptyInside == 1");
        assert_eq!(BrickClass::Surface as u32, 2, "Surface == 2");
    }

    // ── 1. OFF BYTE-IDENTICAL (the 0%-gate) ────────────────────────────────

    /// `golden_composite_pixel_brick(brick_enabled=0)` is BIT-FOR-BIT the pre-M1
    /// `golden_composite_pixel_ex_omega_lit` over a battery of pixels across the demo
    /// scenes (crater / box / smooth-union) under both ORTHO and a perspective camera,
    /// at omega 1.0 and the default omega, and both lighting OFF and ON. Any single
    /// byte difference is a BLOCKER — the 0%-gate is broken.
    #[test]
    fn off_path_is_byte_identical_to_pre_m1_golden() {
        let scenes = [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        // The grid is supplied but MUST be ignored on the OFF path (a degenerate grid
        // proves the OFF path never reads it).
        let dummy_grid = PointerGrid::default_near_field();
        let dummy_cells = vec![0u32; dummy_grid.cell_count()];

        let cameras = [
            ("ortho", CompositeCamera::Ortho),
            (
                "persp",
                CompositeCamera::Perspective {
                    eye: [0.0, 0.0, 2.0],
                    forward: [0.0, 0.0, -1.0],
                    right: [1.0, 0.0, 0.0],
                    up: [0.0, 1.0, 0.0],
                    tan_half_fov: 0.5,
                    aspect: 1.0,
                },
            ),
        ];
        let omegas = [1.0_f32, DEFAULT_MARCHER_OMEGA];
        let light_cfgs = [
            (0u32, DEFAULT_LIGHT_DIR),
            (LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, [0.3, 0.7, 1.0]),
        ];
        let mesh_depths = [MESH_DEPTH_CLEAR, 0.5_f32]; // no-mesh + a covering mesh

        let mut checked = 0u64;
        for (sname, edits) in &scenes {
            for &(cname, cam) in &cameras {
                for &om in &omegas {
                    for &(flags, ldir) in &light_cfgs {
                        for &md in &mesh_depths {
                            for &(px, py) in &pixel_battery() {
                                let off = golden_composite_pixel_brick(
                                    edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, om, flags, ldir,
                                    false, &dummy_grid, &dummy_cells,
                                );
                                let pre_m1 = golden_composite_pixel_ex_omega_lit(
                                    edits, md, px, py, SDF_IMG_W, SDF_IMG_H, cam, om, flags, ldir,
                                );
                                assert_eq!(
                                    off, pre_m1,
                                    "[{sname}/{cname} ω={om} flags={flags} md={md}] OFF path \
                                     diverged from pre-M1 at ({px},{py}): brick 0x{off:08X} \
                                     pre-M1 0x{pre_m1:08X} — the 0%-gate is BROKEN"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        // 3 scenes × 2 cameras × 2 ω × 2 light cfgs × 2 mesh depths × |battery| = 9984.
        assert!(checked > 9_000, "OFF gate must exercise a wide battery (got {checked})");
        println!("[M1 OFF 0%-gate] {checked} pixels byte-identical to pre-M1 golden");
    }

    // ── 2. ON HIT-SET == ANALYTIC (the load-bearing M1 property) ───────────

    /// THE LOAD-BEARING M1 PROPERTY. Over the demo scene + ≥500 random scenes, asserts
    /// the empty-skip is SOUND and behavior-identical at the SHIPPING level:
    ///
    ///   (a) NO OVER-STEP (the direct soundness invariant, budget-INDEPENDENT): not one
    ///       brick-exit step ever jumps from OUTSIDE the solid (pre-step field >= 0)
    ///       straight to INSIDE it (post-step field < 0) — the literal definition of a
    ///       skipped surface. This is the conservative-classifier contract: an
    ///       EmptyOutside brick has no surface within band_half, so stepping to its exit
    ///       cannot cross one. A single `crossed_undetected` is a soundness BLOCKER.
    ///
    ///   (b) PRODUCTION HIT-SET + COLOR (the shipping contract): the production
    ///       `golden_composite_pixel_brick` (WITH its `exhausted` re-march fallback, the
    ///       same the GPU shader runs) yields ON output within ±1/255 of analytic — the
    ///       converged-`t` < `SDF_EPS` rounding is the only difference; tighter than the
    ///       established ±2/255 GPU-golden tolerance.
    ///
    /// The PRIMARY-loop hit-class is also tracked: any divergence there is a `MAX_IT`-cliff
    /// budget-edge artifact on a near-tangent ray (NOT an over-step), and the test asserts
    /// EVERY such pixel is resolved to ±1/255 by the production re-march (so the artifact
    /// is provably non-shipping).
    #[test]
    fn on_hit_set_equals_analytic_over_many_scenes() {
        const SCENES: u64 = 600; // ≥500 random scenes + the demo scenes below
        let battery = pixel_battery();

        let demo: [(&str, Vec<SdfEdit>); 3] =
            [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        let mut overstep_blockers: u64 = 0;
        let mut primary_budget_flips: u64 = 0;
        let mut checked: u64 = 0;
        let mut max_chan: i32 = 0;
        let mut first_overstep: Option<String> = None;
        let mut first_color_violation: Option<String> = None;
        let mut first_unresolved_flip: Option<String> = None;
        let mut unresolved_flips: u64 = 0;

        let mut run_scene = |label: &str, edits: &[SdfEdit]| {
            let field = field_of(edits);
            let (grid, cells) = build_default_grid(&field);
            for &(px, py) in &battery {
                checked += 1;
                let on_trace = march_primary(edits, px, py, &grid, &cells, true);
                let an_trace = march_primary(edits, px, py, &grid, &cells, false);

                // (a) the direct over-step soundness invariant (budget-independent).
                if on_trace.crossed_undetected {
                    overstep_blockers += 1;
                    if first_overstep.is_none() {
                        first_overstep = Some(format!(
                            "[{label}] ({px},{py}) a brick-exit step crossed a surface \
                             undetected (min_field={:.4e}); edits={edits:?}",
                            on_trace.min_field
                        ));
                    }
                }

                // (b) the PRODUCTION shipping contract: ON within ±1/255 of analytic.
                // Lighting OFF: bare Lambert (the ON lighting path is ±3/255 vs the
                // shader and is gated separately).
                let on = golden_composite_pixel_brick(
                    edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, true, &grid, &cells,
                );
                let analytic = golden_composite_pixel_brick(
                    edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho,
                    1.0, 0, DEFAULT_LIGHT_DIR, false, &grid, &cells,
                );
                let dchan = chan_delta(on, analytic);
                max_chan = max_chan.max(dchan);
                if dchan > 1 && first_color_violation.is_none() {
                    first_color_violation = Some(format!(
                        "[{label}] ({px},{py}) per-channel Δ={dchan} > 1/255 \
                         (ON 0x{on:08X} analytic 0x{analytic:08X})"
                    ));
                }

                // Track primary-loop hit-class flips and PROVE each is a budget artifact
                // (the production function resolves it to ±1/255 via the re-march).
                if on_trace.hit != an_trace.hit {
                    primary_budget_flips += 1;
                    // A genuine artifact has at least one path exhausting the budget AND
                    // no over-step; the production function must reconcile it.
                    if dchan > 1 {
                        unresolved_flips += 1;
                        if first_unresolved_flip.is_none() {
                            first_unresolved_flip = Some(format!(
                                "[{label}] ({px},{py}) primary hit-flip NOT resolved by the \
                                 production re-march: ON-exhausted={} AN-exhausted={} dchan={dchan} \
                                 (ON 0x{on:08X} analytic 0x{analytic:08X})",
                                on_trace.exhausted, an_trace.exhausted
                            ));
                        }
                    }
                }
            }
        };

        for (label, edits) in &demo {
            run_scene(label, edits);
        }
        for seed in 0..SCENES {
            let mut rng = XorShift64::new(seed.wrapping_mul(0x100_0001).wrapping_add(1));
            let edits = random_scene(&mut rng);
            run_scene(&format!("rand#{seed}"), &edits);
        }

        // (a) the SOUNDNESS gate: zero over-steps (no surface skipped).
        assert_eq!(
            overstep_blockers, 0,
            "{overstep_blockers}/{checked} brick-exit steps crossed a surface undetected — \
             a SKIPPED surface (SOUNDNESS BLOCKER). First: {}",
            first_overstep.unwrap_or_default()
        );
        // (b) the shipping contract: production ON within ±1/255 of analytic.
        assert!(
            max_chan <= 1,
            "production ON surface color exceeded ±1/255 vs analytic (max per-channel \
             Δ={max_chan}). First: {}",
            first_color_violation.unwrap_or_default()
        );
        // Every primary-loop hit-flip is a budget-edge artifact the production re-march
        // resolves to ±1/255 (none ships as a divergence).
        assert_eq!(
            unresolved_flips, 0,
            "{unresolved_flips} primary-loop hit-flips were NOT resolved by the production \
             re-march (a shipping divergence). First: {}",
            first_unresolved_flip.unwrap_or_default()
        );
        assert!(checked > 50_000, "ON gate must exercise a wide battery (got {checked})");
        println!(
            "[M1 ON-vs-analytic] {checked} pixels over {} scenes: 0 over-steps, \
             {primary_budget_flips} primary-loop budget-edge flips (ALL resolved by the \
             production re-march to ±{max_chan}/255 — non-shipping)",
            SCENES + 3
        );
    }

    // ── 3. EMPTY-SKIP NEVER SKIPS A SURFACE (the property, targeted) ───────

    /// A thin shell straddling an EMPTY/SURFACE brick boundary: every ray that hits the
    /// surface analytically must still hit with the empty-skip ON (no surface skipped),
    /// and every ray that misses analytically must still miss (no spurious hit). Asserts
    /// per-pixel HIT-classification equality on the boundary-grazing scene.
    #[test]
    fn empty_skip_never_skips_or_invents_a_surface_at_brick_boundary() {
        // A small sphere placed so its surface grazes a brick face of the default grid
        // (brick_world = 0.5; a center at a half-cell offset makes the surface cross a
        // cell boundary). Plus a thin box to graze a face from outside.
        let scenes: [(&str, Vec<SdfEdit>); 3] = [
            (
                "sphere_on_face",
                vec![SdfEdit::sphere([0.25, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)],
            ),
            (
                "thin_box_face_graze",
                vec![SdfEdit::box_shape([0.5, 0.0, 0.0], [0.5, 0.6, 0.6], sdf_op::UNION, 0.0)],
            ),
            (
                "off_center_csg",
                vec![
                    SdfEdit::sphere([0.5, 0.5, 0.0], 0.7, sdf_op::UNION, 0.0),
                    SdfEdit::sphere([0.75, 0.5, 0.0], 0.3, sdf_op::SUBTRACT, 0.0),
                ],
            ),
        ];

        for (label, edits) in &scenes {
            let field = field_of(edits);
            let (grid, cells) = build_default_grid(&field);
            // EVERY pixel of the frame (a thin-surface scene wants dense coverage).
            for py in 0..SDF_IMG_H {
                for px in 0..SDF_IMG_W {
                    // The PROPERTY: not one brick-exit step crosses a surface — no surface
                    // skipped (analytic-hit ⟹ no undetected crossing), no surface invented.
                    let on_trace = march_primary(edits, px, py, &grid, &cells, true);
                    assert!(
                        !on_trace.crossed_undetected,
                        "[{label}] ({px},{py}) a brick-exit step crossed a surface UNDETECTED \
                         (min_field={:.4e}) — a surface was SKIPPED",
                        on_trace.min_field
                    );
                    // The production composited color (with re-march) stays within ±1/255.
                    let on = golden_composite_pixel_brick(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, true, &grid, &cells,
                    );
                    let analytic = golden_composite_pixel_brick(
                        edits, MESH_DEPTH_CLEAR, px, py, SDF_IMG_W, SDF_IMG_H,
                        CompositeCamera::Ortho, 1.0, 0, DEFAULT_LIGHT_DIR, false, &grid, &cells,
                    );
                    assert!(
                        chan_delta(on, analytic) <= 1,
                        "[{label}] ({px},{py}) color Δ {} > 1/255 (ON 0x{on:08X} analytic \
                         0x{analytic:08X}) — the empty skip changed the surface",
                        chan_delta(on, analytic)
                    );
                }
            }
        }
    }

    // ── 4. `dist_to_brick_exit` PROGRESS (no zero/negative step) ───────────

    /// A ray parallel to a brick face, starting exactly on a boundary, or axis-aligned
    /// through cell corners still advances by >= `BRICK_EXIT_EPS` (no zero/negative step
    /// → no infinite march). Tests the degenerate directions head-on.
    #[test]
    fn dist_to_brick_exit_always_advances_on_degenerate_rays() {
        let cell_min = [0.0_f32, 0.0, 0.0];
        let bw = 0.5_f32;
        // Cases: (ro, rd, label). All `ro` are on/at a brick face or corner; all `rd`
        // include axis-parallel + fully-degenerate (zero) directions.
        let cases: &[([f32; 3], [f32; 3], &str)] = &[
            // Parallel to the +x face plane (no x component), on the y=0 face.
            ([0.1, 0.0, 0.1], [0.0, 1.0, 0.0], "parallel_y_on_face"),
            // Parallel to a face, sitting exactly on a corner.
            ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], "axis_z_from_corner"),
            // Diagonal through the cell corner (exits at a corner, can graze).
            ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "body_diagonal_from_corner"),
            // Fully degenerate: zero direction (every axis skipped).
            ([0.25, 0.25, 0.25], [0.0, 0.0, 0.0], "zero_direction"),
            // Sub-eps direction on all axes (every axis below BRICK_EXIT_EPS).
            ([0.25, 0.25, 0.25], [1e-6, 1e-6, 1e-6], "sub_eps_all_axes"),
            // A ray starting on the FAR face, pointing further out (negative exit
            // territory) — must clamp UP to advance.
            ([0.5, 0.25, 0.25], [1.0, 0.0, 0.0], "on_far_face_outward"),
            // Negative-x ray from inside (exits the lo face).
            ([0.25, 0.25, 0.25], [-1.0, 0.0, 0.0], "neg_x_from_center"),
        ];
        for &(ro, rd, label) in cases {
            let exit = dist_to_brick_exit(ro, rd, cell_min, bw);
            assert!(exit.is_finite(), "[{label}] exit must be finite, got {exit}");
            assert!(
                exit >= BRICK_EXIT_EPS,
                "[{label}] exit {exit} < BRICK_EXIT_EPS {BRICK_EXIT_EPS} — the march can stall (no progress)"
            );
        }
    }

    /// A well-conditioned ray through a brick exits at the analytically expected slab
    /// distance (the progress clamp does not corrupt a normal exit).
    #[test]
    fn dist_to_brick_exit_matches_slab_far_face_on_normal_ray() {
        let cell_min = [0.0_f32, 0.0, 0.0];
        let bw = 0.5_f32;
        // From the lo-x face center, straight +x: exits the +x face at t = 0.5.
        let exit = dist_to_brick_exit([0.0, 0.25, 0.25], [1.0, 0.0, 0.0], cell_min, bw);
        assert!((exit - 0.5).abs() < 1e-5, "axis-aligned exit must be the slab width 0.5, got {exit}");
        // From the center, +x: exits at t = 0.25 (half the cell).
        let exit2 = dist_to_brick_exit([0.25, 0.25, 0.25], [1.0, 0.0, 0.0], cell_min, bw);
        assert!((exit2 - 0.25).abs() < 1e-5, "centered exit must be 0.25, got {exit2}");
    }

    // ── 5. `build_pointer_grid` CORRECTNESS ────────────────────────────────

    /// Every cell the bake writes equals a direct `classify_brick` of that cell's AABB —
    /// the bake is a faithful per-cell fold of the authority (no index/origin slip).
    #[test]
    fn build_pointer_grid_matches_per_cell_classify() {
        let scenes: [(&str, Vec<SdfEdit>); 3] =
            [("crater", crater()), ("box", box_csg()), ("smooth", smooth_union())];
        for (label, edits) in &scenes {
            let field = field_of(edits);
            let grid = PointerGrid::default_near_field();
            let mut cells = vec![0u32; grid.cell_count()];
            build_pointer_grid(&field, &grid, &mut cells);

            let w = grid.dims[0];
            let h = grid.dims[1];
            let d = grid.dims[2];
            for iz in 0..d {
                for iy in 0..h {
                    for ix in 0..w {
                        let cell_min = grid.cell_min(ix, iy, iz);
                        let expect = classify_brick(
                            &field, cell_min, grid.brick_world, SDF_EDIT_BAND_HALF,
                        ) as u32;
                        let idx = (ix + iy * w + iz * w * h) as usize;
                        assert_eq!(
                            cells[idx], expect,
                            "[{label}] cell ({ix},{iy},{iz}) bake {} != classify {expect}",
                            cells[idx]
                        );
                    }
                }
            }
        }
    }

    /// A cell with no edit nearby bakes EmptyOutside (or EmptyInside deep in a solid); a
    /// cell a surface passes through bakes Surface. Checked against a hand-placed scene.
    #[test]
    fn build_pointer_grid_classifies_empty_vs_surface() {
        // A unit sphere at the origin. Cells far out are EmptyOutside; the cell at the
        // origin (deep inside the sphere) overlaps the sphere's AABB → Surface (the C2
        // conservative rule: a primitive's AABB covers its interior). A cell on the
        // sphere's band is Surface.
        let edits = vec![SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0)];
        let field = field_of(&edits);
        let (grid, cells) = build_default_grid(&field);

        let cell_class = |wx: f32, wy: f32, wz: f32| -> u32 {
            let p = [wx, wy, wz];
            let (class, _) = host_brick_cell(&grid, &cells, p).expect("point inside the default grid");
            class
        };

        // A corner of the [-4,4]³ grid, far from the unit sphere → EmptyOutside.
        assert_eq!(cell_class(-3.5, -3.5, -3.5), EMPTY_OUTSIDE, "far cell must be EmptyOutside");
        // A cell straddling the sphere surface (radius 1) → Surface (class 2).
        assert_eq!(cell_class(1.0, 0.0, 0.0), BrickClass::Surface as u32, "surface cell must be Surface");
        // The center cell overlaps the sphere AABB → Surface (conservative, not EmptyInside).
        assert_eq!(cell_class(0.0, 0.0, 0.0), BrickClass::Surface as u32, "deep-inside cell is Surface (C2)");
    }

    /// Grid indexing round-trips: `cell_min(ix,iy,iz)` then a point inside that cell maps
    /// back to `(ix,iy,iz)` via `host_brick_cell`, and an out-of-grid point returns None.
    #[test]
    fn host_brick_cell_round_trips_and_bounds_check() {
        let edits = crater();
        let field = field_of(&edits);
        let (grid, cells) = build_default_grid(&field);

        // Round-trip a sampling of cells: a point at the cell center maps back to it.
        for &(ix, iy, iz) in &[(0u32, 0u32, 0u32), (5, 7, 3), (15, 15, 15), (8, 0, 12)] {
            let cmin = grid.cell_min(ix, iy, iz);
            let center = [
                cmin[0] + grid.brick_world * 0.5,
                cmin[1] + grid.brick_world * 0.5,
                cmin[2] + grid.brick_world * 0.5,
            ];
            let (_, got_min) = host_brick_cell(&grid, &cells, center)
                .expect("cell-center point must land in the grid");
            assert_eq!(got_min, cmin, "cell ({ix},{iy},{iz}) center must map back to its cell_min");
        }

        // Out-of-grid points (below origin and past the far corner) → None.
        let below = [grid.origin[0] - 1.0, grid.origin[1], grid.origin[2]];
        assert!(host_brick_cell(&grid, &cells, below).is_none(), "point below origin → no cell");
        let far = [
            grid.origin[0] + grid.dims[0] as f32 * grid.brick_world + 1.0,
            grid.origin[1],
            grid.origin[2],
        ];
        assert!(host_brick_cell(&grid, &cells, far).is_none(), "point past the far face → no cell");
    }

    /// The default near-field grid spans the demo `[-4,4]³` extent (DEFAULT_GRID_DIM
    /// cells of DEFAULT_BRICK_WORLD), enclosing the demo scene with margin.
    #[test]
    fn default_near_field_grid_encloses_demo_extent() {
        let grid = PointerGrid::default_near_field();
        assert_eq!(grid.dims, [DEFAULT_GRID_DIM, DEFAULT_GRID_DIM, DEFAULT_GRID_DIM]);
        assert_eq!(grid.brick_world, DEFAULT_BRICK_WORLD);
        let half = DEFAULT_GRID_DIM as f32 * DEFAULT_BRICK_WORLD * 0.5;
        assert!((grid.origin[0] + half).abs() < 1e-6, "grid centered on origin (x)");
        // The demo primitives live within ±3; the grid spans ±4 → enclosed.
        assert!(half >= 4.0 - 1e-6, "default grid must span at least ±4 (got ±{half})");
    }

    /// The empty scene (no edits) bakes an ALL-EmptyOutside grid — every cell is
    /// provably outside, so the marcher skips the whole near-field to the analytic
    /// (background) result.
    #[test]
    fn build_pointer_grid_empty_scene_is_all_empty_outside() {
        let field = SdfEditField::new(); // no edits, gen 0
        let grid = PointerGrid::default_near_field();
        let mut cells = vec![0u32; grid.cell_count()];
        build_pointer_grid(&field, &grid, &mut cells);
        assert!(
            cells.iter().all(|&c| c == EMPTY_OUTSIDE),
            "an empty scene must bake an all-EmptyOutside grid"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// M4 — the CLIP-MAP LOD host CPU tests (the Slice-B gate). These are CPU-runnable
// (no Vulkan device): they prove (1) the per-level bake feeds the proven baker
// correctly (bit-identity vs a direct classify/fill reference, per level), (2) the
// OFF/N=1 UBO tail is byte-identical to the M2 default, (3) the full N=3 UBO array
// matches a hand-checked std140 array-of-structs golden. The GPU image tests are
// Slice C (RTX-gated).
// ════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod m4_clipmap_tests {
    use super::super::{
        AtlasEncoding, BrickLevelParams, M2GridParams, M4GridParams, M4LevelParams,
        M2_ATLAS_DIM, SdfEdit, atlas_voxel_index, bake_brick_atlas_at, m2_tile_atlas_origin, sdf_op,
    };
    use boyko_sdf_math::brick::{
        self, BRICK_ALLOC, BRICK_VOXELS, band_half_at_level, brick_world_at_level, c_max_at_level,
        classify_brick, decode_snorm8, fill_brick, snapped_level_origin, snapped_level_origin_cell,
        toroidal_slot, voxel_size_at_level,
    };
    use boyko_sdf_math::{BrickClass, SDF_EDIT_BAND_HALF, SdfEditField};

    /// The demo "crater" CSG scene (base sphere minus a smaller sphere) — the SAME field the M1/M2
    /// goldens use, so the per-level bake runs against the production demo authority.
    fn crater() -> Vec<SdfEdit> {
        vec![
            SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
            SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
        ]
    }

    fn field_of(edits: &[SdfEdit]) -> SdfEditField {
        let mut f = SdfEditField::new();
        for e in edits {
            assert!(f.push(*e), "scene must fit MAX_SDF_EDITS");
        }
        f.bump_gen();
        f
    }

    /// A DIRECT CPU reference bake of the `M2_ATLAS_DIM³` `Snorm8` atlas for one level's geometry,
    /// independent of `bake_brick_atlas_at`: classify each `M2_GRID_DIM³` cell, fill a SURFACE cell's
    /// apron'd tile, scatter at the cell's TOROIDAL atlas-voxel slot (M5: `toroidal_slot(origin_cell +
    /// cell)`; EMPTY cells leave 0). The bit-exact oracle the level-aware baker must match (the M3
    /// full-bake bit-identity gate, per level). `origin_cell == round(origin/brick_world)`.
    fn reference_atlas_snorm8(
        field: &SdfEditField,
        origin: [f32; 3],
        origin_cell: [i32; 3],
        brick_world: f32,
        voxel_size: f32,
        band_half: f32,
        c_max: f32,
    ) -> Vec<u8> {
        const W: usize = BRICK_ALLOC;
        let dim = brick::M2_GRID_DIM;
        let mut out = vec![0u8; (M2_ATLAS_DIM as usize).pow(3)];
        let mut tile = [0i8; BRICK_VOXELS];
        for cz in 0..dim {
            for cy in 0..dim {
                for cx in 0..dim {
                    let cell_min = [
                        origin[0] + cx as f32 * brick_world,
                        origin[1] + cy as f32 * brick_world,
                        origin[2] + cz as f32 * brick_world,
                    ];
                    let class = classify_brick(field, cell_min, brick_world, band_half);
                    let is_surface = class == BrickClass::Surface;
                    if is_surface {
                        fill_brick(field, cell_min, voxel_size, band_half, c_max, &mut tile);
                    }
                    let slot = toroidal_slot([
                        origin_cell[0] + cx as i32,
                        origin_cell[1] + cy as i32,
                        origin_cell[2] + cz as i32,
                    ]);
                    let [ox, oy, oz] = m2_tile_atlas_origin(slot);
                    for lz in 0..W {
                        for ly in 0..W {
                            for lx in 0..W {
                                let byte =
                                    if is_surface { tile[lx + ly * W + lz * W * W] } else { 0i8 };
                                let vi = atlas_voxel_index(
                                    ox + lx as u32,
                                    oy + ly as u32,
                                    oz + lz as u32,
                                );
                                out[vi] = byte as u8;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Per-level bake feeds the proven baker. For each clip-map level `L = 0..BRICK_LEVELS`, the
    /// level-aware [`bake_brick_atlas_at`] staging at the level's snapped origin / `*_at_level`
    /// brick/voxel/band is BIT-IDENTICAL to the direct `classify_brick`/`fill_brick` reference over
    /// the SAME level grid — proving the Slice-A level table threads correctly into the M3-proven
    /// per-cell baker at every level.
    #[test]
    fn m4_level_bake_equals_full_classify_fill() {
        let camera = [0.37, -1.2, 2.0];
        let field = field_of(&crater());
        for level in 0..brick::BRICK_LEVELS as u32 {
            let geo = BrickLevelParams::at_level(camera, level);
            let mut baked = vec![0u8; (M2_ATLAS_DIM as usize).pow(3)];
            bake_brick_atlas_at(&field, AtlasEncoding::Snorm8, &geo, &mut baked);

            let reference = reference_atlas_snorm8(
                &field,
                snapped_level_origin(camera, level),
                snapped_level_origin_cell(camera, level),
                brick_world_at_level(level),
                voxel_size_at_level(level),
                band_half_at_level(level),
                c_max_at_level(level),
            );
            assert_eq!(
                baked, reference,
                "level {level}: bake_brick_atlas_at diverged from the direct classify/fill reference"
            );
            // The decoded snorm round-trip is well-defined (a sanity tap on the oracle).
            let _ = decode_snorm8(0, SDF_EDIT_BAND_HALF);
        }
    }

    /// The OFF/N=1 keystone: `near_field_only().as_ubo_bytes()[..48]` is byte-for-byte equal to
    /// `M2GridParams::default_near_field().as_bytes()` — a single-level (OFF) clip-map writes exactly
    /// the M2 tail, so the M2 path is unchanged when the clip-map is OFF.
    #[test]
    fn m4_ubo_bytes_off_path_byte_identical() {
        let m4 = M4GridParams::near_field_only();
        let m4_bytes = m4.as_ubo_bytes();
        let m2 = M2GridParams::default_near_field();
        let m2_bytes = m2.as_bytes();
        assert_eq!(m2_bytes.len(), 48, "M2 tail is 48 bytes");
        assert_eq!(
            &m4_bytes[..48],
            m2_bytes,
            "OFF/N=1 keystone: M4 level-0 block must equal the M2 default tail byte-for-byte"
        );
    }

    /// The std140 array-of-structs golden: the full N-level `as_ubo_bytes` matches a hand-checked
    /// layout where level `L` sits at byte `L*48`, lane 0 `origin_brick_world` at +0, lane 1
    /// `dims_atlas_dim` at +16, lane 2 `band_voxel_inv_atlas` at +32, each lane four little-endian
    /// `f32`s. This pins the exact byte layout the Slice-C shader's `m2_levels[BRICK_LEVELS]` reads.
    #[test]
    fn m4_grid_params_layout_golden() {
        let camera = [0.37, -1.2, 2.0];
        let m4 = M4GridParams::camera_centered(camera);
        let bytes = m4.as_ubo_bytes();
        assert_eq!(bytes.len(), brick::BRICK_LEVELS * 48);

        let read_f32 = |off: usize| -> f32 {
            f32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
        };

        for level in 0..brick::BRICK_LEVELS {
            let base = level * 48;
            let origin = snapped_level_origin(camera, level as u32);
            let bw = brick_world_at_level(level as u32);
            let band = band_half_at_level(level as u32);
            let voxel = voxel_size_at_level(level as u32);

            // Lane 0 (origin_brick_world) at +0.
            assert_eq!(read_f32(base), origin[0], "L{level} origin.x at byte {base}");
            assert_eq!(read_f32(base + 4), origin[1], "L{level} origin.y");
            assert_eq!(read_f32(base + 8), origin[2], "L{level} origin.z");
            assert_eq!(read_f32(base + 12), bw, "L{level} brick_world at lane0.w");
            // Lane 1 (dims_atlas_dim) at +16 — level-invariant dims/atlas.
            assert_eq!(read_f32(base + 16), brick::M2_GRID_DIM as f32, "L{level} dims.x");
            assert_eq!(read_f32(base + 20), brick::M2_GRID_DIM as f32, "L{level} dims.y");
            assert_eq!(read_f32(base + 24), brick::M2_GRID_DIM as f32, "L{level} dims.z");
            assert_eq!(read_f32(base + 28), M2_ATLAS_DIM as f32, "L{level} atlas_dim at lane1.w");
            // Lane 2 (band_voxel_inv_atlas) at +32.
            assert_eq!(read_f32(base + 32), band, "L{level} band_half at lane2.x");
            assert_eq!(read_f32(base + 36), voxel, "L{level} voxel_size at lane2.y");
            assert_eq!(read_f32(base + 40), 1.0 / M2_ATLAS_DIM as f32, "L{level} inv_atlas at lane2.z");
            assert_eq!(read_f32(base + 44), level as f32, "L{level} level index at lane2.w");
        }
    }

    /// The `M4LevelParams` struct is exactly one M2 lane block (48 B) — the array packs contiguously.
    #[test]
    fn m4_level_params_is_48_bytes() {
        assert_eq!(core::mem::size_of::<M4LevelParams>(), 48);
        assert_eq!(core::mem::size_of::<M4GridParams>(), brick::BRICK_LEVELS * 48);
    }
}

/// Render P7 GROUP B — the `GoldenLightHeader` `ssao_mode` (header word 11) accessor
/// round-trip + the 0%-gate default. The SSAO host-oracle gather tests (the deep-crevice /
/// flat / seam AO bands) live in [`ssao_gather_tests`] below: the lib re-derives the SSAO
/// math as PLAIN RUST ([`crate::goldens::golden_ssao_attributes`]), since `boyko_shaderdsl` is a
/// dev-dependency only and the shipped backend must not link the eDSL. The `ssao_edsl_sync`
/// integration test (which HAS the dev-dep) cross-checks the plain-Rust per-tap horizon
/// against `boyko_shaderdsl::ssao::ssao_horizon_step_body::<EvalCf>` before any GPU run.
#[cfg(test)]
mod ssao_header_tests {
    use crate::goldens::GoldenLightHeader;

    /// `ssao_mode` (header word 11 = `sky_spec.w`) reads 0 on a freshly-built header — the
    /// automatic 0%-gate: every pre-P7 scene carries `sky_spec.w == 0.0`, so the resolve's
    /// `if (ssao_mode != 0u)` combine is skipped and `ao_final == gMaterial.g` byte-for-byte.
    #[test]
    fn ssao_mode_defaults_to_zero() {
        let h = GoldenLightHeader::new(1, 0, 1.0);
        assert_eq!(h.ssao_mode(), 0, "ssao_mode (word 11) must be 0 by default (the 0%-gate)");
        // The clustered constructor writes the cluster_params lane (words 12..15), NOT sky_spec
        // — so word 11 must still read 0.
        let cfg = crate::goldens::GoldenClusterConfig {
            dim_x: 16,
            dim_y: 9,
            dim_z: 24,
            max_lights_per_cluster: 64,
            z_near: 0.1,
            z_far: 100.0,
        };
        let hc = GoldenLightHeader::new_clustered(1, 0, 1.0, &cfg);
        assert_eq!(hc.ssao_mode(), 0, "new_clustered must not disturb ssao_mode (word 11)");
    }

    /// `with_ssao_mode(m)` round-trips through `ssao_mode()` for every representative `m`
    /// (stored BIT-CAST in `sky_spec.w`, exactly like `with_shadow_mode`/`shadow_mode` for
    /// word 7), and does NOT disturb the shadow_mode word (word 7).
    #[test]
    fn ssao_mode_round_trips_through_with_ssao_mode() {
        for m in [0u32, 1, 2, 0xFFFF_FFFF] {
            let h = GoldenLightHeader::new(1, 0, 1.0).with_ssao_mode(m);
            assert_eq!(h.ssao_mode(), m, "with_ssao_mode({m}) must round-trip through ssao_mode()");
        }
        // Independence: setting ssao_mode (word 11) leaves shadow_mode (word 7) untouched and
        // vice-versa — the two are distinct header words (sky_spec.w vs sky_diffuse.w).
        let both = GoldenLightHeader::new(1, 0, 1.0)
            .with_shadow_mode(1)
            .with_ssao_mode(1);
        assert_eq!(both.shadow_mode(), 1, "with_ssao_mode must not clobber shadow_mode (word 7)");
        assert_eq!(both.ssao_mode(), 1, "with_shadow_mode must not clobber ssao_mode (word 11)");
    }

    /// Render Shadow Phase 3 — the `contact_shadow_mode` (header word 7 BIT 1) builder + reader.
    /// Proves: `with_contact_shadow_mode(true)` round-trips to `contact_shadow_mode() == 1`;
    /// `with_shadow_mode(1).with_contact_shadow_mode(true)` keeps BOTH bits independent
    /// (`shadow_mode() == 1 && contact_shadow_mode() == 1`); and `with_contact_shadow_mode(false)`
    /// leaves word 7 unchanged on a fresh header (the 0%-gate proof — BIT 1 already 0).
    #[test]
    fn contact_shadow_mode_packs_into_word7_bit1() {
        let on = GoldenLightHeader::new(1, 0, 1.0).with_contact_shadow_mode(true);
        assert_eq!(on.contact_shadow_mode(), 1, "with_contact_shadow_mode(true) must read back 1");

        // Bit independence: shadow_mode (bit 0) and contact_shadow_mode (bit 1) coexist in word 7.
        let both = GoldenLightHeader::new(1, 0, 1.0)
            .with_shadow_mode(1)
            .with_contact_shadow_mode(true);
        assert_eq!(both.shadow_mode(), 1, "contact bit must not clobber shadow_mode (word 7 bit 0)");
        assert_eq!(both.contact_shadow_mode(), 1, "shadow_mode bit must not clobber contact (bit 1)");

        // Order independence: setting contact first then shadow_mode keeps both.
        let both_rev = GoldenLightHeader::new(1, 0, 1.0)
            .with_contact_shadow_mode(true)
            .with_shadow_mode(1);
        assert_eq!(both_rev.shadow_mode(), 1, "with_shadow_mode must preserve the contact bit");
        assert_eq!(both_rev.contact_shadow_mode(), 1, "the contact bit must survive with_shadow_mode");

        // 0%-gate: `with_contact_shadow_mode(false)` on a fresh header leaves word 7 byte-unchanged
        // (BIT 1 was already 0), so every pre-Phase-3 scene reads contact_shadow_mode() == 0.
        let fresh = GoldenLightHeader::new(1, 0, 1.0);
        let off = fresh.with_contact_shadow_mode(false);
        assert_eq!(
            off.sky_diffuse[3].to_bits(),
            fresh.sky_diffuse[3].to_bits(),
            "with_contact_shadow_mode(false) must leave word 7 unchanged (the 0%-gate)"
        );
        assert_eq!(off.contact_shadow_mode(), 0, "a fresh header reads contact_shadow_mode() == 0");
    }
}

/// Render P7 GROUP C1 — the resolve SSAO combine 0%-gate. Proves that on a `ssao_mode() == 0`
/// scene (every pre-P7 scene) the SSAO-aware resolve mirrors
/// ([`golden_deferred_resolve_table_ssao`] / [`golden_deferred_resolve_table_shadowed_ssao`])
/// return BYTE-IDENTICAL output for ANY `ssao` argument — i.e. the combine is never taken and
/// `ao_final == attrs.ao`, so wiring the SSAO term in is a true 0%-gate. A positive control
/// asserts that `ssao_mode == 1` with a darkening SSAO term DOES change a lit SDF pixel (the
/// combine is actually wired, not dead).
#[cfg(test)]
mod ssao_resolve_combine_tests {
    use super::super::{PBR_SKY_DIFFUSE};
    use crate::goldens::{golden_deferred_resolve_table, golden_deferred_resolve_table_shadowed, golden_deferred_resolve_table_shadowed_ssao, golden_deferred_resolve_table_ssao, ssao_combine, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes};

    const RO_ZERO: [f32; 3] = [0.0, 0.0, 0.0];

    /// A representative one-material table (slot 0 = a textured dielectric).
    fn materials() -> Vec<GoldenMaterial> {
        vec![GoldenMaterial::new([0.8, 0.6, 0.4, 1.0], 0.0, 0.5, 0.5, [0.0, 0.0, 0.0])]
    }

    /// The degenerate directional + sky table at `exposure`, with the supplied `ssao_mode`
    /// (header word 11). The sky entry drives the ambient term the AO modulates.
    fn table(ssao_mode: u32) -> (GoldenLightHeader, Vec<GoldenLight>) {
        let header = GoldenLightHeader::new(2, 0, 1.0).with_ssao_mode(ssao_mode);
        let lights = vec![
            GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
            GoldenLight::sky(PBR_SKY_DIFFUSE, PBR_SKY_DIFFUSE),
        ];
        (header, lights)
    }

    /// A small sweep of SDF-lit + mesh-sentinel attributes (the combine reads `mask`, `ao`,
    /// and the `view_t >= 1e30` mesh sentinel).
    fn sweep() -> Vec<MarcherAttributes> {
        let mut v = Vec::new();
        for &mask in &[1u8, 0u8] {
            for &ao in &[0u8, 90u8, 200u8, 255u8] {
                for &view_t in &[2.5_f32, 1.0e30] {
                    v.push(MarcherAttributes {
                        base_rgb: [180, 120, 90],
                        oct_rg: [200, 60],
                        mat_id: 0,
                        shadow: 200,
                        ao,
                        mask,
                        view_t,
                    });
                }
            }
        }
        v
    }

    #[test]
    fn ssao_combine_is_identity_when_off() {
        // The pure host combine: `ssao_mode == 0` returns `ao` regardless of `view_t`/`ssao`.
        for &ao in &[0.0_f32, 0.35, 1.0] {
            for &view_t in &[2.5_f32, 1.0e30] {
                for &ssao in &[0.0_f32, 0.5, 1.0] {
                    assert_eq!(
                        ssao_combine(0, ao, view_t, ssao),
                        ao,
                        "ssao_mode==0 must return ao unchanged (the 0%-gate)"
                    );
                }
            }
        }
    }

    #[test]
    fn resolve_table_ssao_off_is_byte_identical() {
        // The SSAO-aware resolve with `ssao_mode == 0` is byte-identical to the pre-P7 fn for
        // EVERY `ssao` argument and EVERY swept attribute — the resolve 0%-gate.
        let mats = materials();
        let (header, lights) = table(0);
        let rd = [0.1, 0.05, -0.99];
        for attrs in sweep() {
            let baseline = golden_deferred_resolve_table(attrs, RO_ZERO, rd, &mats, &header, &lights);
            for &ssao in &[0.0_f32, 0.25, 0.7, 1.0] {
                let got = golden_deferred_resolve_table_ssao(
                    attrs, RO_ZERO, rd, &mats, &header, &lights, ssao,
                );
                assert_eq!(
                    got, baseline,
                    "ssao_mode==0 resolve must be byte-identical for any ssao ({ssao})"
                );
            }
        }
    }

    #[test]
    fn resolve_table_shadowed_ssao_off_is_byte_identical() {
        // The SHADOWED SSAO-aware resolve with `ssao_mode == 0` (AND `shadow_mode == 0`, so no
        // march fires) is byte-identical to the pre-P7 shadowed fn for every `ssao`.
        let mats = materials();
        let (header, lights) = table(0);
        let rd = [0.1, 0.05, -0.99];
        // A trivial field (never marched on the `shadow_mode == 0` / `ssao_mode == 0` path).
        let field = |_q: [f32; 3]| 1.0_f32;
        for attrs in sweep() {
            let baseline = golden_deferred_resolve_table_shadowed(
                attrs, RO_ZERO, rd, &mats, &header, &lights, &field,
            );
            for &ssao in &[0.0_f32, 0.25, 0.7, 1.0] {
                let got = golden_deferred_resolve_table_shadowed_ssao(
                    attrs, RO_ZERO, rd, &mats, &header, &lights, &field, ssao,
                );
                assert_eq!(
                    got, baseline,
                    "ssao_mode==0 shadowed resolve must be byte-identical for any ssao ({ssao})"
                );
            }
        }
    }

    #[test]
    fn resolve_table_ssao_on_darkens_a_lit_sdf_pixel() {
        // Positive control: `ssao_mode == 1` with a strongly-occluding SSAO term (0.0) must
        // darken a fully-unoccluded lit SDF pixel (ao = 255, view_t finite) — the combine is
        // wired, not dead. A mesh pixel (view_t sentinel) takes `min(1.0, ssao) == ssao`.
        let mats = materials();
        let (header, lights) = table(1);
        let rd = [0.1, 0.05, -0.99];
        let attrs = MarcherAttributes {
            base_rgb: [180, 120, 90],
            oct_rg: [200, 60],
            mat_id: 0,
            shadow: 255,
            ao: 255,
            mask: 1,
            view_t: 2.5,
        };
        let unoccluded =
            golden_deferred_resolve_table_ssao(attrs, RO_ZERO, rd, &mats, &header, &lights, 1.0);
        let occluded =
            golden_deferred_resolve_table_ssao(attrs, RO_ZERO, rd, &mats, &header, &lights, 0.0);
        assert_ne!(
            occluded, unoccluded,
            "ssao_mode==1 with a darkening SSAO term must change the lit pixel (combine wired)"
        );
    }
}

/// Render P7 GROUP B — the SSAO host-oracle gather bands. Builds a SYNTHETIC G-buffer
/// (`Vec<MarcherAttributes>`; [`golden_ssao_attributes`] reads `mask` + `view_t` + the
/// center pixel's `oct_rg` normal) at the legacy `64×64` ORTHO extent and asserts the
/// signature AO regimes for the FIXED horizon math (elevation above the tangent plane):
/// a FLAT lit surface (in-plane neighbours, `delta ⊥ N`) stays at AO ≈ 1 (the bug's
/// regression guard — the old screen-direction math BLACKENED it), a deep crevice
/// (neighbours rising above the tangent toward the camera) darkens AO below 1, and a seam
/// (all neighbours background) is fully unoccluded with no NaN. The math is PLAIN RUST (the
/// lib does not link the `boyko_shaderdsl` dev-dep); the `ssao_edsl_sync` integration test
/// cross-checks the per-tap horizon against the eDSL Eval.
#[cfg(test)]
mod ssao_gather_tests {
    use super::super::{CompositeCamera, SsaoParams, SSAO_VIEWT_BG};
    use crate::goldens::{oct_decode, MarcherAttributes, golden_ssao_attributes};

    const W: u32 = 64;
    const H: u32 = 64;
    const CX: u32 = 32;
    const CY: u32 = 32;
    /// The octahedral-quantized center normal the synthetic gbuffer stamps. `[128, 128]`
    /// decodes to ~`+z` (toward the ORTHO camera, which looks down `-z`) — the surface faces
    /// the viewer, the realistic lit-pixel normal.
    const OCT_RG: [u8; 2] = [128, 128];

    /// The decoded center surface normal `N` (the same `oct_decode` the gather reads). The
    /// flat-surface fixture builds a plane PERPENDICULAR to this `N` so `delta ⊥ N` exactly.
    fn center_normal() -> [f32; 3] {
        oct_decode([OCT_RG[0] as f32 / 255.0, OCT_RG[1] as f32 / 255.0])
    }

    /// Builds a `W×H` synthetic G-buffer from a per-pixel `(lit, view_t)` field. The center
    /// pixel's normal is `OCT_RG` (the gather decodes it as the elevation reference); the rest
    /// of the per-pixel normal is irrelevant (the gather reads only the CENTER normal).
    fn synthetic_gbuffer<F: Fn(i32, i32) -> (bool, f32)>(field: F) -> Vec<MarcherAttributes> {
        let mut gbuf = Vec::with_capacity((W as usize) * (H as usize));
        for py in 0..H {
            for px in 0..W {
                let (lit, view_t) = field(px as i32, py as i32);
                gbuf.push(MarcherAttributes {
                    base_rgb: [0, 0, 0],
                    oct_rg: OCT_RG,
                    mat_id: 0,
                    shadow: 255,
                    ao: 255,
                    mask: if lit { 1 } else { 0 },
                    view_t: if lit { view_t } else { SSAO_VIEWT_BG },
                });
            }
        }
        gbuf
    }

    /// The ORTHO world `(x, y)` of a pixel center (mirrors `composite_ray`'s ORTHO arm:
    /// `u = ((px+0.5)/W)*2-1`, `v = -(((py+0.5)/H)*2-1)`, scaled by `SDF_HALF_EXTENT == 1`).
    fn ortho_xy(px: i32, py: i32) -> (f32, f32) {
        let u = (((px as f32) + 0.5) / (W as f32)) * 2.0 - 1.0;
        let v = -((((py as f32) + 0.5) / (H as f32)) * 2.0 - 1.0);
        (u * super::super::SDF_HALF_EXTENT, v * super::super::SDF_HALF_EXTENT)
    }

    #[test]
    fn flat_surface_keeps_ao_near_one() {
        // THE BUG'S REGRESSION GUARD. A literally flat lit plane PERPENDICULAR to the center
        // normal `N`: every neighbour lies in the tangent plane (`delta ⊥ N`), so the
        // elevation `dot(delta, N) == 0` -> no horizon is raised -> occ == 0 -> AO ≈ 1.0. The
        // ORTHO world z is `SDF_CAM_Z - view_t`; to put the surface in the plane through the
        // center perpendicular to `N = (a, a, c)`, solve `N·(P - P0) = 0` for `view_t`:
        // `view_t = view_t0 + (a/c) * (Δx + Δy)`. Under the OLD screen-direction math an
        // in-plane neighbour parallel to the slice axis gave `sampleCos ≈ 1` and BLACKENED
        // this flat lit surface (AO → ~0). The fix makes it AO ≈ 1.
        let n = center_normal();
        let view_t0 = 1.5_f32;
        let (x0, y0) = ortho_xy(CX as i32, CY as i32);
        let gbuf = synthetic_gbuffer(|x, y| {
            let (xw, yw) = ortho_xy(x, y);
            // The tangent-plane depth so delta lands exactly in the plane perpendicular to N.
            let view_t = view_t0 + (n[0] / n[2]) * (xw - x0) + (n[1] / n[2]) * (yw - y0);
            (true, view_t)
        });
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert!(ao.is_finite(), "a flat surface must not produce NaN, got ao = {ao}");
        assert!(
            ao > 0.99,
            "a FLAT lit surface (delta perpendicular to N) must leave AO ≈ 1.0 (the SSAO \
             horizon bug regression guard), got ao = {ao}"
        );
    }

    #[test]
    fn deep_crevice_darkens_ao() {
        // A V-valley: the surface RISES toward the camera (world z grows, i.e. view_t SHRINKS)
        // as the radius from the center grows, so every neighbour sits ABOVE the center's
        // tangent plane (`dot(delta, N) > 0`) and stays well within SSAO_RADIUS. Each tap's
        // elevation is strongly positive inside the falloff -> the per-slice horizon max is
        // high -> occ is large -> AO clearly < 1.
        let gbuf = synthetic_gbuffer(|x, y| {
            let dx = (x - CX as i32) as f32;
            let dy = (y - CY as i32) as f32;
            let r = (dx * dx + dy * dy).sqrt();
            (true, 1.5 - 0.01 * r)
        });
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert!(
            ao < 0.5,
            "a deep crevice (neighbours above the tangent) must darken AO clearly below 1.0, \
             got ao = {ao}"
        );
        assert!(ao >= 0.0, "AO is clamped to [0, 1], got ao = {ao}");
        assert!(ao.is_finite(), "AO must be finite, got ao = {ao}");
    }

    #[test]
    fn isolated_seam_is_fully_unoccluded_no_nan() {
        // A seam: a single lit center pixel, every neighbour is background (mask == 0). Every
        // tap reconstructs Pp = P (the seam's out-of-bounds / non-lit skip) -> delta == 0 ->
        // elev == 0 (guarded by SSAO_EPS, no divide-by-zero NaN) -> occ == 0 -> AO == 1.
        let gbuf = synthetic_gbuffer(|x, y| (x == CX as i32 && y == CY as i32, 1.5));
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert!(ao.is_finite(), "a seam must not produce NaN, got ao = {ao}");
        assert!(
            (ao - 1.0).abs() < 1.0e-6,
            "an all-background seam must be fully unoccluded (ao = 1.0), got ao = {ao}"
        );
    }

    #[test]
    fn non_lit_center_returns_neutral_one() {
        // A non-lit center (background): the gather returns the neutral 1.0 before any march,
        // so the resolve's `min(class_ao, ssao)` leaves the pixel unchanged.
        let gbuf = synthetic_gbuffer(|_x, _y| (false, 0.0));
        let ao = golden_ssao_attributes(&gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default());
        assert_eq!(ao, 1.0, "a non-lit center pixel must return the neutral AO 1.0");
    }

    #[test]
    fn vb_thin_view_t_only_mask_matches_base_dual_gate() {
        // R9-VB-SPLIT-PLAN.md §5 (R9b): the VB_THIN shader variant drops `gMaterial` entirely
        // and gates the background/mask test purely on `view_t >= SSAO_VIEWT_BG` (there is no
        // material mask byte under the VB split — the no-matcache rule). The real G-buffer
        // producer always COUPLES `mask == 0 <=> view_t == SSAO_VIEWT_BG` (see `golden_gbuffer`'s
        // background arm), so the base's dual `mask>0.5 && view_t<BG` gate and VB_THIN's single
        // `view_t<BG` gate classify every pixel IDENTICALLY on any REAL (coupled) G-buffer. This
        // locks that substitutability host-side: forcing `mask` to a CONSTANT 1 (removing its
        // discriminative power entirely — the VB_THIN shape, which carries no mask byte at all)
        // on both the seam fixture (lit center, background neighbourhood) and the
        // background-center fixture must NOT change the gather's AO output, since `view_t`
        // alone still gates every tap either way.
        for gbuf in [
            synthetic_gbuffer(|x, y| (x == CX as i32 && y == CY as i32, 1.5)),
            synthetic_gbuffer(|_x, _y| (false, 0.0)),
        ] {
            let ao_base = golden_ssao_attributes(
                &gbuf, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default(),
            );
            let mut gbuf_vb_thin = gbuf;
            for attrs in &mut gbuf_vb_thin {
                attrs.mask = 1;
            }
            let ao_vb_thin = golden_ssao_attributes(
                &gbuf_vb_thin, CX, CY, W, H, CompositeCamera::Ortho, &SsaoParams::default(),
            );
            assert_eq!(
                ao_base, ao_vb_thin,
                "VB_THIN's view_t-only mask gate must reproduce the base's mask&&view_t dual \
                 gate on a coupled G-buffer (forcing mask=1 changed AO from {ao_base} to \
                 {ao_vb_thin})"
            );
        }
    }

    /// The EXACT per-pixel dither the gather applies (mirror of the `golden_ssao_attributes`
    /// Hilbert+R2 low-discrepancy basis): ONE 64x64 Hilbert index drives two R2 channels — ALPHA1
    /// -> the rotation slot `(r2 * ROT_N) >> 24` over the 64-entry table, ALPHA2 -> the radial
    /// step-phase `((r2 >> 16) + 1) / 256.0`. Returned as `(rot_slot, radial_phase)` so the
    /// determinism + decorrelation test can assert both.
    fn dither(px: u32, py: u32) -> (usize, f32) {
        let hindex = crate::goldens::ssao_hilbert(
            super::super::SSAO_HILBERT_W,
            px & (super::super::SSAO_HILBERT_W - 1),
            py & (super::super::SSAO_HILBERT_W - 1),
        );
        let slot =
            ((crate::goldens::ssao_r2(hindex, super::super::SSAO_R2_ALPHA1).wrapping_mul(super::super::SSAO_ROT_N)) >> 24) as usize;
        let r2_rad = crate::goldens::ssao_r2(hindex, super::super::SSAO_R2_ALPHA2);
        let radial_phase = ((r2_rad >> 16) + 1) as f32 / 256.0;
        (slot, radial_phase)
    }

    #[test]
    fn q1_dither_is_deterministic_in_range_and_decorrelated() {
        // Q1: the per-pixel dither (rotation slot + radial step-phase) is the concentric-ring
        // fix. It must be (1) DETERMINISTIC (the same pixel always yields the same dither — the
        // host oracle and the GPU agree), (2) IN RANGE (slot in [0, 16), radial_phase strictly
        // in (0, 1] so the nearest tap never advances to 0 — no center self-tap), and (3)
        // DECORRELATED (distinct pixels get a SPREAD of (slot, phase) pairs — the property that
        // turns the coherent rings into high-frequency noise the depth-aware blur removes).
        use std::collections::HashSet;

        let mut seen_slots: HashSet<usize> = HashSet::new();
        let mut seen_phase_bins: HashSet<u32> = HashSet::new();
        let mut seen_pairs: HashSet<(usize, u32)> = HashSet::new();

        for py in 0..64u32 {
            for px in 0..64u32 {
                let (slot, phase) = dither(px, py);

                // (1) determinism: a re-evaluation is bit-identical.
                let (slot2, phase2) = dither(px, py);
                assert_eq!(slot, slot2, "rotation slot must be deterministic at ({px},{py})");
                assert_eq!(
                    phase.to_bits(),
                    phase2.to_bits(),
                    "radial_phase must be bit-deterministic at ({px},{py})"
                );

                // (2) range: slot in [0, 64); phase strictly in (0, 1] (no self-tap, no overshoot).
                assert!(
                    slot < (super::super::SSAO_ROT_N as usize),
                    "slot {slot} out of [0, SSAO_ROT_N)"
                );
                assert!(
                    phase > 0.0 && phase <= 1.0,
                    "radial_phase {phase} must be in (0, 1] (strictly positive ⇒ no center \
                     self-tap; ≤ 1 ⇒ the farthest tap reaches at most pix_radius)"
                );

                seen_slots.insert(slot);
                seen_phase_bins.insert((phase * 256.0).round() as u32);
                seen_pairs.insert((slot, (phase * 256.0).round() as u32));
            }
        }

        // (3) decorrelation: over a 64×64 block the dither spreads across the (now 64-entry —
        // the even-slice class-collapse fix) table and the phase band, and produces MANY
        // distinct (slot, phase) pairs — proving neighbouring pixels do NOT march the same
        // step radii (the coherent-ring root cause).
        assert!(
            seen_slots.len() >= 32,
            "the 64-entry rotation must exercise a spread of slots over a 64×64 block (saw {}), \
             else the angular banding stays coherent",
            seen_slots.len()
        );
        assert!(
            seen_phase_bins.len() >= 64,
            "the radial step-phase must spread across its [1/256, 1] band over a 64×64 block \
             (saw {} bins), else the concentric rings stay coherent",
            seen_phase_bins.len()
        );
        assert!(
            seen_pairs.len() >= 256,
            "the (rotation, radial-phase) dither must yield many distinct pairs over a 64×64 \
             block (saw {}), proving the per-pixel decorrelation",
            seen_pairs.len()
        );

        // A direct sanity pair: two distinct pixels get DIFFERENT dither (the decorrelation core).
        assert_ne!(
            dither(0, 0),
            dither(1, 0),
            "adjacent pixels must get a different (slot, radial_phase) dither"
        );
    }
}

/// The SSAO edge-avoiding à-trous denoise host mirror ([`golden_ssao_atrous`]) tests. Proves the
/// N=3-pass chain on the host side: (1) a sharp AO discontinuity is smoothed toward its
/// neighbourhood mean WITHIN the chain's effective footprint, unchanged FAR from it, and (2) the
/// bilateral DEPTH gate prevents bleed across a silhouette (a `view_t` jump > [`SSAO_BLUR_DEPTH_TOL`]).
/// Pure host math; runs device-less.
#[cfg(test)]
mod ssao_atrous_tests {
    use super::super::SSAO_BLUR_DEPTH_TOL;
    use crate::compute::CompositeCamera;
    use crate::goldens::{golden_ssao_atrous, MarcherAttributes};

    const W: u32 = 48;
    const H: u32 = 48;
    /// `N = 3` passes (steps `{1, 2, 4}`), the default `SsaoConfig::atrous_levels` — see
    /// `boyko_render::ssao_config`.
    const LEVELS: u32 = 3;
    /// The chain's effective footprint half-width: `sum(2 * step)` over `steps {1,2,4}` == 14 px
    /// (the plan's "~14px footprint" for N=3). A pixel at least this far from a discontinuity
    /// sees ONLY same-side taps at every pass.
    const REACH: i32 = 14;

    /// A `W×H` synthetic G-buffer from a per-pixel `(ssao_byte, view_t)` field. Only the
    /// `view_t` lane (the depth gate) is meaningful here; `mask`/the rest are inert (the filter
    /// reads neither). Returns `(raw_ssao_bytes, gbuf)`.
    fn build<F: Fn(i32, i32) -> (u8, f32)>(field: F) -> (Vec<u8>, Vec<MarcherAttributes>) {
        let mut ssao = Vec::with_capacity((W * H) as usize);
        let mut gbuf = Vec::with_capacity((W * H) as usize);
        for py in 0..H {
            for px in 0..W {
                let (byte, view_t) = field(px as i32, py as i32);
                ssao.push(byte);
                gbuf.push(MarcherAttributes {
                    base_rgb: [0, 0, 0],
                    oct_rg: [128, 128],
                    mat_id: 0,
                    shadow: 255,
                    ao: 255,
                    mask: 1,
                    view_t,
                });
            }
        }
        (ssao, gbuf)
    }

    #[test]
    fn sharp_ring_is_smoothed_far_stays_exact() {
        // A constant-depth flat surface (every neighbour passes the depth gate) with a SHARP AO
        // discontinuity: the left half is fully-dark (byte 0), the right half fully-bright
        // (byte 255). A pixel FAR ENOUGH from the seam that NO dark tap falls inside the
        // chain's effective footprint (REACH) sees only bright taps at every pass, so its
        // filtered value equals the bright value EXACTLY (a weighted mean of equal taps is that
        // value, and the R8/R16 round-trip quantization is a no-op on an already-uniform image).
        const SEAM: i32 = 20;
        let (ssao, gbuf) = build(|x, _y| (if x < SEAM { 0 } else { 255 }, 1.5));

        let px = (SEAM + REACH + 1) as u32;
        let py = 24;
        let raw = ssao[(py * W + px) as usize];
        let out = golden_ssao_atrous(&ssao, &gbuf, W, H, CompositeCamera::Ortho, LEVELS);
        let filtered = out[(py * W + px) as usize];
        assert_eq!(
            filtered, raw,
            "a pixel whose entire à-trous footprint is uniformly bright must filter to that \
             value EXACTLY, got {filtered} raw {raw}"
        );
    }

    #[test]
    fn sharp_ring_is_smoothed_near_seam() {
        // The counterpart: a pixel close enough to the seam (well inside REACH) that dark taps
        // DO fall inside the footprint at the wider passes — the filtered value must visibly
        // pull down from the raw bright value (proving the chain still smooths a nearby
        // discontinuity).
        const SEAM: i32 = 20;
        let (ssao, gbuf) = build(|x, _y| (if x < SEAM { 0 } else { 255 }, 1.5));

        let px = SEAM as u32; // the first bright column; the seam is one tap away
        let py = 24;
        let raw = ssao[(py * W + px) as usize] as f32;
        let out = golden_ssao_atrous(&ssao, &gbuf, W, H, CompositeCamera::Ortho, LEVELS);
        let filtered = out[(py * W + px) as usize] as f32;
        assert!(
            filtered < raw - 10.0,
            "the à-trous chain must smooth a sharp ring within its effective footprint: raw \
             {raw} filtered {filtered}"
        );
    }

    #[test]
    fn depth_gate_prevents_silhouette_bleed() {
        // A silhouette: the left half is a NEAR surface (`view_t = 1.5`, dark AO byte 40) and the
        // right half is a FAR surface (`view_t = 1.5 + 10*tol`, bright AO byte 255) — a `view_t`
        // jump far beyond the gate. A near-surface pixel ON the boundary must filter ONLY with
        // its near-side (in-tol) neighbours at EVERY pass, so its filtered AO stays near the dark
        // value and is NOT pulled up by the far-side bright taps (no cross-silhouette bleed).
        const SEAM: i32 = 20;
        const DARK: u8 = 40;
        let near_t = 1.5_f32;
        let far_t = 1.5_f32 + 10.0 * SSAO_BLUR_DEPTH_TOL;
        let (ssao, gbuf) = build(|x, _y| {
            if x < SEAM { (DARK, near_t) } else { (255, far_t) }
        });

        let px = (SEAM - 1) as u32;
        let py = 24;
        let out = golden_ssao_atrous(&ssao, &gbuf, W, H, CompositeCamera::Ortho, LEVELS);
        let filtered = out[(py * W + px) as usize];
        assert!(
            filtered <= DARK + 5,
            "the depth gate must reject far-side taps at every pass: a near-surface pixel must \
             filter close to the near AO {DARK} (got {filtered}), NOT bleed the far-side bright \
             AO across the silhouette"
        );
    }

    #[test]
    fn center_always_counts_no_divide_by_zero() {
        // An ISOLATED lit pixel surrounded by a far background (every neighbour fails the depth
        // gate at every pass): the filter must still count the CENTER (weight >= 0.140625, the
        // B3 kernel's own-tap weight) and converge to (approximately) the center's own raw AO —
        // never a 0/0 NaN. The R16/R8 round-trip quantization between passes may shift the
        // result by a few counts, so allow a small tolerance.
        let (ssao, gbuf) = build(|x, y| {
            if x == 24 && y == 24 { (90, 1.5) } else { (255, super::super::SSAO_VIEWT_BG) }
        });
        let out = golden_ssao_atrous(&ssao, &gbuf, W, H, CompositeCamera::Ortho, LEVELS);
        let filtered = out[(24 * W + 24) as usize];
        assert!(
            filtered.abs_diff(90) <= 2,
            "an isolated pixel (all neighbours gated out at every pass) must filter close to \
             its OWN raw AO, got {filtered}"
        );
    }

    #[test]
    fn levels_zero_is_byte_identical_to_raw_gather() {
        let (ssao, gbuf) = build(|x, y| (((x + y) * 7) as u8, 1.5));
        let out = golden_ssao_atrous(&ssao, &gbuf, W, H, CompositeCamera::Ortho, 0);
        assert_eq!(out, ssao, "levels == 0 must return the raw gather byte-identical");
    }
}
