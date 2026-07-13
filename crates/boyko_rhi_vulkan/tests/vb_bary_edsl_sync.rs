//! Host oracle + emit-pin guard for the Visibility-Buffer analytic barycentric/attribute
//! math (`boyko_shaderdsl::vb`, Multi-Paradigm Render-Path Plan §C/§F, rung R7).
//!
//! **Bit-exactness note (plan §C, "Oracle & tolerance" / C4):** rung R7 is PURE MATH +
//! codegen — no rendering, no framegraph, no pipeline (that lands in R8/`R-VBGEO`, which
//! also gains the GPU-vs-host bit-exact/ULP leg once a compute pipeline exists to run the
//! emitted HLSL on). THIS rung's oracle has two tracks, neither of which runs a GPU:
//!
//! - **Track 1 (host-eval vs an INDEPENDENT reference)**: `boyko_shaderdsl::vb`'s `f32`
//!   instantiation is checked against reference formulas derived from first principles and
//!   written INLINE in this file (a different pivot/derivation than `vb`'s own determinant-
//!   anchor form, an independent finite-difference derivative, and a deliberately-buggy
//!   McLaren/Hill gradient form) — never by calling `boyko_shaderdsl::vb` a second time.
//! - **Track 2 (emit span pins)**: `boyko_shaderdsl::emit::emit_hlsl_vb_*` output is
//!   `.contains`-checked for the key generated expressions/sentinels, mirroring
//!   `ssao_edsl_sync.rs`/`interp_edsl_sync.rs`'s drift-gate shape. R7 does not splice these
//!   spans into any committed shader (there is none yet), so there is no re-DXC/byte-
//!   identity leg here — that begins once R8 commits `vb_geom_fetch.hlsli`.

use boyko_shaderdsl::vb::{self, BaryBasis};

/// Asserts `got` (the `f32` host-eval result) and `want` (an `f64` independent-reference
/// value) agree within `tol` (absolute), with a helpful failure message.
fn assert_close(got: f32, want: f64, tol: f64, what: &str) {
    let diff = (got as f64 - want).abs();
    assert!(
        diff <= tol,
        "{what}: got {got} (f32), want {want} (f64 reference), |diff| = {diff} > tol {tol}"
    );
}

// ---- Track 1: an INDEPENDENT reference barycentric formula (a different pivot/derivation
// than `vb::vb_barycentric_grad_body`/`vb_barycentric_eval_body`'s determinant-anchor form)
// -------------------------------------------------------------------------------------------

/// `ax*by - ay*bx` — the 2D cross product (twice the signed area of the parallelogram).
fn cross2(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    ax * by - ay * bx
}

/// The classic sub-triangle-AREA-RATIO barycentric formula (Ericson, "Real-Time Collision
/// Detection"), pivoted at each vertex directly against the query point `p` — a genuinely
/// DIFFERENT computation path than `vb`'s vertex-0-anchor affine-gradient evaluation (no
/// `dlambda/dx`/`dlambda/dy` constants at all, just three sub-triangle areas over the whole
/// triangle's area), written in `f64` for reference precision.
fn reference_lambda(x: [f32; 3], y: [f32; 3], px: f32, py: f32) -> [f64; 3] {
    let (x0, y0, x1, y1, x2, y2) = (
        x[0] as f64,
        y[0] as f64,
        x[1] as f64,
        y[1] as f64,
        x[2] as f64,
        y[2] as f64,
    );
    let (px, py) = (px as f64, py as f64);
    let full = cross2(x1 - x0, y1 - y0, x2 - x0, y2 - y0);
    let l0 = cross2(x1 - px, y1 - py, x2 - px, y2 - py) / full;
    let l1 = cross2(x2 - px, y2 - py, x0 - px, y0 - py) / full;
    let l2 = cross2(x0 - px, y0 - py, x1 - px, y1 - py) / full;
    [l0, l1, l2]
}

fn make_basis(x: [f32; 3], y: [f32; 3]) -> BaryBasis<f32> {
    let (dlambda_dx, dlambda_dy) = vb::vb_barycentric_grad_body::<f32>(x, y);
    BaryBasis {
        dlambda_dx,
        dlambda_dy,
        x0: x[0],
        y0: y[0],
    }
}

/// Fixture (a): a well-conditioned right triangle, several interior pixels — host eval vs
/// the independent area-ratio reference.
#[test]
fn barycentric_matches_independent_reference_well_conditioned() {
    let x = [0.0f32, 100.0, 0.0];
    let y = [0.0f32, 0.0, 100.0];
    let basis = make_basis(x, y);

    for &(px, py) in &[
        (10.0f32, 10.0),
        (50.0, 25.0),
        (1.0, 1.0),
        (33.0, 33.0),
        (0.0, 0.0),
    ] {
        let got = vb::vb_barycentric_eval_body::<f32>(basis, px, py);
        let want = reference_lambda(x, y, px, py);
        for i in 0..3 {
            assert_close(
                got[i],
                want[i],
                1.0e-4,
                &format!("lambda[{i}] at ({px},{py})"),
            );
        }
    }
}

/// Fixture (b): a sliver (near-degenerate `D`, but not exactly zero) triangle — the
/// gradients/eval must stay finite and match the independent reference (looser tolerance:
/// a near-degenerate triangle amplifies floating-point rounding for both the reference and
/// `vb`'s own formula equally, since both divide by the SAME small `D`-scale quantity).
#[test]
fn barycentric_matches_independent_reference_sliver_triangle() {
    let x = [0.0f32, 1000.0, 1000.0];
    let y = [0.0f32, 0.0, 0.001];
    let basis = make_basis(x, y);
    assert!(
        basis.dlambda_dx[0].is_finite(),
        "sliver triangle produced a non-finite gradient"
    );
    assert!(
        basis.dlambda_dy[0].is_finite(),
        "sliver triangle produced a non-finite gradient"
    );

    for &(px, py) in &[(500.0f32, 0.0002), (10.0, 0.00001), (999.0, 0.0009999)] {
        let got = vb::vb_barycentric_eval_body::<f32>(basis, px, py);
        for &l in &got {
            assert!(
                l.is_finite(),
                "sliver triangle lambda is non-finite at ({px},{py})"
            );
        }
        let want = reference_lambda(x, y, px, py);
        for i in 0..3 {
            assert_close(
                got[i],
                want[i],
                5.0e-2,
                &format!("sliver lambda[{i}] at ({px},{py})"),
            );
        }
    }
}

/// Fixture (e): `l0 + l1 + l2 == 1` everywhere in the plane (in AND outside the triangle —
/// the affine extrapolation the constant gradient encodes holds everywhere, not just inside
/// the hull), for a grid of sample points.
#[test]
fn barycentric_sums_to_one_everywhere() {
    let x = [0.0f32, 64.0, 12.0];
    let y = [0.0f32, 8.0, 96.0];
    let basis = make_basis(x, y);

    for pxi in -20..140i32 {
        for pyi in (-20..140i32).step_by(7) {
            let (px, py) = (pxi as f32, pyi as f32);
            let l = vb::vb_barycentric_eval_body::<f32>(basis, px, py);
            let sum = l[0] + l[1] + l[2];
            assert!(
                (sum - 1.0).abs() <= 1.0e-3,
                "lambda sum at ({px},{py}) = {sum}, expected ~1.0"
            );
        }
    }
}

/// Fixture (f): self-consistency — interpolating the vertices' OWN screen positions with
/// their own `lambda` weights reconstructs the queried pixel exactly (up to rounding):
/// `sum_i(lambda_i * x_i) == px`, `sum_i(lambda_i * y_i) == py`.
#[test]
fn barycentric_self_consistent_reconstructs_pixel() {
    let x = [3.0f32, 91.0, 17.0];
    let y = [5.0f32, 21.0, 88.0];
    let basis = make_basis(x, y);

    for &(px, py) in &[
        (20.0f32, 20.0),
        (91.0, 21.0),
        (3.0, 5.0),
        (-15.0, 40.0),
        (60.0, 60.0),
    ] {
        let l = vb::vb_barycentric_eval_body::<f32>(basis, px, py);
        let recon_x = l[0] * x[0] + l[1] * x[1] + l[2] * x[2];
        let recon_y = l[0] * y[0] + l[1] * y[1] + l[2] * y[2];
        assert!(
            (recon_x - px).abs() <= 1.0e-2,
            "reconstructed x = {recon_x}, want {px}"
        );
        assert!(
            (recon_y - py).abs() <= 1.0e-2,
            "reconstructed y = {recon_y}, want {py}"
        );
    }
}

// ---- Track 1: the McLaren/Hill CalcFullBary gradient-bug regression (fixture c) -----------

/// The KNOWN-BUGGY McLaren/Hill gradient form: the raw `dlambda_i/dx` (or `dy`) dotted with
/// the RAW vertex attribute `a_i`, skipping the `/w_i` perspective weighting entirely.
/// Bit-identical to the corrected form only when `w` is constant across the triangle;
/// written INLINE (not calling `boyko_shaderdsl::vb`) so it is a true independent negative
/// control.
fn buggy_raw_lambda_gradient(dlambda: [f32; 3], a: [f32; 3]) -> f32 {
    dlambda[0] * a[0] + dlambda[1] * a[1] + dlambda[2] * a[2]
}

/// The independent perspective-correct VALUE reference (direct full re-evaluation at a
/// given pixel — NOT `vb`'s incremental anchor+gradient form): `lambda` from the
/// [`reference_lambda`] area-ratio formula, then `sum(lambda_i * a_i/w_i) /
/// sum(lambda_i/w_i)`.
fn reference_perspective_value(
    x: [f32; 3],
    y: [f32; 3],
    w: [f32; 3],
    a: [f32; 3],
    px: f32,
    py: f32,
) -> f64 {
    let l = reference_lambda(x, y, px, py);
    let num: f64 = (0..3).map(|i| l[i] * (a[i] as f64) / (w[i] as f64)).sum();
    let den: f64 = (0..3).map(|i| l[i] / (w[i] as f64)).sum();
    num / den
}

/// Fixture (c): a perspective-heavy triangle (`w` ratio 1000:1) sampled NEAR the extreme-`w`
/// vertex, where the corrected [`vb::vb_interp_body`] gradient and the buggy McLaren/Hill
/// raw-`lambda`-dot-raw-attribute form DIVERGE unambiguously. Asserts (1) `vb`'s analytic
/// `d(value)/dx` matches an independent central-finite-difference of
/// [`reference_perspective_value`] (tight tolerance — proving `vb` implements the CORRECTED
/// form), and (2) the buggy form disagrees with that same reference by a large margin
/// (proving a test built against the buggy form would fail — the regression this fixture
/// guards).
///
/// **Why `(px, py) = (99.0, 1.0)` at `w` ratio 1000:1, not e.g. `(30, 30)` at 100:1**: the
/// buggy form is a raw `dlambda/dx . a` dot product that does NOT depend on `w` or the pixel
/// at all — for THIS triangle/attribute it is the constant `0.01` everywhere. The correct
/// (perspective-corrected) derivative is `(0.01/w_ratio) / lerp(1/w0, 1/w_ratio)_at_pixel^2`,
/// which is small away from the high-`w` vertex and only grows large close to it (where the
/// interpolated `1/w` denominator, `Wr`, shrinks toward its minimum at that vertex). Solving
/// this in closed form (`N`, `Wr` are exactly affine in the pixel, so `value = N/Wr` is an
/// exact Mobius transform of the pixel coordinate) and cross-checking numerically:
/// `(30,30)`@100:1 gives `|buggy - fd| ~ 0.0098` (the reported near-miss); `(70,15)`@1000:1
/// gives `|buggy - fd| ~ 0.0099` (still too close to the buggy value: `y` does not move this
/// derivative at all here since `w0 == w2`, and `px=70` is not close enough to the high-`w`
/// vertex at `x=100`); `(99.0, 1.0)`@1000:1 (`Wr ~ 0.011`, close to its vertex-1 minimum
/// `1/1000`) gives the correct derivative `~0.0828` against the buggy `0.01` —
/// `|buggy - fd| ~ 0.0728`, comfortably past the >5e-2 discrimination target. The
/// finite-difference itself stays essentially exact there (the closed-form Mobius analysis
/// puts its relative truncation error at `~8e-5` for `h = 1e-2`, i.e. an absolute error of
/// only `~7e-6` on a `~0.08`-magnitude derivative), so the `5e-3` correctness tolerance below
/// still holds with roughly 700x headroom.
#[test]
fn interp_mclaren_hill_perspective_regression() {
    let x = [0.0f32, 100.0, 0.0];
    let y = [0.0f32, 0.0, 100.0];
    let w = [1.0f32, 1000.0, 1.0]; // 1000:1 perspective ratio at vertex 1.
    let a = [0.0f32, 1.0, 0.0]; // the attribute varies only at the extreme-w vertex.
    let (px, py) = (99.0f32, 1.0); // close to vertex 1 (100, 0) -- see the doc comment above.

    let basis = make_basis(x, y);
    let [value, d_dx, _d_dy] = vb::vb_interp_body::<f32>(basis, px, py, a, w);

    // (1) vb's analytic derivative vs an independent central finite difference of the
    // independently-coded perspective value.
    let h = 1.0e-2f32;
    let v_plus = reference_perspective_value(x, y, w, a, px + h, py);
    let v_minus = reference_perspective_value(x, y, w, a, px - h, py);
    let fd_d_dx = (v_plus - v_minus) / (2.0 * h as f64);
    assert_close(
        d_dx,
        fd_d_dx,
        5.0e-3,
        "vb_interp d(value)/dx vs finite-difference reference",
    );

    // Sanity: vb's own interpolated value agrees with the direct reference at (px, py).
    let want_value = reference_perspective_value(x, y, w, a, px, py);
    assert_close(
        value,
        want_value,
        1.0e-4,
        "vb_interp value vs direct reference",
    );

    // (2) the buggy raw-lambda-dot-raw-attribute gradient diverges measurably from the
    // SAME finite-difference reference -- a test asserting `d_dx == buggy_d_dx` would have
    // FAILED to notice the bug being absent; this asserts the opposite (that they genuinely
    // disagree), proving this fixture discriminates the two forms. At this fixture's
    // (px, py, w-ratio) the actual margin is ~0.0728 (verified above); the threshold below
    // is set to roughly HALF of that (3e-2), so the assert is robustly discriminating
    // without being brittle to small formula-order/rounding changes.
    let buggy_d_dx = buggy_raw_lambda_gradient(basis.dlambda_dx, a);
    let buggy_diff = (buggy_d_dx as f64 - fd_d_dx).abs();
    assert!(
        buggy_diff > 3.0e-2,
        "expected the buggy McLaren/Hill form to diverge from the correct derivative by more \
         than 3e-2 (got |diff| = {buggy_diff}) -- this perspective-heavy fixture must \
         discriminate the two forms, else it cannot catch the regression it targets"
    );
    let correct_diff = (d_dx as f64 - fd_d_dx).abs();
    assert!(
        correct_diff < buggy_diff,
        "vb_interp's own derivative ({d_dx}) should be far closer to the finite-difference \
         reference ({fd_d_dx}) than the buggy form ({buggy_d_dx}) is"
    );
}

/// Fixture (g): `vb_uv_grad` vs a central-difference numerical gradient of `vb_interp`
/// itself, sampled at `+-0.5px` (a moderate, non-extreme perspective triangle, so the
/// O(h^2) finite-difference truncation error of this smooth rational function stays small
/// at `h = 0.5`).
#[test]
fn uv_grad_matches_central_difference() {
    let x = [0.0f32, 80.0, 10.0];
    let y = [0.0f32, 5.0, 70.0];
    let w = [1.0f32, 2.0, 1.5];
    let u = [0.0f32, 1.0, 0.5];
    let v = [0.0f32, 0.2, 1.0];
    let (px, py) = (25.0f32, 20.0);

    let basis = make_basis(x, y);
    let [du_dx, du_dy, dv_dx, dv_dy] = vb::vb_uv_grad_body::<f32>(basis, px, py, u, v, w);

    let h = 0.5f32;
    let central = |a: [f32; 3], dx: f32, dy: f32| -> f32 {
        let plus = vb::vb_interp_body::<f32>(basis, px + dx, py + dy, a, w)[0];
        let minus = vb::vb_interp_body::<f32>(basis, px - dx, py - dy, a, w)[0];
        (plus - minus) / (2.0 * h)
    };

    let fd_du_dx = central(u, h, 0.0);
    let fd_du_dy = central(u, 0.0, h);
    let fd_dv_dx = central(v, h, 0.0);
    let fd_dv_dy = central(v, 0.0, h);

    assert_close(
        du_dx,
        fd_du_dx as f64,
        5.0e-3,
        "du/dx vs central difference",
    );
    assert_close(
        du_dy,
        fd_du_dy as f64,
        5.0e-3,
        "du/dy vs central difference",
    );
    assert_close(
        dv_dx,
        fd_dv_dx as f64,
        5.0e-3,
        "dv/dx vs central difference",
    );
    assert_close(
        dv_dy,
        fd_dv_dy as f64,
        5.0e-3,
        "dv/dy vs central difference",
    );
}

// ---- Track 1: near-plane clip (fixture d) --------------------------------------------------

/// Fixture (d): a near-plane-straddling triangle (one vertex behind the near plane, two in
/// front) — `vb_near_clip` must produce finite, plausible vertices (every returned `w`
/// strictly positive and no `NaN`/`Inf` component anywhere).
#[test]
fn near_clip_straddling_triangle_is_stable() {
    let v = [
        [10.0f32, 10.0, 5.0, 10.0],  // good (w = 10)
        [-5.0f32, -5.0, -1.0, -0.5], // bad (w <= epsilon, behind the near plane)
        [3.0f32, 3.0, 2.0, 50.0],    // good (w = 50)
    ];
    let out = vb::vb_near_clip_body::<f32>(v);

    for (i, vtx) in out.iter().enumerate() {
        for (c, &comp) in vtx.iter().enumerate() {
            assert!(
                comp.is_finite(),
                "clipped vertex {i} component {c} is non-finite: {comp}"
            );
        }
        assert!(
            vtx[3] > 0.0,
            "clipped vertex {i} has non-positive w = {}, expected > 0 after near-clip",
            vtx[3]
        );
    }
    // The bad vertex (index 1) must have moved (it was NOT already in front); its w should
    // now sit at (or very near) the epsilon boundary, since BOTH its neighbours are good.
    assert!(
        (out[1][3] - vb::NEAR_CLIP_W_EPSILON).abs() <= 1.0e-3,
        "shrunk vertex w = {}, expected close to NEAR_CLIP_W_EPSILON = {}",
        out[1][3],
        vb::NEAR_CLIP_W_EPSILON
    );
}

/// A fully in-front triangle is an EXACT passthrough (every vertex `w >
/// NEAR_CLIP_W_EPSILON`, so nothing should move at all) — the "shrink-only, never generate
/// new triangles" contract's no-op case.
#[test]
fn near_clip_all_good_is_exact_passthrough() {
    let v = [
        [1.0f32, 2.0, 3.0, 4.0],
        [5.0f32, 6.0, 7.0, 8.0],
        [9.0f32, 10.0, 11.0, 12.0],
    ];
    let out = vb::vb_near_clip_body::<f32>(v);
    assert_eq!(
        out, v,
        "an already-valid triangle must pass through unchanged"
    );
}

/// A triangle fully behind the near plane (every `w <= epsilon`) has no good neighbour to
/// shrink toward; `vb_near_clip` must still return finite vertices with `w` clamped to the
/// epsilon floor (a defined, stable degenerate case — real culling of a fully-behind
/// triangle happens upstream, out of scope for this rung).
#[test]
fn near_clip_all_behind_near_plane_stays_finite() {
    let v = [
        [1.0f32, 1.0, 1.0, -1.0],
        [2.0f32, 2.0, 2.0, -2.0],
        [3.0f32, 3.0, 3.0, -3.0],
    ];
    let out = vb::vb_near_clip_body::<f32>(v);
    for (i, vtx) in out.iter().enumerate() {
        for (c, &comp) in vtx.iter().enumerate() {
            assert!(
                comp.is_finite(),
                "all-behind vertex {i} component {c} is non-finite"
            );
        }
        assert!(
            (vtx[3] - vb::NEAR_CLIP_W_EPSILON).abs() <= 1.0e-6,
            "all-behind vertex {i} w = {}, expected clamped to NEAR_CLIP_W_EPSILON",
            vtx[3]
        );
    }
}

// ---- Track 2: emit output span pins -------------------------------------------------------

#[test]
fn emit_vb_barycentric_contains_generated_spans() {
    let out = boyko_shaderdsl::emit::emit_hlsl_vb_barycentric();
    assert!(out.contains("// === GENERATED vb_barycentric_grad BEGIN ==="));
    assert!(out.contains("// === GENERATED vb_barycentric_grad END ==="));
    assert!(out.contains("// === GENERATED vb_barycentric_eval BEGIN ==="));
    assert!(out.contains("// === GENERATED vb_barycentric_eval END ==="));
    assert!(out.contains("struct VbBaryGrad { float3 dlambda_dx; float3 dlambda_dy; };"));
    assert!(out.contains("VbBaryGrad vb_barycentric_grad(float3 vx, float3 vy) {"));
    assert!(out.contains(
        "float3 vb_barycentric_eval(float3 dlambda_dx, float3 dlambda_dy, float x0, \
             float y0, float px, float py) {"
    ));
    assert!(out.contains("g.dlambda_dx = float3("));
    assert!(out.contains("g.dlambda_dy = float3("));
    assert!(out.contains("return float3("));
}

#[test]
fn emit_vb_interp_contains_generated_span() {
    let out = boyko_shaderdsl::emit::emit_hlsl_vb_interp();
    assert!(out.contains("// === GENERATED vb_interp BEGIN ==="));
    assert!(out.contains("// === GENERATED vb_interp END ==="));
    assert!(out.contains(
        "float3 vb_interp(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, \
             float px, float py, float3 a, float3 w) {"
    ));
    assert!(out.contains("return float3("));
}

#[test]
fn emit_vb_uv_grad_contains_generated_span() {
    let out = boyko_shaderdsl::emit::emit_hlsl_vb_uv_grad();
    assert!(out.contains("// === GENERATED vb_uv_grad BEGIN ==="));
    assert!(out.contains("// === GENERATED vb_uv_grad END ==="));
    assert!(out.contains(
        "float4 vb_uv_grad(float3 dlambda_dx, float3 dlambda_dy, float x0, float y0, \
             float px, float py, float3 u, float3 v, float3 w) {"
    ));
    assert!(out.contains("return float4("));
}

#[test]
fn emit_vb_near_clip_contains_generated_span() {
    let out = boyko_shaderdsl::emit::emit_hlsl_vb_near_clip();
    assert!(out.contains("// === GENERATED vb_near_clip BEGIN ==="));
    assert!(out.contains("// === GENERATED vb_near_clip END ==="));
    assert!(out.contains("struct VbClippedTri { float4 v0; float4 v1; float4 v2; };"));
    assert!(out.contains("VbClippedTri vb_near_clip(float4 v0, float4 v1, float4 v2) {"));
    assert!(out.contains("c.v0 = float4("));
    assert!(out.contains("c.v1 = float4("));
    assert!(out.contains("c.v2 = float4("));
    assert!(out.contains("return c;"));
}
