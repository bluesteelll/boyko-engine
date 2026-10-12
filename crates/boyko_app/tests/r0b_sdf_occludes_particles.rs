//! **RD1 G18 — an SDF surface hides a depth-tested transparent behind it, on every path**
//! (render wave RD1, defect R0b, device test 6).
//!
//! The scene (`common/r0b_scene.rs`): an SDF sphere on the camera's axis, an ALPHA particle 1 unit
//! behind its centre (the OCCLUDED probe, projecting inside the sphere's silhouette) and an
//! ADDITIVE particle in front of the background, off the sphere (the CONTROL). Particles do not
//! move and the clock is frame-locked, so two runs captured at the same frame render the same
//! bytes. The observer is OBS's `lit` image dump.
//!
//! Each driver boots [`g18_worker`] three times on its path and compares 7×7 windows:
//!
//! 1. armed, both emitters — the run under test;
//! 2. armed, NO emitter — the bare scene (the particle subsystem stays armed, so the merge and the
//!    draw are declared exactly as in run 1, with zero instances);
//! 3. both emitters under `BOYKO_TEST_FORCE=disarm_sdf_merge` — the red control.
//!
//! - the CONTROL window differs between 1 and 2 (the particles are drawn at all — otherwise
//!   "hidden" would be vacuous);
//! - the OCCLUDED window is byte-identical between 1 and 2 — hidden;
//! - the OCCLUDED window differs between 3 and 2 — the probe can see that particle when the
//!   merge is disarmed, so its absence in 1 is the merge's doing.
//!
//! Paths: Deferred, Forward, ForwardPlus and VisibilityBuffer with both legs, and
//! VisibilityBuffer with the SDF leg alone, where the merge is also the first writer of `vb_depth`
//! (before RD1 that frame read the transient unwritten: a dev-profile panic).
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
use image_dump::{DumpFile, VK_R8G8B8A8_UNORM, files_of};
use r0b_scene::{
    ENV_EMIT, ENV_LEGS, ENV_PATH, ENV_TEST_FORCE, R0bArming, Rect, WIN, assert_r0b_resolved,
    build_r0b_app, control_window, occluded_window,
};

/// The marker the drivers set.
const ENV_WORKER: &str = "BOYKO_R0B_G18_WORKER";
/// The capture's settle window.
const SETTLE: u32 = 20;

/// The fixture's geometry, checked on every host: the OCCLUDED window lies inside the sphere's
/// silhouette and inside the occluded billboard, the CONTROL window inside its billboard and
/// outside the sphere — so "differs" and "identical" below are statements about the pixels the
/// module doc says they are.
#[test]
fn the_probe_windows_sit_where_the_gate_needs_them() {
    use r0b_scene::{BILLBOARD, CONTROL_POS, EYE, OCCLUDED_POS, SPHERE_C, SPHERE_R, project, px_radius};
    let centre = project(boyko_app::prelude::Vec3::new(SPHERE_C[0], SPHERE_C[1], SPHERE_C[2]), WIN);
    let sphere_px = px_radius(SPHERE_R, EYE.z - SPHERE_C[2], WIN);
    let dist = |a: (u32, u32), b: (u32, u32)| {
        ((f64::from(a.0) - f64::from(b.0)).powi(2) + (f64::from(a.1) - f64::from(b.1)).powi(2)).sqrt() as f32
    };
    let occ = occluded_window(WIN);
    let occ_billboard = px_radius(BILLBOARD, EYE.z - OCCLUDED_POS.z, WIN);
    for p in occ.pixels() {
        assert!(dist(p, centre) + 1.5 < sphere_px, "occluded probe pixel {p:?} is not inside the sphere's silhouette");
        assert!(dist(p, project(OCCLUDED_POS, WIN)) + 1.0 < occ_billboard, "occluded probe pixel {p:?} is off its billboard");
    }
    let ctl = control_window(WIN);
    let ctl_billboard = px_radius(BILLBOARD, EYE.z - CONTROL_POS.z, WIN);
    for p in ctl.pixels() {
        assert!(dist(p, centre) > sphere_px + 2.0, "control probe pixel {p:?} overlaps the sphere");
        assert!(dist(p, project(CONTROL_POS, WIN)) + 1.0 < ctl_billboard, "control probe pixel {p:?} is off its billboard");
    }
}

/// **The G18 WORKER** — the R0b scene on the path / legs / emitters / force key its driver set.
#[test]
#[ignore = "gpu-windowed: the RD1 G18 worker, spawned by the g18_* drivers; run with --test-threads=1"]
fn g18_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("g18_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = R0bArming::from_env();
    let mut app = build_r0b_app("boyko_app RD1 G18 worker", arming, WIN, WIN);
    app.run();
    assert_r0b_resolved(&app, arming);
}

/// Boots the worker and returns its `lit` capture.
fn lit_of(path: RenderPath, legs: GeometryLegs, emit: &str, force: &str, tag: &str) -> DumpFile {
    let dir = fresh_out(&format!(
        "r0b_g18_{}_{}_{tag}",
        R0bArming::path_word(path),
        R0bArming::legs_word(legs)
    ));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    run_worker(
        "g18_worker",
        &[
            (ENV_WORKER, "1".to_owned()),
            (ENV_PATH, R0bArming::path_word(path).to_owned()),
            (ENV_LEGS, R0bArming::legs_word(legs).to_owned()),
            (ENV_EMIT, emit.to_owned()),
            (ENV_TEST_FORCE, force.to_owned()),
            ("BOYKO_IMAGE_DUMP", "lit".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
        ],
        false,
    );
    let files = files_of(&dir, "lit");
    assert_eq!(files.len(), 1, "one `lit` capture expected ({tag}): {files:?}");
    let d = DumpFile::load(&files[0].1);
    assert_eq!((d.format, d.width, d.height), (VK_R8G8B8A8_UNORM, WIN, WIN), "the `lit` dump's header ({tag})");
    d
}

/// How many pixels of `rect` differ between `a` and `b`.
fn differing(a: &DumpFile, b: &DumpFile, rect: Rect) -> usize {
    rect.pixels().filter(|&(x, y)| a.rgba8(x, y) != b.rgba8(x, y)).count()
}

/// G18 on one path / leg set (the module doc's three runs and three clauses).
fn g18(path: RenderPath, legs: GeometryLegs) {
    let occluded = occluded_window(WIN);
    let control = control_window(WIN);
    let armed = lit_of(path, legs, "both", "", "armed");
    let bare = lit_of(path, legs, "none", "", "bare");

    let control_drawn = differing(&armed, &bare, control);
    eprintln!("G18 {path:?} x {legs:?}: control window {control:?} differs in {control_drawn} px");
    assert!(
        control_drawn > 0,
        "VACUOUS FIXTURE on {path:?} x {legs:?}: the CONTROL particle changed no pixel of {control:?} \
         — the particles are not drawn at all, so 'hidden' below would prove nothing"
    );

    let shown = differing(&armed, &bare, occluded);
    eprintln!("G18 {path:?} x {legs:?}: occluded window {occluded:?} differs in {shown} px (want 0)");
    assert_eq!(
        shown, 0,
        "R0b on {path:?} x {legs:?}: the OCCLUDED particle 1 unit behind the SDF sphere shows through \
         it — {shown} px of {occluded:?} differ from the bare scene"
    );

    let disarmed = lit_of(path, legs, "both", "disarm_sdf_merge", "disarmed");
    let shown_disarmed = differing(&disarmed, &bare, occluded);
    eprintln!("G18 {path:?} x {legs:?}: under disarm_sdf_merge the occluded window differs in {shown_disarmed} px");
    assert!(
        shown_disarmed > 0,
        "RED CONTROL DEAD on {path:?} x {legs:?}: with the merge disarmed the occluded particle is \
         still invisible, so the probe window cannot see it — 'hidden' above proves nothing"
    );
}

#[test]
#[ignore = "gpu-windowed: RD1 G18 on Deferred x Both (spawns g18_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn g18_deferred() {
    g18(RenderPath::Deferred, GeometryLegs::Both);
}

#[test]
#[ignore = "gpu-windowed: RD1 G18 on Forward x Both (spawns g18_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn g18_forward() {
    g18(RenderPath::Forward, GeometryLegs::Both);
}

#[test]
#[ignore = "gpu-windowed: RD1 G18 on ForwardPlus x Both (spawns g18_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn g18_forwardplus() {
    g18(RenderPath::ForwardPlus, GeometryLegs::Both);
}

#[test]
#[ignore = "gpu-windowed: RD1 G18 on VisibilityBuffer x Both (spawns g18_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn g18_vb() {
    g18(RenderPath::VisibilityBuffer, GeometryLegs::Both);
}

#[test]
#[ignore = "gpu-windowed: RD1 G18 on VisibilityBuffer x Sdf, the merge as vb_depth's first writer (spawns g18_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn g18_vb_sdf() {
    g18(RenderPath::VisibilityBuffer, GeometryLegs::Sdf);
}
