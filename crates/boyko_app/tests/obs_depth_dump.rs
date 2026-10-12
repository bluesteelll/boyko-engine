//! **OBS (D) — depth dumps against closed forms** (render-wave plan H0 Scope 6, device test 4).
//!
//! [`obs_depth_worker`] boots one cube of edge 1 at the origin, its front face toward a camera on
//! `+z` at distance 5, mesh leg only, no AA (no jitter). The drivers dump the path's depth and
//! read the centre texel, whose expected value is a closed form of the scene:
//!
//! - **Deferred `depth`** is the Euclidean ray length over 64 (`gbuffer_mrt.fs.hlsl`'s
//!   `MESH_DEPTH_T_MAX` encode): `t / 64`, `t` the ray length to the face through that texel.
//! - **VB `vb_depth`** is hardware reverse-Z: `d = B / z + A` for the face's view depth `z = 4.5`
//!   (`boyko_render::view::forward_view_z_from_depth` inverted).
//!
//! Tolerance: `1e-5` relative — D32 rounding and the rasterizer's interpolation are far below it,
//! and a wrong decode (a scale, the other encode, a neighbouring texel off the face) is far above.
//!
//! The non-ignored test checks the closed forms and the D32 decode on a synthetic file.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_WINDOW_FRAMES` set.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_render::{RenderPath, ResolvedRenderPath};

use graph_receipt::{ENV_PATH, ObsArming, fresh_out, run_worker};
use image_dump::{DEPTH_CAM_Z, DEPTH_CUBE, DEPTH_WIN, DumpFile, build_depth_app, centre_ray_t, encode, files_of, reverse_z};

/// The marker the drivers set.
const ENV_WORKER: &str = "BOYKO_OBS_DEPTH_WORKER";
/// The capture's settle window.
const SETTLE: u32 = 10;
/// The relative tolerance of a depth comparison.
const REL_TOL: f32 = 1e-5;

fn close(got: f32, want: f32) -> bool {
    (got - want).abs() <= REL_TOL * want.abs()
}

#[test]
fn depth_closed_forms_and_the_d32_decode() {
    let c = DEPTH_WIN / 2;
    let t = centre_ray_t(c, c);
    let face = DEPTH_CAM_Z - DEPTH_CUBE * 0.5;
    assert!(t > face && t < face * 1.000_01, "the centre texel's ray is just off-axis: t = {t}");
    // reverse_z inverts forward_view_z_from_depth: z = B / (d - A).
    let (near, far) = (image_dump::DEPTH_NEAR, image_dump::DEPTH_FAR);
    let d = reverse_z(face);
    let back = boyko_render::view::forward_view_z_from_depth(d, near, far);
    assert!(close(back, face), "reverse_z({face}) = {d} decodes back to {back}");
    // The D32 decode of a stored t / 64.
    let f = DumpFile::parse(&encode(image_dump::VK_D32_SFLOAT, 1, 1, 3, 4, &(t / 64.0).to_le_bytes())).expect("d32");
    assert!(close(f.depth(0, 0), t / 64.0));
}

/// **The depth WORKER** — the cube scene on the path `BOYKO_OBS_PATH` names.
#[test]
#[ignore = "gpu-windowed: the OBS depth-dump worker, spawned by the obs_depth_dump_* drivers; run with --test-threads=1"]
fn obs_depth_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("obs_depth_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = ObsArming::from_env();
    let mut app = build_depth_app("boyko_app OBS depth worker", arming.path);
    app.run();
    let resolved = *app.world().resource::<ResolvedRenderPath>();
    assert_eq!(resolved.path, arming.path, "PREMISE: asked for {:?}, the boot resolved {:?}", arming.path, resolved.path);
}

/// Dumps `name` on `path` and returns the centre texel's depth.
fn centre_depth(path: RenderPath, name: &str) -> f32 {
    let dir = fresh_out(&format!("depth_{name}"));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    run_worker(
        "obs_depth_worker",
        &[
            (ENV_WORKER, "1".to_owned()),
            (ENV_PATH, ObsArming::path_word(path).to_owned()),
            ("BOYKO_IMAGE_DUMP", name.to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
        ],
        false,
    );
    let files = files_of(&dir, name);
    assert_eq!(files.len(), 1, "one `{name}` capture expected: {files:?}");
    let d = DumpFile::load(&files[0].1);
    assert_eq!((d.format, d.width, d.height), (image_dump::VK_D32_SFLOAT, DEPTH_WIN, DEPTH_WIN), "the dump's header");
    let c = DEPTH_WIN / 2;
    d.depth(c, c)
}

#[test]
#[ignore = "gpu-windowed: OBS device test 4 on Deferred (obs_depth_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_depth_dump_deferred() {
    let c = DEPTH_WIN / 2;
    let want = centre_ray_t(c, c) / 64.0;
    let got = centre_depth(RenderPath::Deferred, "depth");
    eprintln!("Deferred depth at ({c}, {c}): {got} (t/64 = {want})");
    assert!(close(got, want), "Deferred depth {got} != t/64 = {want} (relative tolerance {REL_TOL})");
}

#[test]
#[ignore = "gpu-windowed: OBS device test 4 on VisibilityBuffer (obs_depth_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_depth_dump_vb() {
    let c = DEPTH_WIN / 2;
    let want = reverse_z(DEPTH_CAM_Z - DEPTH_CUBE * 0.5);
    let got = centre_depth(RenderPath::VisibilityBuffer, "vb_depth");
    eprintln!("VB vb_depth at ({c}, {c}): {got} (reverse-Z = {want})");
    assert!(close(got, want), "VB depth {got} != reverse-Z {want} (relative tolerance {REL_TOL})");
}
