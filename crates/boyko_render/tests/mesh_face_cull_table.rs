//! **RD6 / D-DS — H1: the cull table on the real view-projection builders.**
//!
//! glTF 2.0 "Double Sided": "When this value is false, back-face culling is enabled, i.e., only
//! front-facing triangles are rendered. When this value is true, back-face culling is disabled,
//! triangles are rendered from both sides". A triangle's front is its counter-clockwise side by
//! the right-hand rule, and "the determinant of the node's global transform defines the winding
//! order".
//!
//! Each row places one triangle in front of a viewer — a camera (the Deferred and the
//! Forward/VB projection builders) or a light (CSM cascade 0, a spot face, the six point-cube
//! faces) — through a model transform (identity, a mirror, a rotated mirror), with the triangle's
//! authored front toward the viewer or away from it. It projects the three vertices through the
//! builder's real matrix to framebuffer coordinates on a positive-height viewport, takes the
//! Vulkan polygon area (`a = −½ Σ (xᵢ·yᵢ₊₁ − xᵢ₊₁·yᵢ)`; `VK_FRONT_FACE_COUNTER_CLOCKWISE`: front ⇔
//! `a > 0`), and applies the cull member `CullMode::for_mesh_face(mesh_face_bits(..), kind)`
//! selects. The policy it must reproduce:
//!
//! - camera pass: rasterised ⇔ (authored front toward the eye) ∨ double-sided;
//! - light pass (K1 — the caster keeps HEAD's light-facing set for single-sided content):
//!   rasterised ⇔ (authored front toward the light) ∨ double-sided.
//!
//! [`facing_premise_holds_on_every_builder`] is the premise the table rests on (camera: an
//! authored front toward the eye is FRONT-facing; light: an authored front toward the light is
//! BACK-facing; a mirror swaps both). It is green on HEAD; if it ever goes red the builders'
//! chirality moved and the cull table must be re-derived, not re-tuned.

use boyko_math::{Affine3A, Vec3};
use boyko_render::csm_config::{CsmConfig, CsmFit, resolve_csm};
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::mesh_face::{mesh_face_bits, model_det};
use boyko_render::shadow_atlas::{
    PointShadowInput, ShadowConfig, SpotShadowInput, resolve_shadow_atlas_inputs,
};
use boyko_render::view::{forward_view_proj_rows, marcher_view_proj_rows};
use boyko_rhi::enums::{CullMode, MeshPassKind};
use boyko_scene::{Projection, ViewUniform};

const W: u32 = 1280;
const H: u32 = 720;

/// One viewer: its name, the pass kind, a row-major world→clip matrix, and its world eye (for a
/// directional light, a point far along the light's direction).
struct Viewer {
    name: String,
    kind: MeshPassKind,
    vp: [[f32; 4]; 4],
    eye: [f32; 3],
}

fn transpose(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = m[c][r];
        }
    }
    out
}

/// The triangle's centre, where every viewer looks.
const TARGET: [f32; 3] = [0.0, 0.5, 0.0];

fn camera_view(eye: Vec3) -> ViewUniform {
    let world = Affine3A::look_at_rh(eye, Vec3::new(TARGET[0], TARGET[1], TARGET[2]), Vec3::new(0.0, 1.0, 0.0));
    ViewUniform::from_camera(
        world,
        Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: W as f32 / H as f32, near: 0.1, far: 100.0 },
    )
}

/// Every viewer of the table, each looking at [`TARGET`] from `+Z`-ish so one triangle serves all
/// (every CSM cascade and every point-cube face is listed; only those the triangle lands in
/// judge it).
fn viewers() -> Vec<Viewer> {
    let mut out = Vec::new();
    let eye = Vec3::new(0.3, 0.8, 6.0);
    let view = camera_view(eye);
    out.push(Viewer {
        name: "camera / Deferred (marcher_view_proj_rows)".into(),
        kind: MeshPassKind::CameraView,
        vp: marcher_view_proj_rows(&view, W, H),
        eye: [eye.x, eye.y, eye.z],
    });
    out.push(Viewer {
        name: "camera / Forward + VB (forward_view_proj_rows)".into(),
        kind: MeshPassKind::CameraView,
        vp: forward_view_proj_rows(&view, W, H),
        eye: [eye.x, eye.y, eye.z],
    });

    // CSM cascade 0: the sun shines from +Z (slightly up), so the light sits on the +Z side.
    let sun_dir = [0.1, 0.3, 1.0];
    let cfg = CsmConfig { cascade_count: 4, ..CsmConfig::default() };
    let csm_camera = camera_view(Vec3::new(0.0, 0.5, 4.0));
    let csm = resolve_csm(&cfg, &csm_camera, sun_dir, CsmFit::NONE);
    assert!(csm.active_count >= 1, "fixture: CSM resolves at least one cascade");
    let len = (sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2]).sqrt();
    for (i, cascade) in csm.cascades.iter().enumerate().take(csm.active_count as usize) {
        out.push(Viewer {
            name: format!("light / CSM cascade {i} (resolve_csm)"),
            kind: MeshPassKind::LightView,
            vp: transpose(cascade.view_proj),
            eye: [1000.0 * sun_dir[0] / len, 1000.0 * sun_dir[1] / len, 1000.0 * sun_dir[2] / len],
        });
    }

    // A spot on +Z shining at the triangle, and a point light whose six cube faces see it from
    // every axis (each face rasterises its own projection; only the faces the triangle lands in
    // judge it).
    let atlas = ShadowConfig { enabled: true, ..ShadowConfig::default() };
    let spot_pos = [0.2, 0.9, 5.0];
    let spot = SpotShadowInput {
        position: spot_pos,
        axis: [TARGET[0] - spot_pos[0], TARGET[1] - spot_pos[1], TARGET[2] - spot_pos[2]],
        outer_rad: 0.6,
        range: 30.0,
        priority: 1.0,
    };
    let mut spot_slots = [0u32; 1];
    let resolved = resolve_shadow_atlas_inputs(&atlas, &[spot], &[], &mut spot_slots, &mut []);
    assert_eq!(resolved.active_layers, 1, "fixture: the spot takes one atlas layer");
    out.push(Viewer {
        name: "light / spot face (resolve_shadow_atlas_inputs)".into(),
        kind: MeshPassKind::LightView,
        vp: transpose(resolved.faces[spot_slots[0] as usize].view_proj),
        eye: spot_pos,
    });
    for point_pos in [[0.1, 0.6, 3.0], [3.0, 0.6, 0.1], [0.1, 3.5, 0.2], [-3.0, 0.4, 0.1], [0.1, -2.5, 0.2], [0.2, 0.4, -3.0]] {
        let point = PointShadowInput { position: point_pos, range: 30.0, priority: 1.0 };
        let mut point_slots = [0u32; 1];
        let resolved = resolve_shadow_atlas_inputs(&atlas, &[], &[point], &mut [], &mut point_slots);
        assert_eq!(resolved.active_layers, 6, "fixture: the point takes six atlas layers");
        for face in 0..6 {
            out.push(Viewer {
                name: format!("light / point {point_pos:?} cube face {face}"),
                kind: MeshPassKind::LightView,
                vp: transpose(resolved.faces[point_slots[0] as usize + face].view_proj),
                eye: point_pos,
            });
        }
    }
    out
}

/// The model transforms: identity, a mirror in X, and a rotated, scaled mirror.
fn models() -> Vec<(&'static str, [[f32; 3]; 3])> {
    let (s, c) = 25f32.to_radians().sin_cos();
    vec![
        ("identity", [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        ("mirror (-1,1,1)", [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        // rot_y(25°) · diag(-1.5, 1.5, 1.5)
        ("rotated mirror", [[-1.5 * c, 0.0, 1.5 * s], [0.0, 1.5, 0.0], [1.5 * s, 0.0, 1.5 * c]]),
    ]
}

fn mul3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The cofactor matrix's action on `n` — `det · M⁻ᵀ · n`, the normal transform up to a positive
/// scale when `det > 0` and up to a NEGATIVE one when `det < 0`; multiplied by `sign(det)` it is
/// the authored normal's world direction for either handedness.
fn world_normal(m: &[[f32; 3]; 3], n: [f32; 3]) -> [f32; 3] {
    let cols = [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]];
    let cof = [cross(cols[1], cols[2]), cross(cols[2], cols[0]), cross(cols[0], cols[1])];
    let det = dot(cols[0], cof[0]);
    let w = [
        cof[0][0] * n[0] + cof[1][0] * n[1] + cof[2][0] * n[2],
        cof[0][1] * n[0] + cof[1][1] * n[1] + cof[2][1] * n[2],
        cof[0][2] * n[0] + cof[1][2] * n[1] + cof[2][2] * n[2],
    ];
    let s = det.signum();
    [w[0] * s, w[1] * s, w[2] * s]
}

/// A triangle whose authored (counter-clockwise) front faces the model-space direction `toward`
/// (either `+Z` or `−Z`), centred on the origin in the XY plane, lifted to [`TARGET`] after the
/// model transform.
fn triangle(toward_plus_z: bool) -> [[f32; 3]; 3] {
    let (a, b, c) = ([-0.4, -0.3, 0.0], [0.4, -0.3, 0.0], [0.0, 0.4, 0.0]);
    if toward_plus_z { [a, b, c] } else { [a, c, b] }
}

/// Projects `p` to framebuffer coordinates through a row-major `vp` and a positive-height
/// viewport; `None` when it lands behind the viewer or outside the clip volume's xy.
fn to_framebuffer(vp: &[[f32; 4]; 4], p: [f32; 3]) -> Option<[f64; 2]> {
    let row = |r: usize| vp[r][0] * p[0] + vp[r][1] * p[1] + vp[r][2] * p[2] + vp[r][3];
    let (x, y, w) = (row(0), row(1), row(3));
    if w <= 1e-6 {
        return None;
    }
    let (nx, ny) = (x / w, y / w);
    if !(-1.0..=1.0).contains(&nx) || !(-1.0..=1.0).contains(&ny) {
        return None;
    }
    Some([f64::from((nx + 1.0) * 0.5 * W as f32), f64::from((ny + 1.0) * 0.5 * H as f32)])
}

/// The Vulkan polygon area of a framebuffer-space triangle.
fn vulkan_area(f: [[f64; 2]; 3]) -> f64 {
    let mut s = 0.0;
    for i in 0..3 {
        let j = (i + 1) % 3;
        s += f[i][0] * f[j][1] - f[j][0] * f[i][1];
    }
    -0.5 * s
}

/// One row's outcome: does the triangle land in this viewer, is it front-facing, and does its
/// authored front face the viewer?
struct Row {
    front_facing: bool,
    authored_toward_viewer: bool,
    mirrored: bool,
    instance: InstanceModelCol,
}

fn evaluate(viewer: &Viewer, m: &[[f32; 3]; 3], toward_plus_z: bool) -> Option<Row> {
    let tri = triangle(toward_plus_z);
    let world: Vec<[f32; 3]> = tri
        .iter()
        .map(|v| {
            let w = mul3(m, *v);
            [w[0] + TARGET[0], w[1] + TARGET[1], w[2] + TARGET[2]]
        })
        .collect();
    let fb = [to_framebuffer(&viewer.vp, world[0])?, to_framebuffer(&viewer.vp, world[1])?, to_framebuffer(&viewer.vp, world[2])?];
    let area = vulkan_area(fb);
    assert!(area.abs() > 1e-3, "{}: the fixture triangle is edge-on", viewer.name);
    let n_model = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
    let n_world = world_normal(m, n_model);
    let instance = InstanceModelCol {
        rows: [
            [m[0][0], m[0][1], m[0][2], TARGET[0]],
            [m[1][0], m[1][1], m[1][2], TARGET[1]],
            [m[2][0], m[2][1], m[2][2], TARGET[2]],
        ],
    };
    Some(Row {
        front_facing: area > 0.0,
        authored_toward_viewer: dot(n_world, sub(viewer.eye, TARGET)) > 0.0,
        mirrored: model_det(&instance) < 0.0,
        instance,
    })
}

fn rasterised(cull: CullMode, front_facing: bool) -> bool {
    match cull {
        CullMode::None => true,
        CullMode::Back => front_facing,
        CullMode::Front => !front_facing,
    }
}

/// The premise the table rests on, per builder: a camera sees an authored front toward it as
/// FRONT-facing, a light as BACK-facing (the light basis is mirrored relative to the camera's),
/// and a negative determinant swaps both. Green on HEAD; a red here means a builder's chirality
/// changed — re-derive the cull table (STOP), do not re-tune it.
#[test]
fn facing_premise_holds_on_every_builder() {
    let mut judged = 0;
    let mut families = families();
    for viewer in viewers() {
        for (mname, m) in models() {
            for toward in [true, false] {
                let Some(row) = evaluate(&viewer, &m, toward) else { continue };
                judged += 1;
                tally(&mut families, &viewer);
                let expect_front = match viewer.kind {
                    MeshPassKind::CameraView => row.authored_toward_viewer != row.mirrored,
                    MeshPassKind::LightView => row.authored_toward_viewer == row.mirrored,
                };
                assert_eq!(
                    row.front_facing, expect_front,
                    "STOP — chirality premise broken: {} / {mname} / authored front {} the viewer: \
                     front_facing = {} (the cull table must be re-derived, not re-tuned)",
                    viewer.name,
                    if row.authored_toward_viewer { "toward" } else { "away from" },
                    row.front_facing
                );
            }
        }
    }
    assert_every_family_judged(&families);
    assert!(judged >= 36, "anti-vacuity: too few rows landed in a viewer ({judged})");
}

/// Every builder family must have judged at least one full model × orientation set — a family
/// whose viewers all missed the triangle would make its rows vacuous.
fn assert_every_family_judged(families: &[(&str, u32)]) {
    for (family, n) in families {
        assert!(*n >= 6, "anti-vacuity: the {family} builder judged {n} rows (< 6) — the fixture missed it");
    }
}

/// Counts a judged row against its viewer's builder family.
fn tally(families: &mut [(&'static str, u32)], viewer: &Viewer) {
    for (family, n) in families.iter_mut() {
        if viewer.name.contains(*family) {
            *n += 1;
        }
    }
}

fn families() -> Vec<(&'static str, u32)> {
    vec![("Deferred", 0), ("Forward + VB", 0), ("CSM", 0), ("spot", 0), ("point", 0)]
}

/// H1: the member `CullMode::for_mesh_face(mesh_face_bits(..), kind)` selects rasterises exactly
/// the glTF policy's set, for single- and double-sided materials, on every builder and model.
#[test]
fn cull_table_matches_the_gltf_policy() {
    let mut failures = Vec::new();
    let mut judged = 0;
    let mut families = families();
    for viewer in viewers() {
        for (mname, m) in models() {
            for toward in [true, false] {
                let Some(row) = evaluate(&viewer, &m, toward) else { continue };
                tally(&mut families, &viewer);
                for double_sided in [false, true] {
                    judged += 1;
                    let face = mesh_face_bits(double_sided, &row.instance);
                    let cull = CullMode::for_mesh_face(face, viewer.kind);
                    let got = rasterised(cull, row.front_facing);
                    let want = row.authored_toward_viewer || double_sided;
                    if got != want {
                        failures.push(format!(
                            "{} / {mname} / {} / authored front {} the viewer: {} (cull {cull:?}), \
                             the policy says {}",
                            viewer.name,
                            if double_sided { "double-sided" } else { "single-sided" },
                            if row.authored_toward_viewer { "toward" } else { "away from" },
                            if got { "rasterised" } else { "culled" },
                            if want { "rasterised" } else { "culled" },
                        ));
                    }
                }
            }
        }
    }
    assert_every_family_judged(&families);
    assert!(judged >= 72, "anti-vacuity: too few rows landed in a viewer ({judged})");
    assert!(failures.is_empty(), "RED: {} of {judged} rows disagree with the glTF policy:\n{}", failures.len(), failures.join("\n"));
}
