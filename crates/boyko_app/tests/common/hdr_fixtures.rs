//! Render wave PX0 / PX1a — **the HDR branch fixtures, shared**: the fifteen PX0 branch fixtures
//! (`FIXTURES`), their scene builder and worker body, and the capture of one fixture variant —
//! its display-referred region ([`capture`]) or its scene-linear one ([`capture_scene`], PX1a
//! test 11b).
//! Extracted from `hdr_branch_fixtures.rs` (PX0's tests are the same tests, `px1a/cut.md` C4) so
//! `px1a_hot_fixtures.rs` (PX1a tests 11b, 12, 15) drives the same scenes.
//!
//! Pulled in with `#[path = "common/hdr_fixtures.rs"] mod hdr_fixtures;` beside the sibling
//! modules it reads: `#[path = "common/graph_receipt.rs"] mod graph_receipt;` and
//! `#[path = "common/image_dump.rs"] mod image_dump;`.

#![allow(dead_code)] // each binary uses a subset

use std::path::Path;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::light::LightingConfig;
use boyko_render::post::ResolvedPost;
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig, SsaoConfig, SsaoQuality};
use boyko_rhi_vulkan::present::LIT_BACKGROUND_SCENE;
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{TONEMAP_MODE_MASK, TONEMAP_MODE_SHIFT, eval_params, post_final_eval, ubo_word};

use super::graph_receipt::{Receipt, fresh_out, run_worker};
use super::image_dump::{DumpFile, files_of};

/// Render wave PX1a test 12: `taa` arms TAA on a non-SSAA fixture (default: AA off).
pub const ENV_AA: &str = "BOYKO_PX1A_AA";
/// The fixture convention's force-key variable, decoded by the worker (`boyko_app::test_force`).
pub const ENV_TEST_FORCE: &str = "BOYKO_TEST_FORCE";
/// The worker's fixture knob: `<name>` or `<name>:hot`.
pub const ENV_FIXTURE: &str = "BOYKO_PX0_FIXTURE";
/// The window edge every fixture renders at.
pub const WIN: u32 = 256;
/// Frames before the capture.
pub const SETTLE: u32 = 30;
/// The hot radiance.
pub const HOT_R: f32 = 2.0;
/// The constant every constant branch STORES — the scene-linear radiance the three sites
/// (`FORWARD_LIT_CLEAR`, `forward_sky`'s fallback, Deferred's no-SkyLight arm) write since render
/// wave PX1a (`boyko_rhi_vulkan::present::LIT_BACKGROUND_SCENE`; binary16-exact, so `lit` holds it
/// bit for bit — `boyko_rhi_vulkan/tests/hdr_fixtures.rs`, test 5). Before PX1a they stored the
/// display-referred `(0.05, 0.05, 0.1)`. A constant region shows this radiance's `post_final`
/// display under the run's own words ([`display8`]), so it follows whichever curve the run used.
pub const BACKGROUND_SCENE: [f32; 3] = LIT_BACKGROUND_SCENE;
/// How far a region texel may sit from the traced body's display of its radiance, in 8-bit codes.
/// The device's float → UNORM conversion returns either integer nearest its value (Vulkan
/// Fundamentals, "Conversion from Floating-Point to Normalized Fixed-Point"; triage r1 F3: the
/// RTX 3060 writes `25.5000004` as 25), and that value is within half a code of
/// [`post_final_eval`]'s (the transcendental tolerance), so the code is within one of the oracle's
/// rounded code; two codes off is a different radiance.
pub const DISPLAY_LSB: u8 = 1;

// ===============================================================================================
// The fixture table
// ===============================================================================================

/// What a region must show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expect {
    /// This scene-linear constant's `post_final` display under the run's words ([`display8`]),
    /// within [`DISPLAY_LSB`].
    Constant([f32; 3]),
    /// A producer: these receipt keys hold these values, and the region is no constant.
    Producer(&'static [(&'static str, u64)]),
}

/// Where a region is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The centre 32×32 (on the object).
    Centre,
    /// The top-left 64×64 (the background).
    TopLeft,
}

impl Region {
    fn rect(self) -> (u32, u32, u32, u32) {
        match self {
            Region::Centre => (WIN / 2 - 16, WIN / 2 - 16, 32, 32),
            Region::TopLeft => (0, 0, 64, 64),
        }
    }
}

/// How the hot variant delivers `r ≥ 2.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hot {
    /// No hot variant (a display-referred constant).
    None,
    /// An emissive-only object.
    Emissive,
    /// A uniform sky.
    Sky,
}

/// One branch fixture.
#[derive(Clone, Copy, Debug)]
pub struct Fixture {
    pub name: &'static str,
    pub path: RenderPath,
    pub legs: GeometryLegs,
    pub sky: bool,
    pub ssao: bool,
    pub ssaa: bool,
    pub classified: bool,
    pub region: Region,
    pub expect: Expect,
    pub hot: Hot,
}

pub const fn fx(name: &'static str, path: RenderPath, legs: GeometryLegs, region: Region, expect: Expect, hot: Hot) -> Fixture {
    Fixture { name, path, legs, sky: false, ssao: false, ssaa: false, classified: false, region, expect, hot }
}

/// The fifteen regions of H3 Scope 8.
pub const FIXTURES: [Fixture; 15] = {
    use GeometryLegs::{Both, Mesh, Sdf};
    use Region::{Centre, TopLeft};
    use RenderPath::{Deferred, Forward, ForwardPlus, VisibilityBuffer};
    [
        fx("deferred_surface", Deferred, Mesh, Centre, Expect::Producer(&[("path", 0), ("mesh_leg", 1)]), Hot::Emissive),
        Fixture { sky: true, ..fx("deferred_sky", Deferred, Mesh, TopLeft, Expect::Producer(&[("path", 0), ("light_sky", 1)]), Hot::Sky) },
        fx("deferred_base", Deferred, Mesh, TopLeft, Expect::Constant(BACKGROUND_SCENE), Hot::None),
        fx("forward_opaque", Forward, Mesh, Centre, Expect::Producer(&[("path_is_forward", 1), ("path_is_forward_plus", 0)]), Hot::Emissive),
        fx("forward_opaque_froxel", ForwardPlus, Mesh, Centre, Expect::Producer(&[("path_is_forward_plus", 1)]), Hot::Emissive),
        Fixture { sky: true, ..fx("forward_sky", Forward, Mesh, TopLeft, Expect::Producer(&[("path_is_forward", 1), ("light_sky", 1)]), Hot::Sky) },
        fx("forward_sky_base", Forward, Mesh, TopLeft, Expect::Constant(BACKGROUND_SCENE), Hot::None),
        fx("forward_lit_clear", Forward, Sdf, TopLeft, Expect::Constant(BACKGROUND_SCENE), Hot::None),
        Fixture { sky: true, ..fx("vb_sky", VisibilityBuffer, Mesh, TopLeft, Expect::Producer(&[("path_is_vb", 1), ("light_sky", 1)]), Hot::Sky) },
        fx("sdf_forward_march_forward", Forward, Both, Centre, Expect::Producer(&[("path_is_forward", 1), ("path_has_sdf_forward", 1)]), Hot::Emissive),
        fx("sdf_forward_march_vb", VisibilityBuffer, Sdf, Centre, Expect::Producer(&[("path_is_vb", 1), ("path_has_sdf_forward", 1)]), Hot::Emissive),
        fx("vb_resolve", VisibilityBuffer, Mesh, Centre, Expect::Producer(&[("path_vb_fused", 1), ("vb_classified", 0)]), Hot::Emissive),
        Fixture { classified: true, ..fx("vb_shade", VisibilityBuffer, Mesh, Centre, Expect::Producer(&[("vb_classified", 1)]), Hot::Emissive) },
        Fixture { ssao: true, ..fx("vb_shade_split", VisibilityBuffer, Mesh, Centre, Expect::Producer(&[("path_vb_split", 1)]), Hot::Emissive) },
        Fixture { ssaa: true, sky: true, ..fx("ssaa_downsample", Deferred, Mesh, TopLeft, Expect::Producer(&[("ssaa", 1)]), Hot::Sky) },
    ]
};

pub fn fixture(name: &str) -> &'static Fixture {
    FIXTURES.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{ENV_FIXTURE}={name} names no fixture"))
}

/// The `post_final` words a fixture frame read: its receipt's `[config]` `post_word0..7`
/// (`boyko_render::post_final_words` — one conversion with the bytes the frame uploaded).
pub fn receipt_post_words(r: &Receipt) -> [u32; 8] {
    core::array::from_fn(|i| u32::try_from(r.config(&format!("post_word{i}"))).expect("invariant: a 32-bit post word"))
}

/// The `post_final` words of the engine's default configuration at a 1×1 extent —
/// `LightingConfig::default()`, so `Tonemapper::default()`, exposure ratio 1, sharpen off: what a
/// fixture frame reads (no fixture inserts a `LightingConfig`), for the host checks, which have no
/// run.
pub fn default_post_words() -> [u32; 8] {
    let post = ResolvedPost::from_config(&LightingConfig::default());
    boyko_render::post_final_words(&boyko_app::post_host::post_final_gpu(&post, [1, 1]))
}

/// The tonemapper field of `words`' mode word.
pub fn tonemap_word(words: &[u32; 8]) -> u32 {
    (words[ubo_word("mode_words")] >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK
}

/// `quantize8(post_final_eval(rgb))` under `words` — the traced body over a one-texel scene of
/// radiance `rgb`. The extent words are set to that texel; every other word, the curve included,
/// is `words`'.
pub fn display8(mut words: [u32; 8], rgb: [f32; 3]) -> [u8; 3] {
    words[ubo_word("extent")] = 1;
    words[ubo_word("extent") + 1] = 1;
    let texel = [[rgb[0], rgb[1], rgb[2], 1.0]];
    let o = post_final_eval([0, 0], &eval_params(words), HostTex2::new(&texel, 1, 1));
    [o[0], o[1], o[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

/// The hot region's oracle: [`display8`] of the uniform radiance `r` under `words`.
pub fn hot8(words: [u32; 8], r: f32) -> [u8; 3] {
    display8(words, [r; 3])
}

/// `p` is a texel the display of the oracle's `want` may produce: every channel within
/// [`DISPLAY_LSB`].
pub fn displays(want: [u8; 3], p: [u8; 3]) -> bool {
    want.iter().zip(p).all(|(&w, v)| w.abs_diff(v) <= DISPLAY_LSB)
}

// ===============================================================================================
// The worker
// ===============================================================================================

/// The worker's fixture and variant, as the resource its startup system reads.
#[derive(Resource)]
pub struct FixtureRes(pub Fixture, pub bool);

#[allow(clippy::needless_pass_by_value)]
fn fixture_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    arming: Res<FixtureRes>,
) {
    let FixtureRes(f, hot) = *arming;
    let emissive = if hot && f.hot == Hot::Emissive { [HOT_R; 3] } else { [0.0; 3] };
    let albedo = if hot && f.hot == Hot::Emissive { [0.0, 0.0, 0.0, 1.0] } else { [0.7, 0.3, 0.2, 1.0] };
    let mat = materials.add(Material::new(albedo, 0.0, 0.6, 0.5, emissive, 0));
    let row = u16::try_from(mat.index()).expect("invariant: the fixture mints < 65536 rows");
    if matches!(f.legs, GeometryLegs::Mesh | GeometryLegs::Both) {
        let cube = meshes.cube(dev.get(), 1.4);
        commands.spawn(MeshBundle { material: MaterialHandle(row), ..MeshBundle::new(cube, Transform::IDENTITY) });
    }
    if matches!(f.legs, GeometryLegs::Sdf | GeometryLegs::Both) {
        // On Both the sphere sits in FRONT of the cube's face, so the centre region is the march.
        let z = if f.legs == GeometryLegs::Both { 1.2 } else { 0.0 };
        commands.spawn(SdfPrimitive(SdfEdit::sphere([0.0, 0.0, z], 0.7, sdf_op::UNION, 0.0).with_material(row)));
    }
    // The cold variants are lit by a sun; a hot variant carries NO light (rule xvi).
    if !hot {
        let dir = [-0.45f32, 0.82, 0.36];
        let pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(dir[0], dir[1], dir[2]), Vec3::new(0.0, 1.0, 0.0));
        commands.spawn(DirectionalLightObject {
            transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
            global: GlobalTransform::IDENTITY,
            light: DirectionalLight::new(dir, [1.0, 0.96, 0.90], 2.8),
        });
    }
    if f.sky {
        let (sky, ground) = if hot && f.hot == Hot::Sky { ([HOT_R; 3], [HOT_R; 3]) } else { ([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]) };
        commands.spawn(SkyLight::new(sky, ground));
    }
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: core::f32::consts::FRAC_PI_3, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// **THE WORKER** — one fixture (`BOYKO_PX0_FIXTURE=<name>[:hot]`).
/// The body of every fixture WORKER (`BOYKO_PX0_FIXTURE=<name>[:hot]`): each including binary
/// wraps it in its own ignored worker test, whose name its drivers pass to [`capture`] —
/// the worker must be a top-level test of the spawned binary for `--exact` to find it.
pub fn fixture_worker_main(worker: &str) {
    let Ok(spec) = std::env::var(ENV_FIXTURE) else {
        eprintln!("{worker}: {ENV_FIXTURE} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let (name, hot) = match spec.split_once(':') {
        Some((n, "hot")) => (n, true),
        Some((_, other)) => panic!("{ENV_FIXTURE}: `{other}` is not `hot`"),
        None => (spec.as_str(), false),
    };
    let f = *fixture(name);
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine px0 branch fixture", WIN, WIN);
    app.add_plugins(if f.ssaa { plugins.with_ssaa_scale(2) } else { plugins });
    app.insert_resource(FixtureRes(f, hot));
    // Render wave PX1a test 12: the force keys (an unknown key refuses the boot) and the TAA knob.
    let force = std::env::var(ENV_TEST_FORCE).unwrap_or_default();
    app.insert_resource(boyko_app::test_force::parse(&force).unwrap_or_else(|e| panic!("{e}")));
    let taa = std::env::var(ENV_AA).is_ok_and(|v| v == "taa");
    assert!(!(taa && f.ssaa), "{ENV_AA}=taa on the SSAA fixture: one AA mode per boot");
    app.add_startup_system(fixture_setup);
    app.insert_resource(RenderPathConfig { path: f.path, legs: f.legs });
    if f.ssao {
        app.insert_resource(SsaoConfig { quality: SsaoQuality::High, atrous_levels: 3 });
    }
    if f.ssaa {
        // The runner arms SSAA itself from the boot scale; nothing else to insert.
    } else {
        app.insert_resource(AaConfig { mode: if taa { AaMode::Taa } else { AaMode::Off } });
    }
    app.run();
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran — RED, never a skip");
}

/// One fixture variant's capture: the region's display-referred texels and the receipt.
pub struct Capture {
    pub region: Vec<[u8; 3]>,
    pub receipt: Receipt,
}

/// One fixture variant's SCENE-LINEAR capture (render wave PX1a test 11b): the region's texels of
/// the scene input `post_final` reads — `lit`, or `lit_native` under SSAA — and the receipt.
pub struct SceneCapture {
    pub region: Vec<[f32; 4]>,
    pub receipt: Receipt,
}

/// The `(x, y)` of every texel of `f`'s region, row-major.
fn region_texels(f: &Fixture) -> Vec<(u32, u32)> {
    let (x0, y0, w, h) = f.region.rect();
    (y0..y0 + h).flat_map(|y| (x0..x0 + w).map(move |x| (x, y))).collect()
}

/// Captures `f`'s scene input over its region: `BOYKO_IMAGE_DUMP=lit` (`lit_native` under SSAA,
/// whose region is in native pixels — the window's), decoded to `f32` through `pixfmt`.
pub fn capture_scene(worker: &str, f: &Fixture, hot: bool) -> SceneCapture {
    let tag = format!("{}{}", f.name, if hot { "_hot" } else { "" });
    let dir = fresh_out(&format!("px1a_scene_{tag}"));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let receipt = dir.join("receipt.toml");
    let scene = if f.ssaa { "lit_native" } else { "lit" };
    let mut env = vec![
        (ENV_FIXTURE, format!("{}{}", f.name, if hot { ":hot" } else { "" })),
        ("BOYKO_GRAPH_RECEIPT", receipt.display().to_string()),
        ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
        ("BOYKO_IMAGE_DUMP", scene.to_owned()),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
    ];
    if f.classified {
        env.push(("BOYKO_VB_FORCE_CLASSIFIED", "1".to_owned()));
    }
    run_worker(worker, &env, false);
    let files = files_of(&dir, scene);
    assert_eq!(files.len(), 1, "{tag}: one `{scene}` capture expected: {files:?}");
    let d = DumpFile::load(&files[0].1);
    assert_eq!((d.width, d.height), (WIN, WIN), "{tag}: `{scene}` is not at the window's extent");
    let region = region_texels(f).into_iter().map(|(x, y)| d.texel_f32(x, y)).collect();
    SceneCapture { region, receipt: Receipt::load(&receipt) }
}

pub fn capture(worker: &str, f: &Fixture, hot: bool, keep_in: Option<&Path>) -> Capture {
    let tag = format!("{}{}", f.name, if hot { "_hot" } else { "" });
    let dir = fresh_out(&format!("px0_fixture_{tag}"));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let receipt = dir.join("receipt.toml");
    let bmp = dir.join("host.bmp");
    let mut env = vec![
        (ENV_FIXTURE, format!("{}{}", f.name, if hot { ":hot" } else { "" })),
        ("BOYKO_GRAPH_RECEIPT", receipt.display().to_string()),
        ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 40).to_string()),
    ];
    if f.ssaa {
        env.push(("BOYKO_HOST_DUMP", bmp.display().to_string()));
    } else {
        // `display` is the display-referred image (`lit` is scene-linear since render wave PX1a);
        // `lit` rides along for a `keep_in` copy.
        env.push(("BOYKO_IMAGE_DUMP", "lit,display".to_owned()));
        env.push(("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()));
        env.push(("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()));
    }
    if f.classified {
        env.push(("BOYKO_VB_FORCE_CLASSIFIED", "1".to_owned()));
    }
    run_worker(worker, &env, false);
    let region = if f.ssaa {
        let px = read_bmp_rgb(&bmp);
        region_texels(f).into_iter().map(|(x, y)| px[(y * WIN + x) as usize]).collect()
    } else {
        let files = files_of(&dir, "display");
        assert_eq!(files.len(), 1, "{tag}: one `display` capture expected: {files:?}");
        let d = DumpFile::load(&files[0].1);
        region_texels(f)
            .into_iter()
            .map(|(x, y)| {
                let t = d.rgba8(x, y);
                [t[0], t[1], t[2]]
            })
            .collect()
    };
    if let Some(keep) = keep_in {
        std::fs::create_dir_all(keep).expect("the premove dir is creatable");
        let _ = std::fs::copy(&receipt, keep.join(format!("fixture_{tag}.toml")));
        for (_, p) in files_of(&dir, "lit") {
            let _ = std::fs::copy(&p, keep.join(format!("fixture_{tag}_lit.bin")));
        }
        for (_, p) in files_of(&dir, "display") {
            let _ = std::fs::copy(&p, keep.join(format!("fixture_{tag}_display.bin")));
        }
        if f.ssaa {
            let _ = std::fs::copy(&bmp, keep.join(format!("fixture_{tag}.bmp")));
        }
    }
    Capture { region, receipt: Receipt::load(&receipt) }
}

/// A 32-bpp bottom-up BMP's RGB, row-major top-down (the host dump's format).
pub fn read_bmp_rgb(path: &Path) -> Vec<[u8; 3]> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("no host dump at {} ({e})", path.display()));
    let off = u32::from_le_bytes(b[10..14].try_into().expect("4 bytes")) as usize;
    let w = i32::from_le_bytes(b[18..22].try_into().expect("4 bytes")) as usize;
    let h = i32::from_le_bytes(b[22..26].try_into().expect("4 bytes")).unsigned_abs() as usize;
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let p = off + ((h - 1 - y) * w + x) * 4;
            [b[p + 2], b[p + 1], b[p]]
        })
        .collect()
}
