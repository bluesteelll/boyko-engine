//! IBL1 R1 (`ibl1/cut.md` C11) — **the environment arm on every lit path, against the host chain**.
//!
//! Nine cells, one per lit producer an armed boot builds: Deferred × {Mesh, Sdf} (`deferred_pbr`),
//! Forward × {Mesh, Sdf} (`forward_opaque`, `sdf_forward_march`), ForwardPlus × Mesh
//! (`forward_opaque_froxel`), VB fused (`vb_resolve`), VB classified (`vb_shade`), VB split
//! (`vb_shade_split`, armed by the measurement-only `BOYKO_SDF_MESH=host`, which requests no AO or
//! shadow term) and VB × Sdf (`sdf_forward_march`). Each cell boots ARMED (`EnvSource::Studio`),
//! spawns an `EnvironmentLight` at intensity 1 and yaw [`YAW`], and renders three metal spheres
//! (base 1, metallic 1, roughness 0.05 / 0.3 / 0.6) with NO `DirectionalLight`, no `SkyLight`, no
//! SSAO and no emissive (rule (xvi): sun, diffuse, AO and emissive are zero by construction), so
//! every sphere pixel's `lit` is the environment arm alone:
//!
//! `lit = (f0·A + B)·env_spec·energy_comp × pre_exposure`, `f0 = 1`,
//! `env_spec = env_specular_host(spec_dominant_dir(n, R, pR) rotated by the yaw)`
//!
//! — the host instance of the full chain (`boyko_shaderdsl::ibl`, Eval) over the committed blob. The
//! host normal is the analytic sphere's at the pixel's camera ray (the engine's `generate_ray`), so
//! the comparison is statistical, not texel-exact: per sphere, over the pixels with `NoV ∈ [0.25,
//! 0.95]` (inside the silhouette on every geometry), the MEDIAN relative error must stay under
//! [`MEDIAN_REL`] and the 90th percentile under [`P90_REL`] (the residual is the mesh's tessellation
//! normal, 8-bit filtering weights, the B10G11R11 and f16 steps, and panel edges). The mutations
//! the gate exists for — a `MipMode::None` sampler (every roughness reads the mirror level), a
//! flipped yaw sign, a dropped dominant-direction factor — move the medians by far more.
//!
//! Each cell's receipt names its producer, so a cell that silently took another path is red.
//!
//! **`ibl_armed_no_env_matches_base_*`** (cut C11, ruling 124 (5)): the same worker with a
//! `DirectionalLight` + `SkyLight` and NO `EnvironmentLight`, booted twice — unarmed, and armed by
//! the measurement-only `BOYKO_IBL_ARM=studio` (the REFL pipelines, the +1 layouts and the chain all
//! exist, but the light fold writes today's `SkyLight` row, so every REFL producer takes its
//! hemisphere arm) — must dump the SAME `lit` bytes on Deferred × Mesh, Forward × Mesh and VB fused
//! (the paths of the `grand_showcase_2mat`, `forward_mesh` and `vb_mesh` pins). Each run asserts
//! the boot's `ResolvedEnvironment::armed`, so the armed leg cannot silently run unarmed. The
//! mutation it exists for: the SKY row's environment flag set without an `EnvironmentLight`
//! (`from_sky` writing `LIGHT_FLAG_ENV`) — the armed REFL producers then take the environment arm
//! and the bytes differ.
//!
//! Run: `cargo test -p boyko-app --test ibl_env_paths -- --ignored --test-threads=1` with
//! `BOYKO_DISABLE_VALIDATION=1` (and `--features hwrt` for the second leg).

#![cfg(windows)]

mod common;
#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::environment::{
    EnvSource, EnvironmentConfig, EnvironmentLight, ResolvedEnvironment, STUDIO_ENV_BLOB, parse_env_blob,
};
use boyko_render::light::LightingConfig;
use boyko_render::post::ResolvedPost;
use boyko_render::{GeometryLegs, Material, RenderPath, RenderPathConfig};
use boyko_shaderdsl::ibl::{env_specular_host, spec_dominant_dir_body};

use graph_receipt::{Receipt, fresh_out, run_worker};
use image_dump::{DumpFile, files_of};

/// The worker's cell knob.
const ENV_CELL: &str = "BOYKO_IBL1_CELL";
/// The worker's no-environment knob: a `DirectionalLight` + `SkyLight` instead of the
/// `EnvironmentLight`, and no `EnvironmentConfig` (so only `BOYKO_IBL_ARM` arms the boot).
const ENV_NOENV: &str = "BOYKO_IBL1_NOENV";
/// The arming the worker must observe after the run (`1` armed, `0` unarmed).
const ENV_WANT_ARMED: &str = "BOYKO_IBL1_WANT_ARMED";
/// The worker test the drivers spawn.
const WORKER: &str = "ibl_env_paths_worker";
/// Window edge.
const WIN: u32 = 256;
/// Frames before the capture.
const SETTLE: u32 = 30;
/// The environment's yaw — non-zero, so a flipped rotation sign is visible.
const YAW: f32 = 0.6;
/// The three spheres: centre x, roughness.
const SPHERES: [(f32, f32); 3] = [(-1.3, 0.05), (0.0, 0.3), (1.3, 0.6)];
/// The spheres' radius.
const RADIUS: f32 = 0.55;
/// The camera: eye, 60° vertical field of view, looking at the origin.
const EYE: [f32; 3] = [0.0, 0.0, 4.0];
/// The median relative error bound per sphere.
const MEDIAN_REL: f64 = 0.05;
/// The 90th-percentile relative error bound per sphere.
const P90_REL: f64 = 0.25;

/// One cell: a render path, its legs, the extra environment the worker gets, and the receipt keys
/// that prove the producer.
struct Cell {
    name: &'static str,
    path: RenderPath,
    legs: GeometryLegs,
    env: &'static [(&'static str, &'static str)],
    expect: &'static [(&'static str, u64)],
}

const CELLS: [Cell; 9] = [
    Cell { name: "deferred_mesh", path: RenderPath::Deferred, legs: GeometryLegs::Mesh, env: &[], expect: &[("path", 0), ("mesh_leg", 1), ("sdf_leg", 0)] },
    Cell { name: "deferred_sdf", path: RenderPath::Deferred, legs: GeometryLegs::Sdf, env: &[], expect: &[("path", 0), ("mesh_leg", 0), ("sdf_leg", 1)] },
    Cell { name: "forward_mesh", path: RenderPath::Forward, legs: GeometryLegs::Mesh, env: &[], expect: &[("path_is_forward", 1), ("path_is_forward_plus", 0)] },
    Cell { name: "forward_sdf", path: RenderPath::Forward, legs: GeometryLegs::Sdf, env: &[], expect: &[("path_is_forward", 1), ("path_has_sdf_forward", 1)] },
    Cell { name: "forwardplus_mesh", path: RenderPath::ForwardPlus, legs: GeometryLegs::Mesh, env: &[], expect: &[("path_is_forward_plus", 1)] },
    Cell { name: "vb_fused", path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh, env: &[], expect: &[("path_vb_fused", 1), ("vb_classified", 0)] },
    Cell { name: "vb_classified", path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh, env: &[("BOYKO_VB_FORCE_CLASSIFIED", "1")], expect: &[("vb_classified", 1)] },
    Cell { name: "vb_split", path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh, env: &[("BOYKO_SDF_MESH", "host")], expect: &[("path_vb_split", 1)] },
    Cell { name: "vb_sdf", path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Sdf, env: &[], expect: &[("path_is_vb", 1), ("path_has_sdf_forward", 1)] },
];

fn cell(name: &str) -> &'static Cell {
    CELLS.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{ENV_CELL}={name} names no cell"))
}

// ===============================================================================================
// The worker
// ===============================================================================================

/// The cell's legs, and whether the worker runs the no-environment scene.
#[derive(Resource)]
struct CellRes(GeometryLegs, bool);

#[allow(clippy::needless_pass_by_value)]
fn cell_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    legs: Res<CellRes>,
) {
    let mesh = matches!(legs.0, GeometryLegs::Mesh | GeometryLegs::Both);
    let (sv, si) = common::uv_sphere(RADIUS, 48, 64, [1.0, 1.0, 1.0, 1.0]);
    let sphere = mesh.then(|| meshes.register_mesh(dev.get(), &sv, &si));
    for (x, roughness) in SPHERES {
        let mat = materials.add(Material::new([1.0, 1.0, 1.0, 1.0], 1.0, roughness, 0.5, [0.0; 3], 0));
        let row = u16::try_from(mat.index()).expect("invariant: the cell mints < 65536 rows");
        match sphere {
            Some(s) => {
                commands.spawn(MeshBundle {
                    material: MaterialHandle(row),
                    ..MeshBundle::new(s, Transform::from_translation(Vec3::new(x, 0.0, 0.0)))
                });
            }
            None => {
                commands.spawn(SdfPrimitive(SdfEdit::sphere([x, 0.0, 0.0], RADIUS, sdf_op::UNION, 0.0).with_material(row)));
            }
        }
    }
    if legs.1 {
        let sun = [0.35f32, 0.8, 0.45];
        let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(sun[0], sun[1], sun[2]), Vec3::new(0.0, 1.0, 0.0));
        commands.spawn(DirectionalLightObject {
            transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
            global: GlobalTransform::IDENTITY,
            light: DirectionalLight::new(sun, [1.0, 0.97, 0.92], 3.1),
        });
        commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));
    } else {
        commands.spawn(EnvironmentLight::new(1.0, YAW));
    }
    let pose = Affine3A::look_at_rh(Vec3::new(EYE[0], EYE[1], EYE[2]), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// **THE WORKER** — one cell (`BOYKO_IBL1_CELL=<name>`), spawned by the drivers below.
#[test]
#[ignore = "gpu-windowed: the ibl_env_paths worker (BOYKO_IBL1_CELL set by its drivers); run through a driver with --test-threads=1"]
fn ibl_env_paths_worker() {
    let Ok(name) = std::env::var(ENV_CELL) else {
        eprintln!("{WORKER}: {ENV_CELL} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let c = cell(&name);
    let no_env = std::env::var(ENV_NOENV).is_ok_and(|v| v == "1");
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine ibl1 env paths", WIN, WIN));
    app.insert_resource(CellRes(c.legs, no_env));
    app.add_startup_system(cell_setup);
    app.insert_resource(RenderPathConfig { path: c.path, legs: c.legs });
    if !no_env {
        app.insert_resource(EnvironmentConfig { source: EnvSource::Studio });
    }
    app.run();
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran — RED, never a skip");
    if let Ok(want) = std::env::var(ENV_WANT_ARMED) {
        let armed = app.world().try_resource::<ResolvedEnvironment>().is_some_and(|r| r.armed);
        assert_eq!(armed, want == "1", "{name}: the boot's ResolvedEnvironment::armed is not the leg's arming");
    }
}

// ===============================================================================================
// The host chain
// ===============================================================================================

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The engine's perspective `generate_ray` (`ray_gen.hlsli`) for this camera.
fn ray(px: u32, py: u32) -> [f32; 3] {
    let tan_half = (core::f32::consts::FRAC_PI_3 * 0.5).tan();
    let ndc_x = ((px as f32 + 0.5) / WIN as f32) * 2.0 - 1.0;
    let ndc_y = -(((py as f32 + 0.5) / WIN as f32) * 2.0 - 1.0);
    normalize([ndc_x * tan_half, ndc_y * tan_half, -1.0])
}

/// The analytic hit of `rd` from [`EYE`] on sphere `s`: the unit normal.
fn hit_normal(rd: [f32; 3], s: usize) -> Option<[f32; 3]> {
    let c = [SPHERES[s].0, 0.0, 0.0];
    let oc = [EYE[0] - c[0], EYE[1] - c[1], EYE[2] - c[2]];
    let b = dot(oc, rd);
    let disc = b * b - (dot(oc, oc) - RADIUS * RADIUS);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    let p = [EYE[0] + rd[0] * t, EYE[1] + rd[1] * t, EYE[2] + rd[2] * t];
    Some(normalize([p[0] - c[0], p[1] - c[1], p[2] - c[2]]))
}

/// Karis' `env_brdf_approx` (the producers' split-sum fit, `deferred_pbr.hlsl`).
fn env_brdf_approx(roughness: f32, nov: f32) -> [f32; 2] {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r: [f32; 4] = core::array::from_fn(|i| roughness * c0[i] + c1[i]);
    let a004 = (r[0] * r[0]).min((-9.28 * nov).exp2()) * r[0] + r[1];
    [-1.04 * a004 + r[2], 1.04 * a004 + r[3]]
}

/// The host `lit` of a metal pixel (f0 = 1) with normal `n` seen along `rd`.
fn host_lit(chain: &[u32], tile: u32, levels: u32, n: [f32; 3], rd: [f32; 3], roughness: f32, pre_exposure: f32) -> [f32; 3] {
    let v = [-rd[0], -rd[1], -rd[2]];
    let nov = dot(n, v).max(1e-4);
    let r = {
        let d = 2.0 * dot(n, v);
        [d * n[0] - v[0], d * n[1] - v[1], d * n[2] - v[2]]
    };
    let pr = roughness.clamp(0.045, 1.0);
    let [a, b] = env_brdf_approx(pr, nov);
    let ess = (a + b).max(1e-4);
    let ec = 1.0 + (1.0 / ess - 1.0);
    let dir = spec_dominant_dir_body::<f32>(n, r, pr);
    let env = env_specular_host(chain, tile, levels, dir, pr, 1.0, YAW.cos(), YAW.sin());
    env.map(|e| (a + b) * e * ec * pre_exposure)
}

/// Runs `c` and checks every sphere against the host chain.
fn check_cell(c: &Cell) {
    let dir = fresh_out(&format!("ibl1_env_{}", c.name));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let receipt = dir.join("receipt.toml");
    let mut env = vec![
        (ENV_CELL, c.name.to_owned()),
        ("BOYKO_GRAPH_RECEIPT", receipt.display().to_string()),
        ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
        ("BOYKO_IMAGE_DUMP", "lit".to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
    ];
    for (k, v) in c.env {
        env.push((k, (*v).to_owned()));
    }
    run_worker(WORKER, &env, false);
    let r = Receipt::load(&receipt);
    for (key, want) in c.expect {
        assert_eq!(r.config(key), *want, "{}: receipt `{key}` — the cell did not take its producer", c.name);
    }
    let files = files_of(&dir, "lit");
    assert_eq!(files.len(), 1, "{}: one `lit` capture expected: {files:?}", c.name);
    let d = DumpFile::load(&files[0].1);
    assert_eq!((d.width, d.height), (WIN, WIN), "{}: `lit` is not at the window's extent", c.name);

    let asset = parse_env_blob(STUDIO_ENV_BLOB).expect("invariant: the committed blob parses");
    let chain: Vec<u32> = asset.chain.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
    let pre_exposure = ResolvedPost::from_config(&LightingConfig::default()).pre_exposure;
    let mut failures = Vec::new();
    for (s, &(_, roughness)) in SPHERES.iter().enumerate() {
        let mut rel = Vec::new();
        for py in 0..WIN {
            for px in 0..WIN {
                let rd = ray(px, py);
                let Some(n) = hit_normal(rd, s) else { continue };
                let nov = -dot(n, rd);
                if !(0.25..=0.95).contains(&nov) {
                    continue;
                }
                let want = host_lit(&chain, asset.header.tile, asset.header.levels, n, rd, roughness, pre_exposure);
                let got = d.texel_f32(px, py);
                let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                let (w, g) = (f64::from(lum(want)), f64::from(lum([got[0], got[1], got[2]])));
                rel.push((g - w).abs() / w.max(1e-3));
            }
        }
        assert!(rel.len() > 200, "{}: sphere {s} sampled only {} pixels — the framing moved", c.name, rel.len());
        rel.sort_by(f64::total_cmp);
        let median = rel[rel.len() / 2];
        let p90 = rel[rel.len() * 9 / 10];
        println!("{}: sphere {s} (roughness {roughness}): {} px, median rel {median:.4}, p90 {p90:.4}", c.name, rel.len());
        if median > MEDIAN_REL || p90 > P90_REL {
            failures.push(format!("sphere {s} (roughness {roughness}): median {median:.4} (<= {MEDIAN_REL}), p90 {p90:.4} (<= {P90_REL})"));
        }
    }
    assert!(failures.is_empty(), "{}: the environment arm is not the host chain:\n{}", c.name, failures.join("\n"));
}

macro_rules! driver {
    ($fn_name:ident, $cell:literal) => {
        #[test]
        #[ignore = "gpu-windowed: IBL1 R1's environment-path oracle for one lit producer (spawns ibl_env_paths_worker); run --ignored --test-threads=1"]
        fn $fn_name() {
            check_cell(cell($cell));
        }
    };
}

driver!(ibl_env_paths_deferred_mesh, "deferred_mesh");
driver!(ibl_env_paths_deferred_sdf, "deferred_sdf");
driver!(ibl_env_paths_forward_mesh, "forward_mesh");
driver!(ibl_env_paths_forward_sdf, "forward_sdf");
driver!(ibl_env_paths_forwardplus_mesh, "forwardplus_mesh");
driver!(ibl_env_paths_vb_fused, "vb_fused");
driver!(ibl_env_paths_vb_classified, "vb_classified");
driver!(ibl_env_paths_vb_split, "vb_split");
driver!(ibl_env_paths_vb_sdf, "vb_sdf");

/// One leg of [`check_armed_no_env`]: the no-environment scene booted with `armed`, its `lit` dump.
fn no_env_lit(c: &Cell, armed: bool) -> DumpFile {
    let tag = if armed { "armed" } else { "base" };
    let dir = fresh_out(&format!("ibl1_noenv_{}_{tag}", c.name));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let receipt = dir.join("receipt.toml");
    let mut env = vec![
        (ENV_CELL, c.name.to_owned()),
        (ENV_NOENV, "1".to_owned()),
        (ENV_WANT_ARMED, if armed { "1" } else { "0" }.to_owned()),
        ("BOYKO_GRAPH_RECEIPT", receipt.display().to_string()),
        ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
        ("BOYKO_IMAGE_DUMP", "lit".to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
    ];
    if armed {
        env.push(("BOYKO_IBL_ARM", "studio".to_owned()));
    }
    for (k, v) in c.env {
        env.push((k, (*v).to_owned()));
    }
    run_worker(WORKER, &env, false);
    let r = Receipt::load(&receipt);
    for (key, want) in c.expect {
        assert_eq!(r.config(key), *want, "{} ({tag}): receipt `{key}` — the cell did not take its producer", c.name);
    }
    let files = files_of(&dir, "lit");
    assert_eq!(files.len(), 1, "{} ({tag}): one `lit` capture expected: {files:?}", c.name);
    DumpFile::load(&files[0].1)
}

/// An armed boot with no `EnvironmentLight` is the base boot, byte for byte.
fn check_armed_no_env(c: &Cell) {
    let base = no_env_lit(c, false);
    let armed = no_env_lit(c, true);
    assert_eq!(
        (base.width, base.height, base.format),
        (armed.width, armed.height, armed.format),
        "{}: the two dumps' shapes",
        c.name
    );
    let lit_texels = base.texels.chunks(base.texel_bytes as usize).filter(|t| t.iter().any(|b| *b != 0)).count();
    assert!(lit_texels > 1000, "{}: anti-vacuity — the base dump lit only {lit_texels} texels", c.name);
    let differing = base.texels.iter().zip(&armed.texels).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "{}: the armed boot with no EnvironmentLight moved {differing} `lit` bytes", c.name);
}

macro_rules! armed_no_env {
    ($fn_name:ident, $cell:literal) => {
        #[test]
        #[ignore = "gpu-windowed: IBL1 R1's armed-without-EnvironmentLight byte identity (spawns ibl_env_paths_worker twice, BOYKO_IBL_ARM=studio); run --ignored --test-threads=1"]
        fn $fn_name() {
            check_armed_no_env(cell($cell));
        }
    };
}

armed_no_env!(ibl_armed_no_env_matches_base_deferred_mesh, "deferred_mesh");
armed_no_env!(ibl_armed_no_env_matches_base_forward_mesh, "forward_mesh");
armed_no_env!(ibl_armed_no_env_matches_base_vb_fused, "vb_fused");

/// The host chain itself (no device): the studio's 0.6-roughness metal facing straight back at the
/// camera sees the room's blurred +z wall — a finite, positive, non-trivial radiance — and a
/// MipMode::None-class read (the mirror level at every roughness) differs from it.
#[test]
fn host_chain_is_live() {
    let asset = parse_env_blob(STUDIO_ENV_BLOB).expect("invariant: the committed blob parses");
    let chain: Vec<u32> = asset.chain.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
    let (tile, levels) = (asset.header.tile, asset.header.levels);
    let n = normalize([0.3, 0.2, 1.0]);
    let rd = normalize([-0.3, -0.2, -1.0]);
    for (_, roughness) in SPHERES {
        let lit = host_lit(&chain, tile, levels, n, rd, roughness, 1.0);
        assert!(lit.iter().all(|c| c.is_finite() && *c > 0.0), "roughness {roughness}: {lit:?}");
    }
    let blurred = env_specular_host(&chain, tile, levels, [0.0, 0.0, 1.0], 0.6, 1.0, 1.0, 0.0);
    let mirror = env_specular_host(&chain, tile, levels, [0.0, 0.0, 1.0], 0.0, 1.0, 1.0, 0.0);
    assert_ne!(blurred, mirror, "the 0.6 and mirror levels differ along +z");
}
