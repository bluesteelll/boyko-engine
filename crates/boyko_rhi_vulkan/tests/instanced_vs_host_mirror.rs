//! Mesh foundation M2 — the C2 NUMERICAL gate (CPU, no GPU).
//!
//! A host mirror of the M1/M2 INSTANCED gbuffer vertex transform + the marcher's mesh-depth
//! decode, proving the perspective depth the instanced VS writes ROUND-TRIPS back to the
//! placed world point along the marcher's ray — INDEPENDENT of any GPU.
//!
//! The GPU path is:
//!   * VS (`gbuffer_mrt.vs.hlsl`, instanced arm): `world = m3 * localpos + t`;
//!     `eye_rel = cam_eye - world`.
//!   * FS (`gbuffer_mrt.fs.hlsl`, perspective): `SV_Depth = length(eye_rel) / T_MAX`.
//!   * Marcher (`sdf_gbuffer_composite.hlsl`): a mesh pixel decodes `t_mesh = md * T_MAX`,
//!     reconstructing the surface point as `ro + rd * t_mesh` with a UNIT `rd`.
//!
//! For a pixel whose ray passes through the placed world point, `ro == cam_eye` and
//! `rd == normalize(world - cam_eye)`, so `ro + rd * (depth * T_MAX)` must reconstruct
//! `world` exactly (up to float tolerance) — the C2 invariant the instanced arm depends on
//! under perspective. This test runs unconditionally under `cargo test -p
//! boyko_rhi_vulkan` (no window, no device).

use boyko_rhi_vulkan::compute::MESH_DEPTH_T_MAX;

/// The host mirror of the gbuffer fragment shader's `static const float MESH_DEPTH_T_MAX =
/// 64.0` (the PERSPECTIVE mesh-depth normalizer). It MUST equal the marcher's PERSPECTIVE
/// decode constant [`MESH_DEPTH_T_MAX`] (the C2 sync-pin) — asserted directly in
/// [`mesh_t_max_sync_pin_matches_marcher`] below, and structurally relied on by the round-trip
/// tests (they encode AND decode with this). NOTE: this is DECOUPLED from the marcher's
/// ray-miss bound `SDF_TRACE_T_MAX` (= 10) — the normalizer only sets the depth-buffer range
/// (it cancels in encode→decode), so raster mesh can stand far past the SDF horizon.
const GBUFFER_MESH_T_MAX: f32 = 64.0;

// --- small vec3 helpers (the same arithmetic the HLSL does, scalarized) ---

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let inv = 1.0 / length(a);
    scale(a, inv)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// --- the 3x3 linear part of the instanced affine, mirroring the HLSL `m3` ---

/// The ROW-MAJOR 3x3 linear part of the instanced affine: `m3[i] = model[i].xyz` (the rotation/
/// scale rows, dropping each row's `.w` translation). Mirrors `float3x3 m3 = float3x3(model.r0.xyz,
/// model.r1.xyz, model.r2.xyz)` in `gbuffer_mrt.vs.hlsl`.
fn m3_of(model: [[f32; 4]; 3]) -> [[f32; 3]; 3] {
    [
        [model[0][0], model[0][1], model[0][2]],
        [model[1][0], model[1][1], model[1][2]],
        [model[2][0], model[2][1], model[2][2]],
    ]
}

/// `mul(m, v)` for a row-major 3x3 (each `m[i]` is a row): `out[i] = dot(m[i], v)`. Matches the
/// HLSL `mul(float3x3, float3)` the instanced arm uses for both the position and the normal.
fn mul3(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

/// The determinant of a row-major 3x3. Mirrors the HLSL `det3x3` feeding the W4 degeneracy guard.
fn det3x3(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// The cofactor (adjugate / determinant) inverse of a row-major 3x3. Mirrors the HLSL
/// `inverse3x3` BYTE-FOR-BYTE in arithmetic (same cofactor expansion, same adjugate transpose)
/// so the host proof and the GPU shader compute the same normal matrix. No |det| clamp here —
/// the caller (the normal-matrix builder) applies the W4 guard via [`det3x3`].
fn inverse3x3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let c0 = [
        m[1][1] * m[2][2] - m[1][2] * m[2][1],
        m[1][2] * m[2][0] - m[1][0] * m[2][2],
        m[1][0] * m[2][1] - m[1][1] * m[2][0],
    ];
    let c1 = [
        m[0][2] * m[2][1] - m[0][1] * m[2][2],
        m[0][0] * m[2][2] - m[0][2] * m[2][0],
        m[0][1] * m[2][0] - m[0][0] * m[2][1],
    ];
    let c2 = [
        m[0][1] * m[1][2] - m[0][2] * m[1][1],
        m[0][2] * m[1][0] - m[0][0] * m[1][2],
        m[0][0] * m[1][1] - m[0][1] * m[1][0],
    ];
    let det = m[0][0] * c0[0] + m[0][1] * c0[1] + m[0][2] * c0[2];
    let inv_det = 1.0 / det;
    // Adjugate rows = cofactor columns, each scaled by 1/det (mirrors the HLSL assembly).
    [
        [c0[0] * inv_det, c1[0] * inv_det, c2[0] * inv_det],
        [c0[1] * inv_det, c1[1] * inv_det, c2[1] * inv_det],
        [c0[2] * inv_det, c1[2] * inv_det, c2[2] * inv_det],
    ]
}

/// The transpose of a row-major 3x3.
fn transpose3x3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

/// The instanced VS normal output, mirroring the M4 instanced arm EXACTLY: under
/// `|det(m3)| >= DET_EPS` it is `transpose(inverse3x3(m3)) * normal` (the inverse-transpose);
/// otherwise the W4 guard falls back to `mul(m3, normal)` (the M3 behavior). The guard makes a
/// zero-scale / mirror-singular transform yield a FINITE normal instead of NaN/Inf.
fn instanced_vs_normal(model: [[f32; 4]; 3], normal: [f32; 3]) -> [f32; 3] {
    const DET_EPS: f32 = 1e-8;
    let m3 = m3_of(model);
    let det = det3x3(m3);
    if det.abs() < DET_EPS {
        mul3(m3, normal)
    } else {
        let nm = transpose3x3(inverse3x3(m3));
        mul3(nm, normal)
    }
}

/// The instanced VS transform: a 3x4 ROW-MAJOR affine (`r0`, `r1`, `r2`, each a `[f32; 4]`
/// whose `.xyz` is the rotation/scale row and `.w` the translation component, exactly the
/// `InstanceModelCol` the SSBO carries) applied to a model-space `local` position. Mirrors
/// `world = mul(m3, input.position) + t` in `gbuffer_mrt.vs.hlsl`.
fn instanced_world(model: [[f32; 4]; 3], local: [f32; 3]) -> [f32; 3] {
    let m3_row = |r: [f32; 4]| r[0] * local[0] + r[1] * local[1] + r[2] * local[2];
    [
        m3_row(model[0]) + model[0][3],
        m3_row(model[1]) + model[1][3],
        m3_row(model[2]) + model[2][3],
    ]
}

/// The perspective FS depth: `length(cam_eye - world) / T_MAX` (the euclidean ray-t the
/// fragment writes into `SV_Depth` under `cam_mode == 1`).
fn fs_perspective_depth(cam_eye: [f32; 3], world: [f32; 3]) -> f32 {
    length(sub(cam_eye, world)) / GBUFFER_MESH_T_MAX
}

/// A row-major 3x4 affine: a Y-axis rotation by `yaw` (radians), a uniform `scale`, and a
/// translation `t` — a non-identity placement so the round-trip exercises real rotation +
/// scale + offset (not just translation).
fn yaw_scale_translate(yaw: f32, scale_s: f32, t: [f32; 3]) -> [[f32; 4]; 3] {
    let (s, c) = yaw.sin_cos();
    [
        [c * scale_s, 0.0, s * scale_s, t[0]],
        [0.0, scale_s, 0.0, t[1]],
        [-s * scale_s, 0.0, c * scale_s, t[2]],
    ]
}

/// The C2 sync-pin: the FS perspective depth normalizer equals the marcher's perspective ray-t
/// decode constant. A drift between the two hand-written HLSL literals breaks every perspective
/// mesh pixel's depth ownership.
#[test]
fn mesh_t_max_sync_pin_matches_marcher() {
    assert_eq!(
        GBUFFER_MESH_T_MAX, MESH_DEPTH_T_MAX,
        "the gbuffer fragment's MESH_DEPTH_T_MAX must equal the marcher's perspective decode constant"
    );
}

/// The depth the instanced VS+FS write for a placed vertex ROUND-TRIPS through the marcher's
/// `t_mesh = md * T_MAX` decode back to the SAME world point along the eye ray. This is the
/// C2 numerical proof: if the FS normalizer or the marcher decode drifted, the reconstructed
/// point would diverge from `world`.
#[test]
fn instanced_depth_round_trips_to_world_point() {
    let cam_eye = [0.0_f32, 3.2, 4.5];
    // Three non-identity instance placements at DIFFERENT positions + depths (the M2 demo's
    // intent: perspective foreshortening + distinct depth ownership per instance).
    let models = [
        yaw_scale_translate(0.0, 1.0, [-1.6, 0.51, -1.5]),
        yaw_scale_translate(0.6, 0.7, [1.4, 0.36, -2.2]),
        yaw_scale_translate(-0.3, 1.3, [0.2, 0.9, 0.6]),
    ];
    // A model-space corner of a unit box (so `m3` actually rotates/scales it).
    let local = [0.5_f32, -0.5, 0.5];

    for model in models {
        let world = instanced_world(model, local);

        // The VS varying + FS depth (perspective arm).
        let eye_rel = sub(cam_eye, world);
        let depth = fs_perspective_depth(cam_eye, world);
        // Self-consistency: the depth is exactly the euclidean eye->world distance / normalizer.
        let euclid = length(eye_rel);
        assert!(
            (depth * GBUFFER_MESH_T_MAX - euclid).abs() < 1e-4,
            "FS depth must encode the euclidean eye->surface distance: depth*MESH_T_MAX={} euclid={}",
            depth * GBUFFER_MESH_T_MAX,
            euclid
        );

        // The marcher's ray for THIS pixel: ro = eye, rd = unit dir toward the world point.
        let ro = cam_eye;
        let rd = normalize(sub(world, cam_eye));
        // Decode t_mesh with the marcher's PERSPECTIVE constant (md * MESH_DEPTH_T_MAX), then
        // reconstruct P. The normalizer cancels the FS encode, so `t_mesh == length(eye_rel)`.
        let t_mesh = depth * MESH_DEPTH_T_MAX;
        let reconstructed = add(ro, scale(rd, t_mesh));

        let err = length(sub(reconstructed, world));
        assert!(
            err < 1e-3,
            "ro + rd*(depth*T_MAX) must reconstruct the placed world point: \
             world={world:?} reconstructed={reconstructed:?} err={err}"
        );
    }
}

/// An IDENTITY instance affine reproduces the legacy WORLD-space path exactly: with `m3 ==
/// I` and `t == 0`, `world == local`, so the instanced arm and the legacy arm agree (the M1
/// `GBUFFER_IDENTITY_INSTANCE` contract — the instanced arm with the identity matrix equals
/// the legacy draw). Guards the affine convention (row-major, `.w` = translation).
#[test]
fn identity_instance_reproduces_local_position() {
    let identity = [
        [1.0_f32, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    for local in [[1.0_f32, 2.0, 3.0], [-0.5, 0.5, -0.25], [0.0, 0.0, 0.0]] {
        let world = instanced_world(identity, local);
        assert_eq!(world, local, "identity affine must leave the model-space position unchanged");
    }
}

// === Mesh foundation M4 — the inverse-transpose normal correctness gate (CPU). ===

/// A row-major 3x4 affine: a NON-UNIFORM per-axis scale `(sx, sy, sz)` composed with a Y-axis
/// rotation by `yaw` (radians) and a translation `t`. The non-uniform scale is exactly what
/// makes `mul(m3, normal)` skew the normal off the surface — the M4 inverse-transpose corrects
/// it. The model is `T * R * S` (scale, then rotate, then translate), so the 3x3 linear part is
/// `R * S` (a rotation of a non-uniform diagonal — genuinely non-orthogonal).
fn nonuniform_yaw_translate(yaw: f32, s: [f32; 3], t: [f32; 3]) -> [[f32; 4]; 3] {
    let (sin, cos) = yaw.sin_cos();
    // R (row-major) * diag(sx, sy, sz): scale each COLUMN j of R by s[j].
    [
        [cos * s[0], 0.0, sin * s[2], t[0]],
        [0.0, s[1], 0.0, t[1]],
        [-sin * s[0], 0.0, cos * s[2], t[2]],
    ]
}

/// The M4 correctness proof: under NON-UNIFORM scale, the inverse-transpose normal
/// (`transpose(inverse3x3(m3)) * n`) stays PERPENDICULAR to the transformed surface, whereas the
/// naive `mul(m3, n)` does NOT. The witness: an orthonormal `(n, t1, t2)` triple (a surface
/// patch with normal `n` and two in-plane tangents). After the affine, the surface is spanned by
/// `m3*t1`, `m3*t2`; the correct transformed normal must be ⟂ to BOTH. We assert the
/// inverse-transpose normal satisfies this to tight tolerance AND that the naive `mul(m3, n)`
/// FAILS it (proving the non-uniform scale genuinely skews the naive transform — the test would
/// be vacuous if the chosen scale happened to be uniform). The normals are deliberately OBLIQUE
/// (diagonals mixing the unequally-scaled axes), because an axis-aligned box face is the special
/// case where the naive transform stays perpendicular by accident.
#[test]
fn inverse_transpose_normal_stays_perpendicular_under_nonuniform_scale() {
    // A non-uniform squash/stretch (2x wide, 1x tall, 0.5x deep) + a rotation + an offset.
    let model = nonuniform_yaw_translate(0.7, [2.0, 1.0, 0.5], [0.3, -0.4, 1.1]);
    let m3 = m3_of(model);

    // Surface bases whose normal is NOT aligned to a single scale axis. With axis-aligned box
    // faces the naive `mul(m3, n)` would stay perpendicular by accident (the scaled basis columns
    // are still mutually orthogonal — `dot(s_i*R_i, s_j*R_j) = s_i*s_j*dot(R_i,R_j) = 0`), so the
    // skew the inverse-transpose corrects only shows on OBLIQUE surfaces. Each entry is an
    // orthonormal `(n, t1, t2)` triple whose normal mixes the unequally-scaled axes.
    let inv_sqrt2 = 1.0_f32 / 2.0_f32.sqrt();
    let inv_sqrt3 = 1.0_f32 / 3.0_f32.sqrt();
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 3] = [
        // Normal along the X/Z diagonal (the 2x and 0.5x axes mixed) — the strongest skew.
        ([inv_sqrt2, 0.0, inv_sqrt2], [inv_sqrt2, 0.0, -inv_sqrt2], [0.0, 1.0, 0.0]),
        // Normal along the X/Y diagonal (2x and 1x mixed).
        ([inv_sqrt2, inv_sqrt2, 0.0], [-inv_sqrt2, inv_sqrt2, 0.0], [0.0, 0.0, 1.0]),
        // Normal along the full XYZ diagonal — all three scales mixed.
        (
            [inv_sqrt3, inv_sqrt3, inv_sqrt3],
            [inv_sqrt2, -inv_sqrt2, 0.0],
            [inv_sqrt2 * inv_sqrt3, inv_sqrt2 * inv_sqrt3, -2.0 * inv_sqrt2 * inv_sqrt3],
        ),
    ];

    for (n, t1, t2) in faces {
        // The transformed surface tangents span the post-affine face plane.
        let bt1 = mul3(m3, t1);
        let bt2 = mul3(m3, t2);

        // The M4 normal (inverse-transpose) — normalize so the dot tolerance is scale-free.
        let n_correct = normalize(instanced_vs_normal(model, n));
        let perp1 = dot(n_correct, bt1) / length(bt1);
        let perp2 = dot(n_correct, bt2) / length(bt2);
        assert!(
            perp1.abs() < 1e-5 && perp2.abs() < 1e-5,
            "inverse-transpose normal must stay perpendicular to the transformed surface: \
             n_correct={n_correct:?} perp1={perp1} perp2={perp2}"
        );

        // The naive `mul(m3, n)` is NOT perpendicular under this non-uniform scale (the bug M4
        // fixes). If it WERE perpendicular, the test scene would be effectively uniform-scale and
        // the proof vacuous — so we assert the naive transform measurably fails.
        let n_naive = normalize(mul3(m3, n));
        let naive_perp1 = dot(n_naive, bt1) / length(bt1);
        let naive_perp2 = dot(n_naive, bt2) / length(bt2);
        assert!(
            naive_perp1.abs() > 1e-2 || naive_perp2.abs() > 1e-2,
            "the naive mul(m3, n) must SKEW off the surface under non-uniform scale (else the \
             test is vacuous): n_naive={n_naive:?} naive_perp1={naive_perp1} naive_perp2={naive_perp2}"
        );
    }
}

/// The W4 degeneracy guard: a near-singular model (a zero on one scale axis ⇒ `det ≈ 0`) yields
/// a FINITE normal, never NaN/Inf. Without the `abs(det) < DET_EPS` fallback, `inverse3x3`
/// divides by ~0 and the normal MRT is poisoned (black/garbage lighting). The guard substitutes
/// the M3 `mul(m3, n)`, which is finite. We test a fully-collapsed Z axis (det == 0) and a
/// barely-collapsed one (|det| below DET_EPS).
#[test]
fn degenerate_model_yields_finite_normal() {
    let degenerates = [
        // Z axis fully flattened (det == 0 exactly).
        nonuniform_yaw_translate(0.4, [1.5, 0.9, 0.0], [0.0, 0.0, 0.0]),
        // Z axis collapsed below DET_EPS (det ≈ 1.5 * 0.9 * 5e-9 ≈ 6.75e-9 < 1e-8).
        nonuniform_yaw_translate(0.4, [1.5, 0.9, 5e-9], [0.0, 0.0, 0.0]),
    ];
    for model in degenerates {
        let det = det3x3(m3_of(model));
        assert!(det.abs() < 1e-8, "test setup: the model must be degenerate (|det|={det})");
        for n in [[0.0_f32, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            let out = instanced_vs_normal(model, n);
            assert!(
                out.iter().all(|c| c.is_finite()),
                "the W4 guard must keep the normal finite on a degenerate model: n={n:?} out={out:?}"
            );
        }
    }
}
