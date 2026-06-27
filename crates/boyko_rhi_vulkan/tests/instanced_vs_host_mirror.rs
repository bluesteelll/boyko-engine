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

use boyko_rhi_vulkan::compute::SDF_TRACE_T_MAX;

/// The host mirror of the gbuffer fragment shader's `static const float T_MAX = 10.0`. It
/// MUST equal the marcher's [`SDF_TRACE_T_MAX`] (the C2 sync-pin) — asserted directly in
/// [`t_max_sync_pin_matches_marcher`] below, and structurally relied on by the round-trip
/// tests (they decode with `SDF_TRACE_T_MAX` but the FS encodes with this).
const GBUFFER_T_MAX: f32 = 10.0;

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
    length(sub(cam_eye, world)) / GBUFFER_T_MAX
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

/// The C2 sync-pin: the FS depth normalizer equals the marcher's ray-t decode constant. A
/// drift between the two hand-written HLSL literals breaks every perspective mesh pixel's
/// depth ownership.
#[test]
fn t_max_sync_pin_matches_marcher() {
    assert_eq!(
        GBUFFER_T_MAX, SDF_TRACE_T_MAX,
        "the gbuffer fragment's T_MAX must equal the marcher's SDF_TRACE_T_MAX decode constant"
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
        // Self-consistency: the depth is exactly the euclidean eye->world distance / T_MAX.
        let euclid = length(eye_rel);
        assert!(
            (depth * GBUFFER_T_MAX - euclid).abs() < 1e-4,
            "FS depth must encode the euclidean eye->surface distance: depth*T_MAX={} euclid={}",
            depth * GBUFFER_T_MAX,
            euclid
        );

        // The marcher's ray for THIS pixel: ro = eye, rd = unit dir toward the world point.
        let ro = cam_eye;
        let rd = normalize(sub(world, cam_eye));
        // Decode t_mesh with the MARCHER's constant (md * T_MAX), then reconstruct P.
        let t_mesh = depth * SDF_TRACE_T_MAX;
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
