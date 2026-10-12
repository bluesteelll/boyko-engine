//! **RD1 G18-mesh-depth (D) — the merge never moves a mesh-won depth texel**
//! (render wave RD1, defect R0b, device test 7).
//!
//! The scene (`common/r0b_scene.rs` with its mesh cube): a mesh cube in FRONT of the SDF sphere,
//! covering its centre; the particle subsystem armed with no emitter (so the merge and the draw are
//! declared, with zero instances). Each driver dumps the path's depth image at frame end — OBS's
//! `depth` (Deferred), `forward_depth` (Forward), `vb_depth` (VisibilityBuffer) — on a merge run and
//! on a `BOYKO_TEST_FORCE=disarm_sdf_merge` run, and requires:
//!
//! - **bit-identical over the whole mesh-won rectangle** — at a pixel the mesh won, the composite
//!   `t` IS the decoded mesh `t`, so the biased encode lands behind the stored depth and the test
//!   rejects the write (the discriminator: a merge without the bias, or with it inverted, moves
//!   these texels);
//! - **different over every pixel of an SDF-won rectangle** (the anti-vacuity half: the merge did
//!   write where the sphere owns the pixel).
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_WINDOW_FRAMES` set (each driver also sets a per-worker backstop).

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;
#[path = "common/r0b_scene.rs"]
mod r0b_scene;

use boyko_render::{GeometryLegs, RenderPath};

use graph_receipt::{fresh_out, run_worker};
use image_dump::{DumpFile, VK_D32_SFLOAT, files_of};
use r0b_scene::{
    ENV_EMIT, ENV_LEGS, ENV_MESH, ENV_PATH, ENV_TEST_FORCE, R0bArming, WIN, assert_r0b_resolved,
    build_r0b_app, mesh_won_rect, sdf_won_rect,
};

/// The marker the drivers set.
const ENV_WORKER: &str = "BOYKO_R0B_MESH_DEPTH_WORKER";
/// The capture's settle window.
const SETTLE: u32 = 20;

/// The fixture's geometry, checked on every host: the mesh-won rectangle lies inside the cube's
/// front face (itself inside the sphere's silhouette), the SDF-won one inside the sphere and
/// outside the cube.
#[test]
fn the_rectangles_sit_where_the_gate_needs_them() {
    use r0b_scene::{CUBE_EDGE, CUBE_POS, EYE, SPHERE_C, SPHERE_R, project, px_radius};
    let centre = project(boyko_app::prelude::Vec3::new(SPHERE_C[0], SPHERE_C[1], SPHERE_C[2]), WIN);
    let sphere_px = px_radius(SPHERE_R, EYE.z - SPHERE_C[2], WIN);
    let cube_px = px_radius(CUBE_EDGE * 0.5, EYE.z - (CUBE_POS.z + CUBE_EDGE * 0.5), WIN);
    let off = |p: (u32, u32)| {
        ((f64::from(p.0) - f64::from(centre.0)).abs() as f32, (f64::from(p.1) - f64::from(centre.1)).abs() as f32)
    };
    for p in mesh_won_rect(WIN).pixels() {
        let (dx, dy) = off(p);
        assert!(dx + 1.0 < cube_px && dy + 1.0 < cube_px, "mesh-won pixel {p:?} is not on the cube's front face");
    }
    for p in sdf_won_rect(WIN).pixels() {
        let (dx, dy) = off(p);
        assert!(dx > cube_px + 1.0, "SDF-won pixel {p:?} touches the cube");
        assert!((dx * dx + dy * dy).sqrt() + 1.5 < sphere_px, "SDF-won pixel {p:?} is not inside the sphere");
    }
}

/// **The mesh-depth WORKER** — the R0b scene with the mesh cube, on the path its driver set.
#[test]
#[ignore = "gpu-windowed: the RD1 mesh-depth worker, spawned by the mesh_depth_* drivers; run with --test-threads=1"]
fn mesh_depth_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("mesh_depth_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = R0bArming::from_env();
    let mut app = build_r0b_app("boyko_app RD1 mesh-depth worker", arming, WIN, WIN);
    app.run();
    assert_r0b_resolved(&app, arming);
}

/// Boots the worker on `path` and returns its `name` depth capture.
fn depth_of(path: RenderPath, name: &str, force: &str, tag: &str) -> DumpFile {
    let dir = fresh_out(&format!("r0b_mesh_depth_{}_{tag}", R0bArming::path_word(path)));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    run_worker(
        "mesh_depth_worker",
        &[
            (ENV_WORKER, "1".to_owned()),
            (ENV_PATH, R0bArming::path_word(path).to_owned()),
            (ENV_LEGS, R0bArming::legs_word(GeometryLegs::Both).to_owned()),
            (ENV_EMIT, "none".to_owned()),
            (ENV_MESH, "1".to_owned()),
            (ENV_TEST_FORCE, force.to_owned()),
            ("BOYKO_IMAGE_DUMP", name.to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
        ],
        false,
    );
    let files = files_of(&dir, name);
    assert_eq!(files.len(), 1, "one `{name}` capture expected ({tag}): {files:?}");
    let d = DumpFile::load(&files[0].1);
    assert_eq!((d.format, d.width, d.height), (VK_D32_SFLOAT, WIN, WIN), "the `{name}` dump's header ({tag})");
    d
}

fn mesh_depth(path: RenderPath, name: &str) {
    // The disarmed control first: before RD1 the fixture refuses the key, which is this gate's red.
    let disarmed = depth_of(path, name, "disarm_sdf_merge", "disarmed");
    let merged = depth_of(path, name, "", "merged");

    let mesh = mesh_won_rect(WIN);
    let moved: Vec<(u32, u32, f32, f32)> = mesh
        .pixels()
        .filter(|&(x, y)| merged.depth(x, y).to_bits() != disarmed.depth(x, y).to_bits())
        .map(|(x, y)| (x, y, disarmed.depth(x, y), merged.depth(x, y)))
        .collect();
    eprintln!("{path:?} `{name}`: mesh-won {mesh:?}: {} px moved (want 0)", moved.len());
    assert!(
        moved.is_empty(),
        "R0b DISCRIMINATOR on {path:?}: the merge moved {} mesh-won `{name}` texel(s) of {mesh:?} \
         (x, y, disarmed, merged): {:?}",
        moved.len(),
        &moved[..moved.len().min(8)]
    );

    let sdf = sdf_won_rect(WIN);
    let total = sdf.pixels().count();
    let written = sdf.pixels().filter(|&(x, y)| merged.depth(x, y).to_bits() != disarmed.depth(x, y).to_bits()).count();
    eprintln!("{path:?} `{name}`: SDF-won {sdf:?}: {written} of {total} px written by the merge");
    assert_eq!(
        written, total,
        "VACUOUS on {path:?}: the merge left {} of {total} SDF-won `{name}` texel(s) of {sdf:?} untouched",
        total - written
    );
}

#[test]
#[ignore = "gpu-windowed: RD1 mesh-depth on Deferred (spawns mesh_depth_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn mesh_depth_deferred() {
    mesh_depth(RenderPath::Deferred, "depth");
}

#[test]
#[ignore = "gpu-windowed: RD1 mesh-depth on Forward (spawns mesh_depth_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn mesh_depth_forward() {
    mesh_depth(RenderPath::Forward, "forward_depth");
}

#[test]
#[ignore = "gpu-windowed: RD1 mesh-depth on VisibilityBuffer (spawns mesh_depth_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn mesh_depth_vb() {
    mesh_depth(RenderPath::VisibilityBuffer, "vb_depth");
}
