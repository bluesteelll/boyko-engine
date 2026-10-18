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
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig};
use boyko_rhi_vulkan::present::ImageDumpName;

use graph_receipt::{fresh_out, run_worker};
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
