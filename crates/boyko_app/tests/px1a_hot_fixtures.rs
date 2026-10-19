//! Render wave PX1a tests 11 (here from C4), 11b and 15 (the switch commit) and 12 (the NaN hook)
//! — `plan/H4-PX1a-hdr-lit.md` Tests 11, 11b, 12, 15; `px1a/cut.md` C4, C7, C11: **`lit` holds
//! scene-linear radiance above 1.0, and the display shoulders it.**
//!
//! **11 — the 50× sun.** A glossy SDF sphere under a sun at 50 × the reference illuminance (no
//! SkyLight, so the background is the dark constant). The sun core is the brightest `lit` texel
//! inside the sphere's region. There:
//! - (D) `lit` (`lit_native` under SSAA) holds radiance **> 1.0** — at HEAD, whose `lit` is
//!   display-referred RGBA8, it is ≤ 1.0 by construction;
//! - (D) `display` is ≥ 254/255 in every channel;
//! - along the ray from the core to the right, `display`'s luminance is a monotone shoulder (each
//!   step at most 1 LSB above the previous one) until it leaves the highlight.
//!
//! **11b — every producer stores above 1.0** (from the switch commit, C7). Each of PX0's HOT branch
//! fixtures (`common/hdr_fixtures.rs`: one per `lit`-store producer — `deferred_pbr` surface and
//! sky, `forward_opaque` (+ `_froxel`), `forward_sky`, `vb_sky`, `sdf_forward_march` on Forward and
//! VB, `vb_resolve`, `vb_shade`, `vb_shade_split` — and the SSAA box): the fixture's receipt keys
//! select its producer, and (D) `lit` (`lit_native` under SSAA) over the fixture's region holds
//! `max > 1.0` and equals the region's representable radiance (`HOT_R = 2.0`, exact in binary16)
//! within one format step in every texel. A clamp left at ANY one store site reds exactly that
//! producer's driver (mutation leg C7 (i): `vb_shade_split.comp.hlsl`'s clamp).
//!
//! **15 — particle E1** (C7). The particle lab scene (`particle_scene`, VB, the `particle_additive`
//! pin's configuration) at exposure 1.0 and 0.5, plus the particle-off run at 1.0 that masks the
//! particle pixels: over every pixel a particle reaches, (D) `lit` at exposure 0.5 equals half of
//! `lit` at exposure 1.0 within one binary16 step per channel (a power-of-two scale commutes with
//! every rounding on the way, so the halving is exact up to the blend's measured one-step error,
//! `goldens::RGBA16F_ROUNDING`). Red without the FS's `× pre_exposure` (mutation leg C7 (iv)): the
//! particle's own radiance does not halve.
//!
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/hdr_fixtures.rs"]
mod hdr_fixtures;
#[path = "common/image_dump.rs"]
mod image_dump;
#[path = "particle_scene/mod.rs"]
mod particle_scene;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig};
use boyko_rhi_vulkan::present::ImageDumpName;

use boyko_shaderdsl::pixfmt::{Channel, steps_between};
use graph_receipt::{fresh_out, run_worker};
use hdr_fixtures::{Expect, HOT_R, Hot, capture_scene, fixture, fixture_worker_main};
use image_dump::{DumpFile, files_of};

/// The worker's knob: `sun` | `sun_ssaa`.
const ENV_HOT: &str = "BOYKO_PX1A_HOT";
/// The window edge.
const WIN: u32 = 256;
/// Frames before the capture.
const SETTLE: u32 = 30;
/// The sun's illuminance: 50 × the OBS scenes' 2.8 lux.
const SUN_LUX: f32 = 2.8 * 50.0;
/// The sun's direction TO the light: up, right and toward the camera, so the highlight faces it.
const SUN_DIR: [f32; 3] = [0.35, 0.55, 1.0];
/// The steps of the shoulder walk.
const SHOULDER_STEPS: u32 = 24;

#[derive(Resource)]
struct HotRes(&'static str);

#[allow(clippy::needless_pass_by_value)]
fn sun_setup(mut commands: Commands, mut materials: ResMut<Assets<Material>>, hot: Res<HotRes>) {
    let _ = hot.0;
    let glossy = materials.add(Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.15, 0.5, [0.0; 3], 0));
    let row = u16::try_from(glossy.index()).expect("invariant: < 65536 rows");
    commands.spawn(SdfPrimitive(SdfEdit::sphere([0.0, 0.0, 0.0], 1.0, sdf_op::UNION, 0.0).with_material(row)));
    let pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], SUN_LUX),
    });
    let cam = Affine3A::look_at_rh(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: cam.translation, rotation: Quat::from_mat3(cam.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// **THE WORKER** — `BOYKO_PX1A_HOT=sun|sun_ssaa`.
#[test]
#[ignore = "gpu-windowed: the PX1a hot-fixture worker, spawned by the px1a_hot_fixtures drivers; run with --test-threads=1"]
fn px1a_hot_worker() {
    let Ok(which) = std::env::var(ENV_HOT) else {
        eprintln!("px1a_hot_worker: {ENV_HOT} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let which: &'static str = match which.trim() {
        "sun" => "sun",
        "sun_ssaa" => "sun_ssaa",
        other => panic!("{ENV_HOT}={other} is not sun|sun_ssaa"),
    };
    let ssaa = which == "sun_ssaa";
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine px1a hot fixture", WIN, WIN);
    app.add_plugins(if ssaa { plugins.with_ssaa_scale(2) } else { plugins });
    app.insert_resource(HotRes(which));
    app.add_startup_system(sun_setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Sdf });
    if !ssaa {
        app.insert_resource(AaConfig { mode: AaMode::Off });
    }
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

fn luma(t: [f32; 4]) -> f32 {
    0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]
}

/// The sun core: the brightest texel of `scene` (the radiance image) — the sphere fills the centre.
fn sun_core(scene: &DumpFile) -> (u32, u32) {
    let mut best = (0, 0);
    let mut best_y = f32::MIN;
    for y in 0..scene.height {
        for x in 0..scene.width {
            let l = luma(scene.texel_f32(x, y));
            if l > best_y {
                best_y = l;
                best = (x, y);
            }
        }
    }
    best
}

/// Test 11's display half: `display` at the core is ≥ 254 in every channel, and the walk to the
/// right is a monotone shoulder (each step ≤ 1 LSB above the previous). `Err` names the failure.
fn check_display_shoulder(display: &[[u8; 3]], width: u32, core: (u32, u32)) -> Result<(), String> {
    let at = |x: u32| display[(core.1 * width + x) as usize];
    let c = at(core.0);
    if c.iter().any(|&v| v < 254) {
        return Err(format!("display at the sun core {core:?} is {c:?}, not >= 254/255"));
    }
    let lum = |p: [u8; 3]| (2126 * u32::from(p[0]) + 7152 * u32::from(p[1]) + 722 * u32::from(p[2])) / 10000;
    let mut prev = lum(c);
    for step in 1..=SHOULDER_STEPS {
        let x = core.0 + step;
        if x >= width {
            break;
        }
        let v = lum(at(x));
        if v > prev + 1 {
            return Err(format!("display rises from {prev} to {v} at {step} px right of the core — no monotone shoulder"));
        }
        prev = v;
    }
    Ok(())
}

fn drive_sun(ssaa: bool) {
    let scene_name = if ssaa { "lit_native" } else { "lit" };
    assert!(ImageDumpName::parse(scene_name).is_some(), "no image-dump name `{scene_name}`");
    let display_known = ImageDumpName::parse("display").is_some();
    let names = if display_known { format!("{scene_name},display") } else { scene_name.to_owned() };
    let tag = if ssaa { "px1a_hot_sun_ssaa" } else { "px1a_hot_sun" };
    let dir = fresh_out(tag);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    run_worker(
        "px1a_hot_worker",
        &[
            (ENV_HOT, if ssaa { "sun_ssaa" } else { "sun" }.to_owned()),
            ("BOYKO_IMAGE_DUMP", names),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 10).to_string()),
        ],
        false,
    );
    let scene_files = files_of(&dir, scene_name);
    assert_eq!(scene_files.len(), 1, "one `{scene_name}` capture expected: {scene_files:?}");
    let scene = DumpFile::load(&scene_files[0].1);
    let core = sun_core(&scene);
    let t = scene.texel_f32(core.0, core.1);
    let max = t[0].max(t[1]).max(t[2]);
    assert!(max > 1.0, "(D) {scene_name} max <= 1.0 at the sun core {core:?}: {t:?} — the store is display-referred");
    assert!(display_known, "no image-dump name `display`");
    let disp_files = files_of(&dir, "display");
    assert_eq!(disp_files.len(), 1, "one `display` capture expected: {disp_files:?}");
    let d = DumpFile::load(&disp_files[0].1);
    // Under SSAA `lit_native` and `display` share the native extent; otherwise `lit` and `display` do.
    assert_eq!((d.width, d.height), (scene.width, scene.height), "display and the scene image differ in extent");
    let px: Vec<[u8; 3]> = (0..d.height)
        .flat_map(|y| (0..d.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = d.rgba8(x, y);
            [p[0], p[1], p[2]]
        })
        .collect();
    if let Err(e) = check_display_shoulder(&px, d.width, core) {
        panic!("{tag}: {e}");
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11, the 50x sun on Deferred (spawns px1a_hot_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_sun_core_is_hdr_and_shoulders() {
    drive_sun(false);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11, the 50x sun under SSAA (spawns px1a_hot_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_sun_core_is_hdr_and_shoulders_under_ssaa() {
    drive_sun(true);
}

// ---- host checks of the verdict (not ignored) ------------------------------------------------

#[test]
fn the_shoulder_check_reds_a_clipped_core_and_a_rising_ray() {
    let w = 8u32;
    let ramp: Vec<[u8; 3]> = (0..w).map(|x| [255 - 10 * x as u8; 3]).collect();
    check_display_shoulder(&ramp, w, (0, 0)).expect("a falling ramp from white is a shoulder");
    let dim: Vec<[u8; 3]> = (0..w).map(|_| [200u8; 3]).collect();
    assert!(check_display_shoulder(&dim, w, (0, 0)).unwrap_err().contains("not >= 254"));
    let mut bump = ramp.clone();
    bump[4] = [250; 3];
    assert!(check_display_shoulder(&bump, w, (0, 0)).unwrap_err().contains("no monotone shoulder"));
}

// ---- 11b: every producer stores above 1.0 (the switch commit) -----------------------------------

/// The worker the 11b drivers spawn — PX0's fixture body (`BOYKO_PX0_FIXTURE=<name>:hot`).
const FIXTURE_WORKER: &str = "px1a_hot_fixture_worker";

/// **THE 11b WORKER** — one PX0 branch fixture (`BOYKO_PX0_FIXTURE=<name>[:hot]`).
#[test]
#[ignore = "gpu-windowed: the PX1a 11b fixture worker, spawned by the px1a_hot_11b drivers; run with --test-threads=1"]
fn px1a_hot_fixture_worker() {
    fixture_worker_main(FIXTURE_WORKER);
}

/// Test 11b's verdict over one hot region: some channel above 1.0, and every texel's rgb within one
/// binary16 step of `want`. `Err` names the first failure.
fn check_hot_scene(region: &[[f32; 4]], want: f32) -> Result<(), String> {
    let max = region.iter().flat_map(|t| t[..3].iter().copied()).fold(f32::MIN, f32::max);
    if max <= 1.0 {
        return Err(format!("(D) max {max} <= 1.0 over the region — the store is display-referred or clamped"));
    }
    for (i, t) in region.iter().enumerate() {
        for (k, &v) in t[..3].iter().enumerate() {
            match steps_between(Channel::F16, v, want) {
                Ok(n) if n <= 1 => {}
                Ok(n) => return Err(format!("texel {i} channel {k} = {v}: {n} binary16 steps from {want}")),
                Err(e) => return Err(format!("texel {i} channel {k} = {v}: not a finite binary16 value ({e:?})")),
            }
        }
    }
    Ok(())
}

fn drive_11b(name: &str) {
    let f = fixture(name);
    assert_ne!(f.hot, Hot::None, "{name} has no hot variant");
    let c = capture_scene(FIXTURE_WORKER, f, true);
    if let Expect::Producer(keys) = f.expect {
        for (k, v) in keys {
            let got = c.receipt.config(k);
            assert_eq!(got, *v, "{name}: PREMISE [config] {k} = {got}, the fixture needs {v}");
        }
    }
    if let Err(e) = check_hot_scene(&c.region, HOT_R) {
        panic!("{name} (11b): {e}");
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, deferred_pbr surface stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_deferred_surface() {
    drive_11b("deferred_surface");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, deferred_pbr sky stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_deferred_sky() {
    drive_11b("deferred_sky");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, forward_opaque stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_forward_opaque() {
    drive_11b("forward_opaque");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, forward_opaque_froxel stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_forward_opaque_froxel() {
    drive_11b("forward_opaque_froxel");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, forward_sky stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_forward_sky() {
    drive_11b("forward_sky");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_sky stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_sky() {
    drive_11b("vb_sky");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, sdf_forward_march on Forward stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_sdf_forward_march_forward() {
    drive_11b("sdf_forward_march_forward");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, sdf_forward_march on VB stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_sdf_forward_march_vb() {
    drive_11b("sdf_forward_march_vb");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_resolve stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_resolve() {
    drive_11b("vb_resolve");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_shade stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_shade() {
    drive_11b("vb_shade");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, vb_shade_split stores above 1.0 (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_vb_shade_split() {
    drive_11b("vb_shade_split");
}

#[test]
#[ignore = "gpu-windowed: PX1a test 11b, the SSAA box stores above 1.0 into lit_native (spawns px1a_hot_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_hot_11b_ssaa_downsample() {
    drive_11b("ssaa_downsample");
}

// ---- 15: particle E1 (the switch commit) ---------------------------------------------------------

/// The 15 worker's exposure knob (`LightingConfig::exposure` for the run).
const ENV_PARTICLE_EXPOSURE: &str = "BOYKO_PX1A_PARTICLE_EXPOSURE";
/// The particle runs' window edge (the lab scene's `BOYKO_WIN`).
const PARTICLE_WIN: u32 = 256;
/// A pixel is a particle pixel when the particle-on run exceeds the particle-off run there by this
/// much luminance at exposure 1.0.
const PARTICLE_MASK_LUMA: f32 = 0.05;
/// Anti-vacuity: the fewest particle pixels the verdict needs (the lab's fan covers hundreds even
/// at 256 px).
const MIN_PARTICLE_PIXELS: usize = 64;

/// **THE 15 WORKER** — the particle lab scene at `BOYKO_PX1A_PARTICLE_EXPOSURE`.
#[test]
#[ignore = "gpu-windowed: the PX1a test 15 particle worker, spawned by px1a_particle_e1_pre_exposure; run with --test-threads=1"]
fn px1a_particle_worker() {
    let Ok(e) = std::env::var(ENV_PARTICLE_EXPOSURE) else {
        eprintln!("px1a_particle_worker: {ENV_PARTICLE_EXPOSURE} is unset -- SKIPPED (spawned by its driver).");
        return;
    };
    let exposure: f32 = e.trim().parse().unwrap_or_else(|_| panic!("{ENV_PARTICLE_EXPOSURE}={e} is not an f32"));
    let mut app = particle_scene::build_app("boyko_engine px1a particle E1");
    app.world_mut().resource_mut::<boyko_render::LightingConfig>().exposure = exposure;
    app.run();
    assert!(app.world().resource::<HostFrameStats>().frames > 0, "the frame loop never ran — RED, never a skip");
}

/// One particle run's `lit` at the lab's capture frame.
fn particle_lit(tag: &str, exposure: f32, particles_on: bool) -> DumpFile {
    let dir = fresh_out(&format!("px1a_particle_{tag}"));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let mut env = vec![
        (ENV_PARTICLE_EXPOSURE, exposure.to_string()),
        ("BOYKO_RENDER_PATH", "vb".to_owned()),
        ("BOYKO_WIN", PARTICLE_WIN.to_string()),
        ("BOYKO_IMAGE_DUMP", "lit".to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", particle_scene::CAPTURE_FRAME.to_string()),
        ("BOYKO_WINDOW_FRAMES", (particle_scene::CAPTURE_FRAME + 10).to_string()),
    ];
    if !particles_on {
        env.push(("BOYKO_PARTICLE_MODE", "off".to_owned()));
    }
    run_worker("px1a_particle_worker", &env, false);
    let files = files_of(&dir, "lit");
    assert_eq!(files.len(), 1, "{tag}: one `lit` capture expected: {files:?}");
    DumpFile::load(&files[0].1)
}

/// Test 15's verdict: over the particle pixels (`on1` above `off1` by [`PARTICLE_MASK_LUMA`]),
/// `half` is `on1 / 2` within one binary16 step per channel. `Ok(particle pixels)` or the failure.
fn check_particle_e1(on1: &[[f32; 4]], half: &[[f32; 4]], off1: &[[f32; 4]]) -> Result<usize, String> {
    let mut n = 0;
    for (i, ((a, h), o)) in on1.iter().zip(half).zip(off1).enumerate() {
        if luma(*a) - luma(*o) <= PARTICLE_MASK_LUMA {
            continue;
        }
        n += 1;
        for (k, (&full, &got)) in a[..3].iter().zip(&h[..3]).enumerate() {
            let want = full * 0.5;
            match steps_between(Channel::F16, got, want) {
                Ok(s) if s <= 1 => {}
                Ok(s) => {
                    return Err(format!(
                        "particle pixel {i} channel {k}: lit at exposure 0.5 = {got}, half of exposure 1.0's = {want} — {s} binary16 steps apart (no pre-exposure on the particle?)"
                    ));
                }
                Err(e) => return Err(format!("particle pixel {i} channel {k}: {e:?}")),
            }
        }
    }
    if n < MIN_PARTICLE_PIXELS {
        return Err(format!("only {n} particle pixels (floor {MIN_PARTICLE_PIXELS}) — the particles did not reach lit"));
    }
    Ok(n)
}

fn texels(d: &DumpFile) -> Vec<[f32; 4]> {
    (0..d.height).flat_map(|y| (0..d.width).map(move |x| (x, y))).map(|(x, y)| d.texel_f32(x, y)).collect()
}

#[test]
#[ignore = "gpu-windowed: PX1a test 15, particle E1 at exposure 1.0 vs 0.5 (spawns px1a_particle_worker three times); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_particle_e1_pre_exposure() {
    let on1 = particle_lit("on1", 1.0, true);
    let half = particle_lit("on_half", 0.5, true);
    let off1 = particle_lit("off1", 1.0, false);
    for d in [&half, &off1] {
        assert_eq!((d.width, d.height), (on1.width, on1.height), "the three captures' extents");
    }
    match check_particle_e1(&texels(&on1), &texels(&half), &texels(&off1)) {
        Ok(n) => println!("px1a_particle_e1_pre_exposure: {n} particle pixels halve within one binary16 step"),
        Err(e) => panic!("15: {e}"),
    }
}

#[test]
fn the_11b_and_15_verdicts_red_their_failure_classes() {
    let hot = [[2.0, 2.0, 2.0, 1.0]; 4];
    check_hot_scene(&hot, 2.0).expect("an exact hot region");
    assert!(check_hot_scene(&[[1.0, 1.0, 1.0, 1.0]; 4], 2.0).unwrap_err().contains("<= 1.0"), "a clamped store");
    let mut two_steps = hot;
    two_steps[2][1] = 2.0 + 2.0 / 512.0;
    assert!(check_hot_scene(&two_steps, 2.0).unwrap_err().contains("binary16 steps"), "a texel two steps off");

    let n = MIN_PARTICLE_PIXELS;
    let on1 = vec![[1.5, 1.0, 0.5, 1.0]; n];
    let off1 = vec![[0.25, 0.25, 0.25, 1.0]; n];
    let half: Vec<[f32; 4]> = on1.iter().map(|t| [t[0] * 0.5, t[1] * 0.5, t[2] * 0.5, 1.0]).collect();
    assert_eq!(check_particle_e1(&on1, &half, &off1), Ok(n));
    // No pre-exposure on the particle: only the background halves.
    let unscaled: Vec<[f32; 4]> =
        on1.iter().zip(&off1).map(|(a, o)| [a[0] - o[0] * 0.5, a[1] - o[1] * 0.5, a[2] - o[2] * 0.5, 1.0]).collect();
    assert!(check_particle_e1(&on1, &unscaled, &off1).unwrap_err().contains("no pre-exposure"));
    assert!(check_particle_e1(&on1[..8], &half[..8], &off1[..8]).unwrap_err().contains("did not reach lit"));
}
