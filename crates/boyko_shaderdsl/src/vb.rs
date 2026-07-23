//! Visibility-Buffer analytic attribute-reconstruction math (Multi-Paradigm Render-Path
//! Plan §C/§F, rung R7) — screen-space barycentrics, perspective-correct attribute
//! interpolation, texcoord gradients, and a simplified near-plane clip, authored ONCE
//! generic over a [`FieldScalar`] backend and instantiated two ways (the established
//! campaign pattern, see [`crate::oct`]/[`crate::ssao`]):
//!
//! - `S = f32` — the **Eval** backend: every op is a single `core` f32 instruction, the
//!   CPU oracle `crates/boyko_rhi_vulkan/tests/vb_bary_edsl_sync.rs` locks against an
//!   independently-derived reference.
//! - `S = Emit` — the **HLSL SSA recorder** (`crate::emit`, `feature = "emit"`); the
//!   printer (`crate::emit::emit_hlsl_vb_barycentric` and siblings) walks the arena into
//!   the self-contained HLSL function spans R8 splices into `vb_geom_fetch.hlsli`.
//!
//! This rung is PURE MATH + codegen: no rendering, no framegraph, no pipeline wiring (that
//! is R8/R-VBGEO). `FieldScalar`'s op-set is already transcendental-free (determinants,
//! divides, `min`/`max`/`select`, no `sin`/`cos`/`sqrt` needed here), so every function
//! below is written directly against it — no new `Cf`/`InterpBackend` axis, and no new
//! `FieldScalar` method (the frozen SDF/marcher/oct/ssao emitted `.spv` cannot fork).
//!
//! # DAIS Appendix A closed form (Schied & Dachsbacher 2015)
//!
//! For a triangle with SCREEN-SPACE vertices `p0, p1, p2` (already projected — the
//! perspective divide has already happened for the SCREEN `x, y`, unlike the per-vertex
//! attributes/`w` below), the barycentric weight `lambda_i` is an AFFINE function of the
//! screen position `(x, y)` with a CONSTANT gradient:
//!
//! ```text
//! D           = det(p2 - p1, p0 - p1)              // twice the signed area
//! dlambda_i/dx = (y_j - y_k) / D
//! dlambda_i/dy = (x_k - x_j) / D
//! ```
//!
//! where `(j, k)` are the OTHER two vertex indices in cyclic order (`i=0 -> j=1,k=2`;
//! `i=1 -> j=2,k=0`; `i=2 -> j=0,k=1`). `D` is shared by all six components, so the whole
//! gradient constant-set costs exactly ONE divide ([`vb_barycentric_grad_body`]).
//!
//! # The shared-anchor evaluation (vertex 0)
//!
//! Every per-pixel quantity below ([`vb_barycentric_eval_body`], [`vb_interp_body`],
//! [`vb_uv_grad_body`]) is evaluated from the SAME screen-space anchor: vertex 0's own
//! screen position `(x0, y0)`. At that anchor `lambda = (1, 0, 0)` (a vertex's barycentric
//! coordinate against itself is 1), so:
//!
//! ```text
//! lambda(x, y) = lambda(anchor) + dx * dlambda/dx + dy * dlambda/dy,   dx = x - x0, dy = y - y0
//! ```
//!
//! reproduces the plan's literal per-vertex determinant-ratio formula (verified: it is the
//! SAME affine function, merely evaluated via its value+gradient at one point instead of
//! re-deriving the ratio at every pixel) — see [`BaryBasis`].
//!
//! # Perspective-correct interpolation (DAIS Eq. 1-2/5) + the McLaren/Hill hazard
//!
//! A vertex attribute `a` is linear in CLIP space, not screen space; `a / w` (and `1 / w`)
//! IS linear in screen space, so it shares the SAME affine-evaluation trick as `lambda`:
//!
//! ```text
//! N(x,y)  = (a/w)_s + dx * d(a/w)/dx + dy * d(a/w)/dy     // (a/w)_s = a0/w0 (vertex-0 anchor)
//! Wr(x,y) = (1/w)_s + dx * d(1/w)/dx + dy * d(1/w)/dy     // (1/w)_s = 1/w0
//! a(x,y)  = N(x,y) / Wr(x,y)
//! ```
//!
//! `d(a/w)/dx = sum_i dlambda_i/dx * (a_i/w_i)` (and the `1/w`, `dy` siblings) — the SAME
//! kind of affine-function gradient as `lambda` itself, since `a/w` is a fixed linear
//! combination of the `lambda_i` with coefficients `a_i/w_i`.
//!
//! **The McLaren/Hill `CalcFullBary` gradient bug** (documented by Hable's 2022
//! correction to a widely-copied VB utility): a common BUGGY implementation computes the
//! screen-space attribute gradient as `sum_i dlambda_i/dx * a_i` — the RAW vertex value,
//! skipping the `/w_i` perspective weighting entirely. This is bit-identical to the correct
//! form ONLY when `w` is constant across the triangle (an orthographic/no-perspective
//! degenerate case); for a genuinely perspective triangle (`w` varies vertex-to-vertex) the
//! two diverge. [`vb_interp_body`] implements the CORRECTED form: the gradient is folded
//! from `a_i / w_i` (the `a_over_w` term below), and the FINAL derivative
//! ([`vb_interp_body`]'s `d_value_dx`/`d_value_dy`) additionally applies the quotient rule
//! (since `a = N / Wr` is a RATIO of two affine functions, not affine itself) —
//! `d(a)/dx = (dN/dx * Wr - N * dW/dx) / Wr^2`. The host oracle's regression fixture
//! constructs a `w`-ratio-100:1 triangle where the buggy raw-`a_i` gradient and this
//! corrected form diverge measurably (`vb_bary_edsl_sync.rs`).
//!
//! # Near-plane clip (simplified Blinn-Newell, shrink-only)
//!
//! Hardware/compute `ddx`/`ddy` (and this module's own analytic gradients) are unstable as
//! `w -> 0` (the perspective divide blows up), so a vertex behind the near plane is clipped
//! ANALYTICALLY, before rasterization — see [`vb_near_clip_body`].

use crate::scalar::FieldScalar;

/// The `w <= epsilon` trigger for [`vb_near_clip_body`]: a vertex at or below this clip-space
/// `w` is "behind" the near plane and gets shrunk toward its good neighbour(s). `1.0e-4`
/// matches this codebase's other divide-guard epsilons (e.g. [`crate::ssao::SSAO_EPS`]).
pub const NEAR_CLIP_W_EPSILON: f32 = 1.0e-4;

/// The divide-by-zero floor for a near-clip edge-crossing denominator (`w_j - w_i`):
/// [`safe_denom`] clamps the MAGNITUDE of a near-clip denominator to at least this value
/// (sign-preserving) so a near-equal-`w` edge never produces `Inf`/`NaN` — including on a
/// branch [`vb_near_clip_body`]'s `select` ultimately discards (both `select` arms are always
/// evaluated, so a discarded branch's own division must still be finite).
pub const NEAR_CLIP_DENOM_EPS: f32 = 1.0e-6;

/// The per-triangle CONSTANT basis every per-pixel VB evaluation
/// ([`vb_barycentric_eval_body`], [`vb_interp_body`], [`vb_uv_grad_body`]) is driven from:
/// the screen-space barycentric gradients ([`vb_barycentric_grad_body`]) plus the shared
/// evaluation anchor — vertex 0's own screen position. Computed ONCE per triangle (the
/// `vb_geom_fetch.hlsli` per-pixel fetch, R8), reused for the pixel's `lambda` plus every
/// attribute channel (position, uv, ...) at that pixel.
#[derive(Clone, Copy, Debug)]
pub struct BaryBasis<S> {
    /// `dlambda_i/dx` for `i = 0, 1, 2` (screen-space, constant per triangle).
    pub dlambda_dx: [S; 3],
    /// `dlambda_i/dy` for `i = 0, 1, 2` (screen-space, constant per triangle).
    pub dlambda_dy: [S; 3],
    /// The shared evaluation anchor's screen-space `x` — vertex 0's own `x`.
    pub x0: S,
    /// The shared evaluation anchor's screen-space `y` — vertex 0's own `y`.
    pub y0: S,
}

/// Computes the per-triangle constant screen-space barycentric gradients
/// `(dlambda_i/dx, dlambda_i/dy)` for a triangle with screen-space vertices `(x[i], y[i])`
/// — DAIS Appendix A. `D = det(p2 - p1, p0 - p1)` (twice the signed area) is the ONLY
/// divide; every gradient component is one multiply by `1/D` (transcendental-free).
///
/// Callers combine this with vertex 0's own screen position into a [`BaryBasis`] (see the
/// module doc's shared-anchor evaluation).
///
/// A truly degenerate (`D == 0`, zero-area) triangle produces `Inf`/`NaN` gradients — out of
/// scope for this rung: a zero-area triangle rasterizes no pixels, so a real caller (the R8
/// `vb_geom_fetch` fetch, gated on a rasterized pixel) never reaches this with `D == 0`. A
/// near-degenerate (small but nonzero `D`, a "sliver" triangle) triangle stays finite.
#[inline]
pub fn vb_barycentric_grad_body<S: FieldScalar>(x: [S; 3], y: [S; 3]) -> ([S; 3], [S; 3]) {
    // D = det(p2 - p1, p0 - p1) = (x2-x1)*(y0-y1) - (y2-y1)*(x0-x1).
    let d = (x[2].sub(x[1]))
        .mul(y[0].sub(y[1]))
        .sub((y[2].sub(y[1])).mul(x[0].sub(x[1])));
    let rcp_d = S::lit(1.0).div(d);

    let dlambda_dx = [
        (y[1].sub(y[2])).mul(rcp_d), // dlambda_0/dx = (y1-y2)/D
        (y[2].sub(y[0])).mul(rcp_d), // dlambda_1/dx = (y2-y0)/D
        (y[0].sub(y[1])).mul(rcp_d), // dlambda_2/dx = (y0-y1)/D
    ];
    let dlambda_dy = [
        (x[2].sub(x[1])).mul(rcp_d), // dlambda_0/dy = (x2-x1)/D
        (x[0].sub(x[2])).mul(rcp_d), // dlambda_1/dy = (x0-x2)/D
        (x[1].sub(x[0])).mul(rcp_d), // dlambda_2/dy = (x1-x0)/D
    ];
    (dlambda_dx, dlambda_dy)
}

/// Evaluates the per-pixel barycentric weights `lambda = [l0, l1, l2]` at screen position
/// `(px, py)` from a triangle's [`BaryBasis`] (the shared-anchor evaluation the module doc
/// derives): `lambda(x,y) = lambda(anchor) + dx*dlambda/dx + dy*dlambda/dy`, `dx = px -
/// basis.x0`, `dy = py - basis.y0`, and `lambda(anchor) = (1, 0, 0)` (vertex 0 is its own
/// barycentric coordinate 1). Sums to 1 everywhere (the three `dlambda/dx` — and `dy` —
/// components each telescope to 0, a property of the DAIS gradient itself, not of this
/// evaluation), and at `(px,py) == (basis.x0, basis.y0)` returns exactly `(1, 0, 0)`.
#[inline]
pub fn vb_barycentric_eval_body<S: FieldScalar>(basis: BaryBasis<S>, px: S, py: S) -> [S; 3] {
    let dx = px.sub(basis.x0);
    let dy = py.sub(basis.y0);

    let l0 = S::lit(1.0)
        .add(basis.dlambda_dx[0].mul(dx))
        .add(basis.dlambda_dy[0].mul(dy));
    let l1 = basis.dlambda_dx[1].mul(dx).add(basis.dlambda_dy[1].mul(dy));
    let l2 = basis.dlambda_dx[2].mul(dx).add(basis.dlambda_dy[2].mul(dy));
    [l0, l1, l2]
}

/// Perspective-correct interpolation of ONE scalar vertex-attribute channel `a = [a0, a1,
/// a2]` (paired with the vertex clip-space `w = [w0, w1, w2]`) at screen position `(px,
/// py)`, PLUS its screen-space derivatives — DAIS Eq. 1-2/5, driven by the SAME
/// [`BaryBasis`] [`vb_barycentric_eval_body`] uses. Returns `[value, d(value)/dx,
/// d(value)/dy]`.
///
/// `N = (a/w)_s + dx*d(a/w)/dx + dy*d(a/w)/dy`, `Wr = (1/w)_s + dx*d(1/w)/dx +
/// dy*d(1/w)/dy`, `value = N / Wr` — both `N` and `Wr` are AFFINE in `(dx, dy)` (their
/// gradients, `d_n_dx`/`d_n_dy`/`d_w_dx`/`d_w_dy` below, are the SAME constants
/// everywhere), so the derivative of the RATIO `value = N/Wr` needs the quotient rule:
/// `d(value)/dx = (d_n_dx*Wr - value_num*d_w_dx) / Wr^2` (and the `dy` sibling). This is the
/// CORRECTED McLaren/Hill form (see the module doc): the gradient folds `a_i/w_i`, not the
/// raw `a_i`, and the final derivative applies the quotient rule rather than reusing the
/// raw `dlambda/dx` directly.
#[inline]
pub fn vb_interp_body<S: FieldScalar>(
    basis: BaryBasis<S>,
    px: S,
    py: S,
    a: [S; 3],
    w: [S; 3],
) -> [S; 3] {
    let dx = px.sub(basis.x0);
    let dy = py.sub(basis.y0);

    let inv_w = [
        S::lit(1.0).div(w[0]),
        S::lit(1.0).div(w[1]),
        S::lit(1.0).div(w[2]),
    ];
    let a_over_w = [a[0].mul(inv_w[0]), a[1].mul(inv_w[1]), a[2].mul(inv_w[2])];

    // d(a/w)/dx = sum_i dlambda_i/dx * (a_i/w_i); the `dy` sibling is analogous. `a/w` is a
    // fixed linear combination of `lambda_i`, so its screen-space gradient is the SAME
    // combination of the barycentric gradient constants.
    let d_n_dx = basis.dlambda_dx[0]
        .mul(a_over_w[0])
        .add(basis.dlambda_dx[1].mul(a_over_w[1]))
        .add(basis.dlambda_dx[2].mul(a_over_w[2]));
    let d_n_dy = basis.dlambda_dy[0]
        .mul(a_over_w[0])
        .add(basis.dlambda_dy[1].mul(a_over_w[1]))
        .add(basis.dlambda_dy[2].mul(a_over_w[2]));
    let d_w_dx = basis.dlambda_dx[0]
        .mul(inv_w[0])
        .add(basis.dlambda_dx[1].mul(inv_w[1]))
        .add(basis.dlambda_dx[2].mul(inv_w[2]));
    let d_w_dy = basis.dlambda_dy[0]
        .mul(inv_w[0])
        .add(basis.dlambda_dy[1].mul(inv_w[1]))
        .add(basis.dlambda_dy[2].mul(inv_w[2]));

    // Value at the vertex-0 anchor: (a/w)_s = a0/w0, (1/w)_s = 1/w0.
    let n = a_over_w[0].add(dx.mul(d_n_dx)).add(dy.mul(d_n_dy));
    let wr = inv_w[0].add(dx.mul(d_w_dx)).add(dy.mul(d_w_dy));
    let value = n.div(wr);

    // Quotient rule: d(value)/dx = (d_n_dx*wr - n*d_w_dx) / wr^2. `d_n_dx`/`d_w_dx` are the
    // CONSTANTS above (N/Wr are affine in (x,y), so their own gradient IS the constant
    // gradient — no re-derivation at the pixel).
    let wr2 = wr.mul(wr);
    let d_value_dx = d_n_dx.mul(wr).sub(n.mul(d_w_dx)).div(wr2);
    let d_value_dy = d_n_dy.mul(wr).sub(n.mul(d_w_dy)).div(wr2);

    [value, d_value_dx, d_value_dy]
}

/// Screen-space texcoord gradients `(du/dx, du/dy, dv/dx, dv/dy)` for `SampleGrad`, FREE
/// from [`vb_interp_body`]'s own machinery: `u`/`v` are each interpolated as their OWN
/// perspective-correct scalar attribute channel (one [`vb_interp_body`] call per channel);
/// only the derivative pair of each is kept here (a caller wanting the `u`/`v` VALUES
/// themselves calls [`vb_interp_body`] directly for shading, so this returns ONLY the 4
/// derivatives `SampleGrad(tex, sampler, uv, float2(du_dx,dv_dx), float2(du_dy,dv_dy))`
/// needs).
#[inline]
pub fn vb_uv_grad_body<S: FieldScalar>(
    basis: BaryBasis<S>,
    px: S,
    py: S,
    u: [S; 3],
    v: [S; 3],
    w: [S; 3],
) -> [S; 4] {
    let [_u_value, du_dx, du_dy] = vb_interp_body(basis, px, py, u, w);
    let [_v_value, dv_dx, dv_dy] = vb_interp_body(basis, px, py, v, w);
    [du_dx, du_dy, dv_dx, dv_dy]
}

/// Floors the MAGNITUDE of a near-clip edge-crossing denominator to
/// [`NEAR_CLIP_DENOM_EPS`], preserving sign — the divide-by-zero guard [`vb_near_clip_body`]
/// applies to every edge-crossing `t`, even on a `select` arm the caller ultimately
/// discards (both arms are always evaluated, see the module doc).
#[inline]
fn safe_denom<S: FieldScalar>(d: S) -> S {
    let floor = S::lit(NEAR_CLIP_DENOM_EPS);
    let mag = d.abs().max(floor);
    S::select(d.lt(S::lit(0.0)), mag.neg(), mag)
}

/// Component-wise `lerp` of two clip-space vertices (`[x, y, z, w]`) by `t`.
#[inline]
fn lerp_vertex4<S: FieldScalar>(a: [S; 4], b: [S; 4], t: S) -> [S; 4] {
    [
        a[0].lerp(b[0], t),
        a[1].lerp(b[1], t),
        a[2].lerp(b[2], t),
        a[3].lerp(b[3], t),
    ]
}

/// Component-wise value-select between two clip-space vertices.
#[inline]
fn select_vertex4<S: FieldScalar>(cond: S::Mask, t: [S; 4], e: [S; 4]) -> [S; 4] {
    [
        S::select(cond, t[0], e[0]),
        S::select(cond, t[1], e[1]),
        S::select(cond, t[2], e[2]),
        S::select(cond, t[3], e[3]),
    ]
}

/// Shrinks (never subdivides) a triangle's three clip-space vertices `v[i] = [x, y, z, w]`
/// so every returned `w` is `> `[`NEAR_CLIP_W_EPSILON`] before the perspective
/// divide/screen projection — a simplified Blinn-Newell near-plane clip (the module doc:
/// hardware/analytic `ddx`/`ddy` are unstable as `w -> 0`, so the near plane is handled
/// BEFORE rasterization, not by the derivative machinery).
///
/// For each vertex `i` with `w_i <= `[`NEAR_CLIP_W_EPSILON`] ("bad"), the other two vertices
/// `j`, `k` are inspected (branchless — every candidate below is ALWAYS computed, the
/// `select`s just pick which one survives):
/// - **both good**: `v[i]` is replaced by the AVERAGE of the two near-plane crossings
///   `lerp(v[i], v[j], t_ij)` and `lerp(v[i], v[k], t_ik)` (each `t` solves `w ==
///   NEAR_CLIP_W_EPSILON` linearly along that edge) — this keeps exactly 3 vertices (never
///   4), the "simplified" (non-polygon-clip) trade-off the plan calls for.
/// - **one good** (say `j`): `v[i]` is replaced by `lerp(v[i], v[j], t_ij)` alone.
/// - **neither good** (the whole triangle is behind the near plane): `v[i]` is left in place
///   with `w` hard-clamped to [`NEAR_CLIP_W_EPSILON`] — not geometrically meaningful, but
///   NaN/Inf-safe; a fully-behind triangle is expected to be culled upstream (out of scope
///   here).
///
/// A vertex that is already good (`w_i > `[`NEAR_CLIP_W_EPSILON`]) is returned UNCHANGED
/// (exact passthrough) — a fully in-front triangle is a byte-identical no-op.
#[inline]
pub fn vb_near_clip_body<S: FieldScalar>(v: [[S; 4]; 3]) -> [[S; 4]; 3] {
    let eps = S::lit(NEAR_CLIP_W_EPSILON);
    let w = [v[0][3], v[1][3], v[2][3]];
    let bad = [w[0].le(eps), w[1].le(eps), w[2].le(eps)];
    let good = [w[0].gt(eps), w[1].gt(eps), w[2].gt(eps)];

    let mut out = v;
    for i in 0..3 {
        let j = (i + 1) % 3;
        let k = (i + 2) % 3;

        let t_ij = (eps.sub(w[i])).div(safe_denom(w[j].sub(w[i]))).clamp01();
        let cand_j = lerp_vertex4(v[i], v[j], t_ij);
        let t_ik = (eps.sub(w[i])).div(safe_denom(w[k].sub(w[i]))).clamp01();
        let cand_k = lerp_vertex4(v[i], v[k], t_ik);
        let half = S::lit(0.5);
        let avg_jk = [
            cand_j[0].add(cand_k[0]).mul(half),
            cand_j[1].add(cand_k[1]).mul(half),
            cand_j[2].add(cand_k[2]).mul(half),
            cand_j[3].add(cand_k[3]).mul(half),
        ];
        let self_clamped = [v[i][0], v[i][1], v[i][2], w[i].max(eps)];

        // good_j=T,good_k=T -> avg_jk; good_j=T,good_k=F -> cand_j;
        // good_j=F,good_k=T -> cand_k; good_j=F,good_k=F -> self_clamped.
        let shrunk = select_vertex4(
            good[j],
            select_vertex4(good[k], avg_jk, cand_j),
            select_vertex4(good[k], cand_k, self_clamped),
        );
        out[i] = select_vertex4(bad[i], shrunk, v[i]);
    }
    out
}
