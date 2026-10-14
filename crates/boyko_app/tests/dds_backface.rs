//! **RD6 / D-DS device gates — glTF `doubleSided` on the opaque rail** (render wave RD6, defect
//! D-DS, commit D-4; the cut's §3 D-4 list plus critique W5's per-family arms).
//!
//! glTF 2.0 "Double Sided": "When this value is false, back-face culling is enabled, i.e., only
//! front-facing triangles are rendered. When this value is true, back-face culling is disabled,
//! triangles are rendered from both sides, and the normal vectors of the back-facing triangles
//! are negated".
//!
//! Every gate boots the `common/dds_scene.rs` worker once per arm, dumps `lit` and the path's depth
//! image (`depth` / `forward_depth` / `vb_depth`) with `BOYKO_IMAGE_DUMP`, and compares arms pixel
//! for pixel over a host-computed footprint:
//!
//! 1. `dds_back_face_lit_*` — a double-sided card seen from its authored BACK is lit like its
//!    front (`|A − B| ≤ 1/255` per channel over the 2-px-eroded footprint, `A ≥ 32/255`), with
//!    the sun along the view direction and ambient + emission zeroed (rule xvi). Red before the
//!    fix: the back shades with its front normal, `N·L < 0`, dark.
//! 2. `dds_single_sided_back_culled_*` — a single-sided card seen from its back, a backdrop 2 units
//!    behind: the depth over the card's footprint equals the no-card run's (the card is culled).
//! 3. `dds_double_sided_caster_*` / `dds_double_sided_point_caster_*` — a horizontal double-sided
//!    card with its authored front AWAY from the light still casts on the floor (the lit ratio in
//!    the host-predicted umbra, over pixels where the floor is visible in both runs, ≤ 0.5); the
//!    single-sided controls keep HEAD's light-facing caster set (K1): toward the light casts,
//!    away does not — except the sun on a `hwrt` Deferred boot, whose traced shadow rays are
//!    facing-agnostic (DS-D8), so there the away card casts too.
//! 4. `dds_mirrored_cube_*` — the built-in cube at scale `(−1, 1, 1)` renders the pixels of the
//!    unmirrored one (its X mirror is the same geometric cube). Green before the fix (culling is
//!    off); the regression guard of a mirrored instance turned inside-out.
//! 5. `dds_mirrored_card_lit_*` — test 1's back arm on an X-mirrored instance (an IN-PLANE axis, so
//!    the card's normal is unchanged and only its winding reverses).
//! 6. `dds_forwardplus_prepass_parity` — ForwardPlus's depth prepass and its EQUAL colour pass cull
//!    the same faces: the image over a culled card equals the no-card run's.
//!
//! W5 (one failing gate per pipeline family): the TEX family (`dds_back_face_lit_deferred_textured`),
//! the ForwardPlus `Back` member (`dds_single_sided_back_culled_forwardplus`), the point-light
//! caster family (`dds_double_sided_point_caster_*`), and under `hwrt` the MV / MVPM families
//! (`dds_back_face_lit_deferred_denoised`: the temporal denoiser routes the camera raster through
//! them).
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_WINDOW_FRAMES` set (each driver also sets a per-worker backstop).

#[path = "common/dds_scene.rs"]
mod dds_scene;
#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_render::RenderPath;

use dds_scene::{
    DdsArming, ENV_BACKDROP, ENV_DS, ENV_EMISSIVE, ENV_FACING, ENV_LIGHT, ENV_MIRROR, ENV_NOCARD,
    ENV_PATH, ENV_SCENE, ENV_TEX, EMPTY_PIXEL, Rect, WIN, assert_dds_resolved, backdrop_band, build_dds_app,
    card_mask, shadow_rect,
};
use graph_receipt::{fresh_out, run_worker};
use image_dump::{DumpFile, VK_D32_SFLOAT, files_of};

/// The marker the drivers set.
const ENV_WORKER: &str = "BOYKO_DDS_WORKER";
/// The capture's settle window.
const SETTLE: u32 = 12;
/// One 8-bit code, in the linear `lit` units the gates compare.
const LSB: f32 = 1.0 / 255.0;

/// **The D-DS WORKER** — the `common/dds_scene.rs` scene the driver's env names.
#[test]
#[ignore = "gpu-windowed: the RD6 D-DS worker, spawned by the dds_* drivers; run with --test-threads=1"]
fn dds_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("dds_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = DdsArming::from_env();
    let mut app = build_dds_app("boyko_app RD6 D-DS worker", arming);
    app.run();
    assert_dds_resolved(&app, arming);
}

/// One arm's captures.
struct Capture {
    lit: DumpFile,
    depth: DumpFile,
}

/// Boots the worker with `env` on `path` and returns its `lit` and depth captures.
fn capture(path: RenderPath, env: &[(&str, &str)], tag: &str) -> Capture {
    let depth_name = DdsArming::depth_name(path);
    let dir = fresh_out(&format!("dds_{}_{tag}", DdsArming::path_word(path)));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let mut all: Vec<(&str, String)> = vec![
        (ENV_WORKER, "1".to_owned()),
        (ENV_PATH, DdsArming::path_word(path).to_owned()),
        ("BOYKO_IMAGE_DUMP", format!("lit,{depth_name}")),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 30).to_string()),
    ];
    all.extend(env.iter().map(|&(k, v)| (k, v.to_owned())));
    run_worker("dds_worker", &all, false);
    let load = |name: &str| {
        let files = files_of(&dir, name);
        assert_eq!(files.len(), 1, "one `{name}` capture expected ({tag}): {files:?}");
        let d = DumpFile::load(&files[0].1);
        assert_eq!((d.width, d.height), (WIN, WIN), "the `{name}` dump's extent ({tag})");
        d
    };
    let depth = load(depth_name);
    assert_eq!(depth.format, VK_D32_SFLOAT, "the depth dump is D32 ({tag})");
    Capture { lit: load("lit"), depth }
}

fn rgb(c: &Capture, x: u32, y: u32) -> [f32; 3] {
    let t = c.lit.texel_f32(x, y);
    [t[0], t[1], t[2]]
}

fn mean_rgb(c: &Capture, r: Rect) -> [f32; 3] {
    let mut s = [0.0f64; 3];
    let mut n = 0.0f64;
    for (x, y) in r.pixels() {
        let p = rgb(c, x, y);
        for k in 0..3 {
            s[k] += f64::from(p[k]);
        }
        n += 1.0;
    }
    s.map(|v| (v / n) as f32)
}

/// The largest per-channel difference between two captures over `r`, with its pixel.
fn max_diff(a: &Capture, b: &Capture, r: Rect) -> (f32, (u32, u32)) {
    let mut worst = (0.0f32, (r.x0, r.y0));
    for (x, y) in r.pixels() {
        let (pa, pb) = (rgb(a, x, y), rgb(b, x, y));
        for k in 0..3 {
            let d = (pa[k] - pb[k]).abs();
            if d > worst.0 {
                worst = (d, (x, y));
            }
        }
    }
    worst
}

// --- 1 / 5: a double-sided back face is lit like its front --------------------------------------

/// `A` = the double-sided card's front toward the camera; `B` = its back (optionally mirrored).
fn back_face_lit(path: RenderPath, extra: &[(&str, &str)], mirrored_b: bool) {
    let mask = card_mask();
    let base: Vec<(&str, &str)> = [(ENV_SCENE, "card"), (ENV_DS, "1")].into_iter().chain(extra.iter().copied()).collect();
    let a = capture(path, &base, "front");
    let mut b_env = base.clone();
    b_env.push((ENV_FACING, "back"));
    if mirrored_b {
        b_env.push((ENV_MIRROR, "1"));
    }
    let b = capture(path, &b_env, if mirrored_b { "back_mirrored" } else { "back" });

    // The "different red": a double-sided back face must be RASTERISED (its depth written).
    let clear = b.depth.depth(EMPTY_PIXEL.0, EMPTY_PIXEL.1);
    let undrawn = mask.pixels().filter(|&(x, y)| b.depth.depth(x, y).to_bits() == clear.to_bits()).count();
    assert_eq!(undrawn, 0, "DIFFERENT RED on {path:?}: {undrawn} px of the double-sided card's back are not drawn — a double-sided back face is culled (the cull table is wrong)");

    let (ma, mb) = (mean_rgb(&a, mask), mean_rgb(&b, mask));
    eprintln!("{path:?}: front mean {ma:?}, back{} mean {mb:?}", if mirrored_b { " (mirrored)" } else { "" });
    assert!(ma.iter().all(|&c| c >= 32.0 * LSB), "PREMISE on {path:?}: the lit front is not lit ({ma:?})");
    let (d, at) = max_diff(&a, &b, mask);
    assert!(
        d <= LSB,
        "RED on {path:?}: the double-sided card's back{} is not lit like its front — max |A-B| {d} at {at:?} \
         (front mean {ma:?}, back mean {mb:?}: the back shades with its un-negated normal)",
        if mirrored_b { " on a mirrored instance" } else { "" }
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_deferred() {
    back_face_lit(RenderPath::Deferred, &[], false);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_forward() {
    back_face_lit(RenderPath::Forward, &[], false);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on ForwardPlus (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_forwardplus() {
    back_face_lit(RenderPath::ForwardPlus, &[], false);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_vb() {
    back_face_lit(RenderPath::VisibilityBuffer, &[], false);
}

/// W5: the TEX family (a textured double-sided material — an albedo and a flat normal map, so the
/// TBN block runs before the final-normal flip).
#[test]
#[ignore = "gpu-windowed: RD6 test 1 on Deferred, textured (the gbuffer_mrt_tex family; spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_deferred_textured() {
    back_face_lit(RenderPath::Deferred, &[(ENV_TEX, "1")], false);
}

/// W5: the MV / MVPM families — the temporal shadow denoiser routes the Deferred camera raster
/// through the motion-vector pipelines (`hwrt` builds on a ray-query device).
#[cfg(feature = "hwrt")]
#[test]
#[ignore = "feature+gpu-windowed+gpu-cap: RD6 test 1 on Deferred under the temporal denoiser (the MV/MVPM families; needs ray query; spawns dds_worker); run with --features hwrt, --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_deferred_denoised() {
    back_face_lit(RenderPath::Deferred, &[(dds_scene::ENV_DENOISE, "1")], false);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 5 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_card_lit_deferred() {
    back_face_lit(RenderPath::Deferred, &[], true);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 5 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_card_lit_forward() {
    back_face_lit(RenderPath::Forward, &[], true);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 5 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_card_lit_vb() {
    back_face_lit(RenderPath::VisibilityBuffer, &[], true);
}

// --- 2 / 6: a single-sided back face is culled ---------------------------------------------------

/// The single-sided card's back toward the camera with a backdrop, against the no-card run.
fn single_sided_back_culled(path: RenderPath) {
    let mask = card_mask();
    let band = backdrop_band();
    let with = capture(path, &[(ENV_SCENE, "card"), (ENV_FACING, "back"), (ENV_BACKDROP, "1")], "card");
    let without = capture(path, &[(ENV_SCENE, "card"), (ENV_BACKDROP, "1"), (ENV_NOCARD, "1")], "nocard");

    let clear = with.depth.depth(EMPTY_PIXEL.0, EMPTY_PIXEL.1);
    let backdrop_missing = band.pixels().filter(|&(x, y)| with.depth.depth(x, y).to_bits() == clear.to_bits()).count();
    assert_eq!(backdrop_missing, 0, "DIFFERENT RED on {path:?}: {backdrop_missing} backdrop px are not drawn — FRONT faces are culled (the cull sign is wrong)");

    let differing = mask.pixels().filter(|&(x, y)| with.depth.depth(x, y).to_bits() != without.depth.depth(x, y).to_bits()).count();
    let total = mask.pixels().count();
    assert_eq!(
        differing, 0,
        "RED on {path:?}: {differing} of {total} px over the single-sided card's footprint hold the CARD's depth — \
         its back face was rasterised instead of culled"
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 2 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_deferred() {
    single_sided_back_culled(RenderPath::Deferred);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 2 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_forward() {
    single_sided_back_culled(RenderPath::Forward);
}

/// W5: the ForwardPlus `Back` member (the depth prepass) has its own gate.
#[test]
#[ignore = "gpu-windowed: RD6 test 2 on ForwardPlus (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_forwardplus() {
    single_sided_back_culled(RenderPath::ForwardPlus);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 2 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_vb() {
    single_sided_back_culled(RenderPath::VisibilityBuffer);
}

/// Test 6: ForwardPlus prepass parity — the image over a culled single-sided card (an EMISSIVE
/// one, so a drawn card is never confused with an undrawn pixel) equals the no-card run's: the
/// prepass and the EQUAL colour pass culled the same face. A prepass that keeps the face while the
/// colour pass culls it leaves undrawn pixels there.
#[test]
#[ignore = "gpu-windowed: RD6 test 6, ForwardPlus prepass parity (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_forwardplus_prepass_parity() {
    let path = RenderPath::ForwardPlus;
    let mask = card_mask();
    let with = capture(path, &[(ENV_SCENE, "card"), (ENV_FACING, "back"), (ENV_BACKDROP, "1"), (ENV_EMISSIVE, "1")], "card");
    let without = capture(path, &[(ENV_SCENE, "card"), (ENV_BACKDROP, "1"), (ENV_NOCARD, "1")], "nocard");
    let backdrop = mean_rgb(&without, mask);
    assert!(backdrop.iter().any(|&c| c >= 8.0 * LSB), "PREMISE: the backdrop behind the card is lit ({backdrop:?})");
    let (d, at) = max_diff(&with, &without, mask);
    assert!(
        d <= LSB,
        "RED: over the single-sided card's footprint ForwardPlus does not show the backdrop — max |with - without| \
         {d} at {at:?} (the card was drawn, or the prepass and the colour pass culled different faces)"
    );
}

// --- 3: casters cull by the flag -----------------------------------------------------------------

/// The lit ratio over the predicted umbra, restricted to pixels where the floor shows in both runs.
fn shadow_ratio(run: &Capture, reference: &Capture, rect: Rect) -> (f64, usize) {
    let luma = |p: [f32; 3]| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]);
    let (mut num, mut den, mut n) = (0.0f64, 0.0f64, 0usize);
    for (x, y) in rect.pixels() {
        if run.depth.depth(x, y).to_bits() != reference.depth.depth(x, y).to_bits() {
            continue; // the card itself covers this pixel
        }
        num += luma(rgb(run, x, y));
        den += luma(rgb(reference, x, y));
        n += 1;
    }
    (if den > 0.0 { num / den } else { f64::NAN }, n)
}

/// The double-sided card faces AWAY from the light and must cast; the single-sided controls keep
/// HEAD's caster set (K1).
fn double_sided_caster(path: RenderPath, light: &str) {
    let point = light == "point";
    let rect = shadow_rect(point);
    let caster = |extra: &[(&str, &str)], tag: &str| {
        let mut env = vec![(ENV_SCENE, "caster"), (ENV_LIGHT, light)];
        env.extend_from_slice(extra);
        capture(path, &env, tag)
    };
    let reference = caster(&[(ENV_NOCARD, "1")], &format!("{light}_nocard"));
    let ratio = |c: &Capture, what: &str| {
        let (r, n) = shadow_ratio(c, &reference, rect);
        assert!(n >= 40, "VACUOUS on {path:?} ({light}): only {n} floor px of the umbra {rect:?} are visible for {what}");
        eprintln!("{path:?} ({light}): {what}: ratio {r:.3} over {n} px");
        r
    };

    let ss_toward = caster(&[], &format!("{light}_ss_toward"));
    let r = ratio(&ss_toward, "single-sided, front toward the light");
    assert!(r <= 0.5, "CONTROL on {path:?} ({light}): a single-sided card facing the light must cast (ratio {r:.3})");
    let ss_away = caster(&[(ENV_FACING, "back")], &format!("{light}_ss_away"));
    let r = ratio(&ss_away, "single-sided, front away from the light");
    // DS-D8 (a stated narrowing): on a `hwrt` Deferred boot the sun's mesh shadow is also traced
    // (`deferred_pbr.hlsl`'s `FORCE_OPAQUE` ray query) and RT shadow rays treat every triangle as
    // two-sided, so there the away-facing single-sided card DOES occlude and K1's control inverts.
    // The raster caster members this test is about are then gated by the software leg.
    if cfg!(feature = "hwrt") && path == RenderPath::Deferred && !point {
        assert!(
            r <= 0.5,
            "CONTROL on {path:?} ({light}, hwrt): RT shadow rays are facing-agnostic (DS-D8), so a single-sided card \
             facing away must still cast here (ratio {r:.3})"
        );
    } else {
        assert!(
            r >= 0.95,
            "CONTROL on {path:?} ({light}): a single-sided card facing away must not cast (K1; ratio {r:.3})"
        );
    }
    let ds_away = caster(&[(ENV_FACING, "back"), (ENV_DS, "1")], &format!("{light}_ds_away"));
    let r = ratio(&ds_away, "double-sided, front away from the light");
    assert!(
        r <= 0.5,
        "RED on {path:?} ({light}): a double-sided card whose authored front faces away from the light casts no \
         shadow (ratio {r:.3}; the caster pass culled it as a single-sided face)"
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Deferred, sun (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_caster_deferred() {
    double_sided_caster(RenderPath::Deferred, "sun");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Forward, sun (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_caster_forward() {
    double_sided_caster(RenderPath::Forward, "sun");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on VisibilityBuffer, sun (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_caster_vb() {
    double_sided_caster(RenderPath::VisibilityBuffer, "sun");
}

/// W5: the point-light caster family (`point_depth_pipeline`), per path.
#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Deferred, point light (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_point_caster_deferred() {
    double_sided_caster(RenderPath::Deferred, "point");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Forward, point light (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_point_caster_forward() {
    double_sided_caster(RenderPath::Forward, "point");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on VisibilityBuffer, point light (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_point_caster_vb() {
    double_sided_caster(RenderPath::VisibilityBuffer, "point");
}

// --- 4: a mirrored instance keeps its facing ------------------------------------------------------

fn mirrored_cube(path: RenderPath) {
    let a = capture(path, &[(ENV_SCENE, "cube")], "cube");
    let b = capture(path, &[(ENV_SCENE, "cube"), (ENV_MIRROR, "1")], "cube_mirrored");
    let clear = a.depth.depth(EMPTY_PIXEL.0, EMPTY_PIXEL.1);
    let covered = |x: u32, y: u32| a.depth.depth(x, y).to_bits() != clear.to_bits();
    // The cube mask, eroded by 2 px (every pixel within 2 px is the cube's too).
    let mut n = 0;
    let mut worst = (0.0f32, (0, 0));
    for y in 2..WIN - 2 {
        for x in 2..WIN - 2 {
            if !(x - 2..=x + 2).all(|i| (y - 2..=y + 2).all(|j| covered(i, j))) {
                continue;
            }
            n += 1;
            let (pa, pb) = (rgb(&a, x, y), rgb(&b, x, y));
            for k in 0..3 {
                let d = (pa[k] - pb[k]).abs();
                if d > worst.0 {
                    worst = (d, (x, y));
                }
            }
        }
    }
    assert!(n >= 500, "VACUOUS on {path:?}: the cube covers only {n} eroded px");
    assert!(
        worst.0 <= LSB,
        "RED on {path:?}: the X-mirrored cube does not render the unmirrored cube's pixels — max |A-B| {} at {:?} \
         over {n} px (a mirrored instance turned inside-out: its cull or flip ignores the mirror)",
        worst.0,
        worst.1
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 4 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_deferred() {
    mirrored_cube(RenderPath::Deferred);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 4 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_forward() {
    mirrored_cube(RenderPath::Forward);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 4 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_vb() {
    mirrored_cube(RenderPath::VisibilityBuffer);
}

/// The fixture geometry the gates read, checked on every host: the card mask sits inside the
/// window, the backdrop band beside it, the predicted umbras inside the window.
#[test]
fn the_dds_windows_sit_where_the_gates_need_them() {
    let card = card_mask();
    assert!(card.x0 > 8 && card.x1 < WIN - 8 && card.y0 > 8 && card.y1 < WIN - 8, "card {card:?}");
    let band = backdrop_band();
    assert!(band.x1 + 4 < card.x0 && band.x0 > 0, "band {band:?} vs card {card:?}");
    for point in [false, true] {
        let s = shadow_rect(point);
        assert!(s.x0 > 4 && s.x1 < WIN - 4 && s.y0 > 4 && s.y1 < WIN - 4, "umbra {s:?} (point={point})");
        assert!(s.pixels().count() >= 80, "umbra {s:?} (point={point}) is too small to judge");
    }
}
