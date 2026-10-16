//! Render wave PX0 (H3 Scope 8; `px0/cut.md` D-F1..D-F3, PX0-11, PX0-12) — **the HDR branch
//! fixtures, their hot variants, the producer matrix, and the two receipt keys' device gates**.
//!
//! # The fixtures
//!
//! One small scene per tail branch, each with a screen REGION where only that branch runs (the
//! table in [`FIXTURES`]): a centre 32×32 on the object for a surface producer, the top-left 64×64
//! for a background branch. Each region is read from the frame's `lit` image dump (RGBA8 at HEAD,
//! `BOYKO_IMAGE_DUMP=lit`), or — for the SSAA box, whose output `aa_out` has no dump name — from
//! the host dump BMP (`BOYKO_IMAGE_DUMP` and `BOYKO_HOST_DUMP` are exclusive).
//!
//! - **Constant branches** (`lit = base`, `BACKGROUND_BASE`, `FORWARD_LIT_CLEAR`): every texel of
//!   the region is a UNORM conversion of the constant the spec permits — per channel one of the two
//!   codes nearest `f · 255` ([`constant8`]; a rounding mode is not the spec's).
//! - **Producer branches**: the receipt keys that select them hold the fixture's values, and the
//!   region is not all conversions of the background constant.
//! - **Hot variants** (every lit-store producer and the SSAA box): an emissive-only surface (black
//!   albedo, no light, no SkyLight, SSAO / DDGI off — rule xvi's zeroed terms) or a uniform sky
//!   (`sky = ground = r`, no directional) at `r = 2.0`; the region equals
//!   `quantize8(post_final_eval(r))` within 1 LSB — the hot oracle is the traced body itself
//!   (`boyko_shaderdsl::post`), at HEAD's ACES Hill + OETF — which proves the fixture delivers
//!   `r ≥ 2.0` to its region now, instead of a wrong red at PX1a's test 11b.
//!
//! # The producer matrix ([`producer_matrix`], host-tested here)
//!
//! Over the transition-pinned legs' receipts (non-TAA, non-particle) ∪ the fixtures: every branch
//! needs ≥ 1 region and every lit-store producer ≥ 1 hot region; a gap is red. With
//! `BOYKO_PX0_PREMOVE_DIR=<dir>` (GL-H0) the matrix driver reads the dir's pin receipts, runs every
//! fixture into it, and writes `<dir>/producer_matrix.md`. The three `boyko_rhi_vulkan` harness
//! pins (`grand_showcase`, `deferred_sdf_only`, `deferred_mesh_only`) have no runner and no
//! receipt; the matrix lists them as "no receipt".
//!
//! # The receipt keys (PX0-11)
//!
//! [`receipt_light_sky_tracks_the_light_table`] and [`receipt_vb_classified_tracks_the_producer`]
//! are the device red-firsts of `light_sky` / `vb_classified` (red before the keys exist: "no key").
//!
//! Windowed-test conventions: the device drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use std::path::{Path, PathBuf};

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{AaConfig, AaMode, GeometryLegs, Material, RenderPath, RenderPathConfig, SsaoConfig, SsaoQuality};
use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::post::{eval_params, post_final_eval};

use graph_receipt::{Receipt, fresh_out, run_worker};
use image_dump::{DumpFile, files_of};

/// The worker's fixture knob: `<name>` or `<name>:hot`.
const ENV_FIXTURE: &str = "BOYKO_PX0_FIXTURE";
/// The window edge every fixture renders at.
const WIN: u32 = 256;
/// Frames before the capture.
const SETTLE: u32 = 30;
/// The hot radiance.
const HOT_R: f32 = 2.0;
/// The constant background every constant branch writes (`FORWARD_LIT_CLEAR`, `BACKGROUND_BASE`,
/// the G-buffer albedo clear `lit = base` reads) — the same linear value on all three.
const BACKGROUND: [f32; 3] = [0.05, 0.05, 0.1];

// ===============================================================================================
// The fixture table
// ===============================================================================================

/// What a region must show.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Expect {
    /// Exactly this constant's 8-bit value.
    Constant([f32; 3]),
    /// A producer: these receipt keys hold these values, and the region is no constant.
    Producer(&'static [(&'static str, u64)]),
}

/// Where a region is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
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
enum Hot {
    /// No hot variant (a display-referred constant).
    None,
    /// An emissive-only object.
    Emissive,
    /// A uniform sky.
    Sky,
}

/// One branch fixture.
#[derive(Clone, Copy, Debug)]
struct Fixture {
    name: &'static str,
    path: RenderPath,
    legs: GeometryLegs,
    sky: bool,
    ssao: bool,
    ssaa: bool,
    classified: bool,
    region: Region,
    expect: Expect,
    hot: Hot,
}

const fn fx(name: &'static str, path: RenderPath, legs: GeometryLegs, region: Region, expect: Expect, hot: Hot) -> Fixture {
    Fixture { name, path, legs, sky: false, ssao: false, ssaa: false, classified: false, region, expect, hot }
}

/// The fifteen regions of H3 Scope 8.
const FIXTURES: [Fixture; 15] = {
    use GeometryLegs::{Both, Mesh, Sdf};
    use Region::{Centre, TopLeft};
    use RenderPath::{Deferred, Forward, ForwardPlus, VisibilityBuffer};
    [
        fx("deferred_surface", Deferred, Mesh, Centre, Expect::Producer(&[("path", 0), ("mesh_leg", 1)]), Hot::Emissive),
        Fixture { sky: true, ..fx("deferred_sky", Deferred, Mesh, TopLeft, Expect::Producer(&[("path", 0), ("light_sky", 1)]), Hot::Sky) },
        fx("deferred_base", Deferred, Mesh, TopLeft, Expect::Constant(BACKGROUND), Hot::None),
        fx("forward_opaque", Forward, Mesh, Centre, Expect::Producer(&[("path_is_forward", 1), ("path_is_forward_plus", 0)]), Hot::Emissive),
        fx("forward_opaque_froxel", ForwardPlus, Mesh, Centre, Expect::Producer(&[("path_is_forward_plus", 1)]), Hot::Emissive),
        Fixture { sky: true, ..fx("forward_sky", Forward, Mesh, TopLeft, Expect::Producer(&[("path_is_forward", 1), ("light_sky", 1)]), Hot::Sky) },
        fx("forward_sky_base", Forward, Mesh, TopLeft, Expect::Constant(BACKGROUND), Hot::None),
        fx("forward_lit_clear", Forward, Sdf, TopLeft, Expect::Constant(BACKGROUND), Hot::None),
        Fixture { sky: true, ..fx("vb_sky", VisibilityBuffer, Mesh, TopLeft, Expect::Producer(&[("path_is_vb", 1), ("light_sky", 1)]), Hot::Sky) },
        fx("sdf_forward_march_forward", Forward, Both, Centre, Expect::Producer(&[("path_is_forward", 1), ("path_has_sdf_forward", 1)]), Hot::Emissive),
        fx("sdf_forward_march_vb", VisibilityBuffer, Sdf, Centre, Expect::Producer(&[("path_is_vb", 1), ("path_has_sdf_forward", 1)]), Hot::Emissive),
        fx("vb_resolve", VisibilityBuffer, Mesh, Centre, Expect::Producer(&[("path_vb_fused", 1), ("vb_classified", 0)]), Hot::Emissive),
        Fixture { classified: true, ..fx("vb_shade", VisibilityBuffer, Mesh, Centre, Expect::Producer(&[("vb_classified", 1)]), Hot::Emissive) },
        Fixture { ssao: true, ..fx("vb_shade_split", VisibilityBuffer, Mesh, Centre, Expect::Producer(&[("path_vb_split", 1)]), Hot::Emissive) },
        Fixture { ssaa: true, sky: true, ..fx("ssaa_downsample", Deferred, Mesh, TopLeft, Expect::Producer(&[("ssaa", 1)]), Hot::Sky) },
    ]
};

fn fixture(name: &str) -> &'static Fixture {
    FIXTURES.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{ENV_FIXTURE}={name} names no fixture"))
}

/// The 8-bit codes the RGBA8 `lit` store may write for a linear constant, per channel `(lo, hi)`.
///
/// Vulkan's float → UNORM conversion (Fundamentals, "Conversion from Floating-Point to Normalized
/// Fixed-Point") returns ONE OF the two integers nearest `f · 255` — it "should" round to nearest,
/// it must return the integer when the product is one — so the oracle is that pair, not a rounding
/// mode (triage r1 F3: the RTX 3060 writes blue `0.1 · 255 = 25.5000004` as 25). The product is
/// taken in f64, where it is exact for an f32 `f`; `lo == hi` exactly when it is integral.
fn constant8(c: [f32; 3]) -> [(u8, u8); 3] {
    c.map(|v| {
        let r = f64::from(v.clamp(0.0, 1.0)) * 255.0;
        (r.floor() as u8, r.ceil() as u8)
    })
}

/// `p` is a texel the UNORM conversion of the constant `c` may produce ([`constant8`]).
fn admits(c: [f32; 3], p: [u8; 3]) -> bool {
    constant8(c).iter().zip(p).all(|(&(lo, hi), v)| (lo..=hi).contains(&v))
}

/// `quantize8(post_final_eval(r))` — the hot region's oracle: the traced body over a one-texel
/// scene of radiance `r`, ACES Hill (mode 0), exposure ratio 1.
fn hot8(r: f32) -> [u8; 3] {
    let mut words = [0u32; 8];
    words[boyko_shaderdsl::post::ubo_word("exposure_ratio")] = 1.0f32.to_bits();
    words[boyko_shaderdsl::post::ubo_word("extent")] = 1;
    words[boyko_shaderdsl::post::ubo_word("extent") + 1] = 1;
    let texel = [[r, r, r, 1.0]];
    let o = post_final_eval([0, 0], &eval_params(words), HostTex2::new(&texel, 1, 1));
    [o[0], o[1], o[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

// ===============================================================================================
// The worker
// ===============================================================================================

/// The worker's fixture and variant, as the resource its startup system reads.
#[derive(Resource)]
struct FixtureRes(Fixture, bool);

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
#[test]
#[ignore = "gpu-windowed: the PX0 branch-fixture worker, spawned by the hdr_branch_fixtures drivers; run with --test-threads=1"]
fn hdr_branch_fixture_worker() {
    let Ok(spec) = std::env::var(ENV_FIXTURE) else {
        eprintln!("hdr_branch_fixture_worker: {ENV_FIXTURE} is unset -- SKIPPED (spawned by its drivers).");
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
    app.add_startup_system(fixture_setup);
    app.insert_resource(RenderPathConfig { path: f.path, legs: f.legs });
    if f.ssao {
        app.insert_resource(SsaoConfig { quality: SsaoQuality::High, atrous_levels: 3 });
    }
    if f.ssaa {
        // The runner arms SSAA itself from the boot scale; nothing else to insert.
    } else {
        app.insert_resource(AaConfig { mode: AaMode::Off });
    }
    app.run();
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran — RED, never a skip");
}

// ===============================================================================================
// The drivers
// ===============================================================================================

/// One fixture variant's capture: the region's texels and the receipt.
struct Capture {
    region: Vec<[u8; 3]>,
    receipt: Receipt,
}

fn capture(f: &Fixture, hot: bool, keep_in: Option<&Path>) -> Capture {
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
        env.push(("BOYKO_IMAGE_DUMP", "lit".to_owned()));
        env.push(("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()));
        env.push(("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()));
    }
    if f.classified {
        env.push(("BOYKO_VB_FORCE_CLASSIFIED", "1".to_owned()));
    }
    run_worker("hdr_branch_fixture_worker", &env, false);
    let (x0, y0, w, h) = f.region.rect();
    let region = if f.ssaa {
        let px = read_bmp_rgb(&bmp);
        (y0..y0 + h).flat_map(|y| (x0..x0 + w).map(move |x| (x, y))).map(|(x, y)| px[(y * WIN + x) as usize]).collect()
    } else {
        let files = files_of(&dir, "lit");
        assert_eq!(files.len(), 1, "{tag}: one `lit` capture expected: {files:?}");
        let d = DumpFile::load(&files[0].1);
        (y0..y0 + h)
            .flat_map(|y| (x0..x0 + w).map(move |x| (x, y)))
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
        if f.ssaa {
            let _ = std::fs::copy(&bmp, keep.join(format!("fixture_{tag}.bmp")));
        }
    }
    Capture { region, receipt: Receipt::load(&receipt) }
}

/// A 32-bpp bottom-up BMP's RGB, row-major top-down (the host dump's format).
fn read_bmp_rgb(path: &Path) -> Vec<[u8; 3]> {
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

/// The checks of one capture against its fixture (`hot` selects the hot oracle).
fn check(f: &Fixture, hot: bool, c: &Capture) -> Result<(), String> {
    let all = |want: [u8; 3], tol: u8| c.region.iter().all(|p| (0..3).all(|k| p[k].abs_diff(want[k]) <= tol));
    if hot {
        let want = hot8(HOT_R);
        return if all(want, 1) {
            Ok(())
        } else {
            Err(format!(
                "{} (hot): region radiance below {HOT_R}: expected quantize8(post_final_eval({HOT_R})) = {want:?} within 1 LSB, first texel {:?}",
                f.name, c.region[0]
            ))
        };
    }
    match f.expect {
        Expect::Constant(k) => match c.region.iter().find(|p| !admits(k, **p)) {
            None => Ok(()),
            Some(p) => Err(format!(
                "{}: the constant region is not a UNORM conversion of {k:?} (per channel {:?}; first other texel {p:?})",
                f.name,
                constant8(k)
            )),
        },
        Expect::Producer(keys) => {
            for (k, v) in keys {
                let got = c.receipt.config(k);
                if got != *v {
                    return Err(format!("{}: PREMISE [config] {k} = {got}, the fixture needs {v}", f.name));
                }
            }
            if c.region.iter().all(|p| admits(BACKGROUND, *p)) {
                return Err(format!(
                    "{}: region empty — every texel is a UNORM conversion of the background constant (per channel {:?})",
                    f.name,
                    constant8(BACKGROUND)
                ));
            }
            Ok(())
        }
    }
}

fn drive(name: &str) {
    let f = fixture(name);
    let cold = capture(f, false, None);
    if let Err(e) = check(f, false, &cold) {
        panic!("{e}");
    }
    if f.hot != Hot::None {
        let hot = capture(f, true, None);
        if let Err(e) = check(f, true, &hot) {
            panic!("{e}");
        }
    }
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (deferred surface + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_deferred_surface() {
    drive("deferred_surface");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (deferred SkyLight sky + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_deferred_sky() {
    drive("deferred_sky");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (deferred lit = base); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_deferred_base() {
    drive("deferred_base");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_opaque + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_opaque() {
    drive("forward_opaque");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_opaque_froxel + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_opaque_froxel() {
    drive("forward_opaque_froxel");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_sky with SkyLight + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_sky() {
    drive("forward_sky");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_sky BACKGROUND_BASE); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_sky_base() {
    drive("forward_sky_base");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (FORWARD_LIT_CLEAR); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_lit_clear() {
    drive("forward_lit_clear");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_sky + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_sky() {
    drive("vb_sky");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (sdf_forward_march on Forward + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_sdf_forward_march_forward() {
    drive("sdf_forward_march_forward");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (sdf_forward_march on VB + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_sdf_forward_march_vb() {
    drive("sdf_forward_march_vb");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_resolve + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_resolve() {
    drive("vb_resolve");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_shade, classified + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_shade() {
    drive("vb_shade");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_shade_split + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_shade_split() {
    drive("vb_shade_split");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (the SSAA box, host dump + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_ssaa_downsample() {
    drive("ssaa_downsample");
}

/// **PX0-11's `light_sky` gate**: Forward × Mesh without a SkyLight — `light_sky = 0` while the
/// sky PIPELINE is still built (`forward_sky = 1`, OBS's old proxy); with one — `light_sky = 1`.
#[test]
#[ignore = "gpu-windowed: PX0-11 receipt gate for light_sky (spawns hdr_branch_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn receipt_light_sky_tracks_the_light_table() {
    let without = capture(fixture("forward_sky_base"), false, None);
    assert_eq!(without.receipt.config("forward_sky"), 1, "PREMISE: the sky pipeline is built on every Forward boot");
    assert_eq!(without.receipt.config("light_sky"), 0, "a table with no SkyLight must read light_sky = 0");
    let with = capture(fixture("forward_sky"), false, None);
    assert_eq!(with.receipt.config("light_sky"), 1, "a table with a SkyLight must read light_sky = 1");
}

/// **PX0-11's `vb_classified` gate**: the classified producer (`BOYKO_VB_FORCE_CLASSIFIED`, the
/// producer selector's own knob) reads 1, the fused one 0.
#[test]
#[ignore = "gpu-windowed: PX0-11 receipt gate for vb_classified (spawns hdr_branch_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn receipt_vb_classified_tracks_the_producer() {
    let fused = capture(fixture("vb_resolve"), false, None);
    assert_eq!(fused.receipt.config("vb_classified"), 0, "the fused producer must read vb_classified = 0");
    let classified = capture(fixture("vb_shade"), false, None);
    assert_eq!(classified.receipt.config("vb_classified"), 1, "the classified producer must read vb_classified = 1");
}

// ===============================================================================================
// The producer matrix
// ===============================================================================================

/// The lit-store producers (each needs a hot region).
const LIT_STORE: [&str; 12] = [
    "deferred_surface",
    "deferred_sky",
    "forward_opaque",
    "forward_opaque_froxel",
    "forward_sky",
    "vb_sky",
    "sdf_forward_march_forward",
    "sdf_forward_march_vb",
    "vb_resolve",
    "vb_shade",
    "vb_shade_split",
    "ssaa_downsample",
];

/// The `boyko_rhi_vulkan` harness pins: no runner, no receipt.
const NO_RECEIPT_PINS: [&str; 3] = ["grand_showcase", "deferred_sdf_only", "deferred_mesh_only"];

/// The branches a transition-pinned leg's `[config]` exercises (the recorder's own predicates).
fn branches_of(cfg: &dyn Fn(&str) -> u64) -> Vec<&'static str> {
    let mut b = Vec::new();
    let mesh = cfg("mesh_leg") == 1;
    match cfg("path") {
        0 => {
            if mesh {
                b.push("deferred_surface");
            }
            b.push(if cfg("light_sky") == 1 { "deferred_sky" } else { "deferred_base" });
            if cfg("ssaa") == 1 {
                b.push("ssaa_downsample");
            }
        }
        1 | 2 => {
            if mesh {
                b.push(if cfg("path_is_forward_plus") == 1 { "forward_opaque_froxel" } else { "forward_opaque" });
                b.push(if cfg("light_sky") == 1 { "forward_sky" } else { "forward_sky_base" });
            } else {
                b.push("forward_lit_clear");
            }
            if cfg("path_has_sdf_forward") == 1 {
                b.push("sdf_forward_march_forward");
            }
        }
        _ => {
            b.push(if cfg("light_sky") == 1 { "vb_sky" } else { "forward_sky_base" });
            if cfg("path_has_sdf_forward") == 1 {
                b.push("sdf_forward_march_vb");
            }
            if mesh {
                b.push(if cfg("path_vb_split") == 1 {
                    "vb_shade_split"
                } else if cfg("vb_classified") == 1 {
                    "vb_shade"
                } else {
                    "vb_resolve"
                });
            }
        }
    }
    b
}

/// One input of the matrix: a leg (a pin's receipt) or a fixture (with whether its hot region
/// passed).
struct MatrixInput<'a> {
    name: String,
    cfg: Box<dyn Fn(&str) -> u64 + 'a>,
    fixture: Option<(&'static str, bool)>,
}

/// The matrix: per branch, the legs and fixtures covering it; red when a branch has no region or a
/// lit-store producer no hot region.
fn producer_matrix(inputs: &[MatrixInput<'_>]) -> Result<String, Vec<String>> {
    let mut out = String::from("| branch | transition-pinned legs | fixture | hot region |\n|---|---|---|---|\n");
    let mut bad = Vec::new();
    for f in FIXTURES {
        let legs: Vec<&str> = inputs
            .iter()
            .filter(|i| i.fixture.is_none())
            .filter(|i| (i.cfg)("taa") == 0 && (i.cfg)("path_has_particles") == 0)
            .filter(|i| branches_of(&*i.cfg).contains(&f.name))
            .map(|i| i.name.as_str())
            .collect();
        let fix = inputs.iter().find(|i| i.fixture.is_some_and(|(n, _)| n == f.name));
        let hot = fix.is_some_and(|i| i.fixture.is_some_and(|(_, h)| h));
        if legs.is_empty() && fix.is_none() {
            bad.push(format!("branch `{}` is reached by no transition-pinned leg and no fixture", f.name));
        }
        if LIT_STORE.contains(&f.name) && !hot {
            bad.push(format!("lit-store producer `{}` has no hot region (r >= {HOT_R})", f.name));
        }
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            f.name,
            if legs.is_empty() { "—".to_owned() } else { legs.join(", ") },
            if fix.is_some() { "yes" } else { "—" },
            if hot { "yes" } else { "—" }
        ));
    }
    out.push_str(&format!("\nNo receipt (boyko_rhi_vulkan harness pins, no runner): {}\n", NO_RECEIPT_PINS.join(", ")));
    if bad.is_empty() { Ok(out) } else { Err(bad) }
}

/// **The matrix over GL-H0's pre-move set** (`BOYKO_PX0_PREMOVE_DIR`): every fixture (both
/// variants) dumped into the dir, every pin receipt in it read, `producer_matrix.md` written.
#[test]
#[ignore = "gpu-windowed: PX0 producer matrix over GL-H0's pre-move dir (BOYKO_PX0_PREMOVE_DIR; runs every fixture); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn producer_matrix_over_premove() {
    let Ok(dir) = std::env::var("BOYKO_PX0_PREMOVE_DIR") else {
        panic!("BOYKO_PX0_PREMOVE_DIR is unset: name GL-H0's pre-move dir");
    };
    let dir = PathBuf::from(dir);
    let mut inputs: Vec<MatrixInput<'static>> = Vec::new();
    for f in FIXTURES {
        let cold = capture(&f, false, Some(&dir));
        check(&f, false, &cold).unwrap_or_else(|e| panic!("{e}"));
        let hot = f.hot != Hot::None && {
            let h = capture(&f, true, Some(&dir));
            check(&f, true, &h).unwrap_or_else(|e| panic!("{e}"));
            true
        };
        let r = cold.receipt;
        inputs.push(MatrixInput { name: f.name.to_owned(), cfg: Box::new(move |k| r.config(k)), fixture: Some((f.name, hot)) });
    }
    for e in std::fs::read_dir(&dir).expect("the premove dir is readable").flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        if !name.ends_with(".toml") || name.starts_with("fixture_") {
            continue;
        }
        let r = Receipt::load(&p);
        inputs.push(MatrixInput { name, cfg: Box::new(move |k| r.config(k)), fixture: None });
    }
    match producer_matrix(&inputs) {
        Ok(md) => std::fs::write(dir.join("producer_matrix.md"), md).expect("write producer_matrix.md"),
        Err(bad) => panic!("producer matrix:\n{}", bad.join("\n")),
    }
}

/// The matrix on synthetic receipts, both directions: complete fixtures pass; a missing hot region
/// and a branch with no region are each named.
#[test]
fn the_matrix_reds_a_gap_and_passes_full_coverage() {
    let full: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.hot != Hot::None)) })
        .collect();
    assert!(producer_matrix(&full).is_ok(), "{:?}", producer_matrix(&full).err());

    let cold_only: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.name != "vb_shade" && f.hot != Hot::None)) })
        .collect();
    let e = producer_matrix(&cold_only).expect_err("a producer with no hot region must red");
    assert!(e.iter().any(|m| m.contains("`vb_shade` has no hot region")), "{e:?}");

    let missing: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .filter(|f| f.name != "forward_lit_clear")
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.hot != Hot::None)) })
        .collect();
    let e = producer_matrix(&missing).expect_err("a branch with no region must red");
    assert!(e.iter().any(|m| m.contains("`forward_lit_clear` is reached by no")), "{e:?}");

    // A transition-pinned leg covers a branch without a fixture: a Forward × Sdf receipt.
    let mut leg_only: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .filter(|f| f.name != "forward_lit_clear")
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.hot != Hot::None)) })
        .collect();
    leg_only.push(MatrixInput {
        name: "sdf_forward_only".to_owned(),
        cfg: Box::new(|k| u64::from(k == "path")),
        fixture: None,
    });
    assert!(producer_matrix(&leg_only).is_ok(), "{:?}", producer_matrix(&leg_only).err());
}

/// The hot oracle is hot: `r = 2.0` through HEAD's tail is above display mid-grey and below white,
/// and a cold `r = 0.5` differs from it (the "region radiance below 2.0" red a cold fixture gets).
#[test]
fn the_hot_oracle_separates_hot_from_cold() {
    let hot = hot8(HOT_R);
    let cold = hot8(0.5);
    assert!(hot.iter().all(|v| (200..255).contains(v)), "ACES(2) through the OETF is ≈ 230: {hot:?}");
    assert!(hot.iter().zip(&cold).all(|(h, c)| h.abs_diff(*c) > 1), "{hot:?} vs {cold:?}");
    assert_eq!(constant8(BACKGROUND), [(12, 13), (12, 13), (25, 26)]);
    assert_eq!(constant8([0.0, 1.0, 0.5]), [(0, 0), (255, 255), (127, 128)], "an integral product has one code");
}

/// A host [`Capture`] whose region is `n` copies of `texel`, with a `[config]` of `keys`.
fn synthetic_capture(texel: [u8; 3], n: usize, keys: &[(&str, u64)]) -> Capture {
    let mut text = String::from("[config]\n");
    for (k, v) in keys {
        text.push_str(&format!("{k} = {v}\n"));
    }
    Capture { region: vec![texel; n], receipt: Receipt::parse(&text).expect("invariant: a synthetic receipt parses") }
}

/// **A constant region is judged by the spec's conversion, not by round-half-up** (triage r1 F3):
/// Vulkan's float → UNORM conversion returns either integer nearest `f · 255` (exactly the integer
/// when the product is one). The RTX 3060 writes BACKGROUND (`[0.05, 0.05, 0.1]`, blue `0.1 · 255 =
/// 25.5000004`) as `[13, 13, 25]` (`px0/gl/fixtures.log`, the three constant drivers), which the
/// round-half-up oracle `[13, 13, 26]` refused; both are permitted, a code two steps off is not.
#[test]
fn a_constant_region_admits_both_spec_neighbours() {
    for name in ["deferred_base", "forward_sky_base", "forward_lit_clear"] {
        let f = fixture(name);
        for texel in [[13, 13, 25], [13, 13, 26], [12, 12, 25]] {
            let c = synthetic_capture(texel, 64, &[]);
            assert_eq!(check(f, false, &c), Ok(()), "{name}: {texel:?} is a permitted conversion of the background");
        }
        for texel in [[13, 13, 24], [14, 13, 25], [13, 11, 26]] {
            let c = synthetic_capture(texel, 64, &[]);
            assert!(check(f, false, &c).is_err(), "{name}: {texel:?} is no conversion of the background");
        }
    }
}

/// **A producer region showing only the background is empty, under the same conversion** (triage
/// r1 F3): the guard compared with round-half-up `[13, 13, 26]`, so on the RTX 3060 — whose
/// background is `[13, 13, 25]` — a producer fixture whose region showed nothing but background
/// passed it. One producer texel in the region clears the guard.
#[test]
fn a_background_only_producer_region_is_empty() {
    let f = fixture("forward_sky");
    let keys = [("path_is_forward", 1), ("light_sky", 1)];
    for texel in [[13, 13, 25], [13, 13, 26]] {
        let e = check(f, false, &synthetic_capture(texel, 64, &keys)).expect_err("a background-only region must red");
        assert!(e.contains("region empty"), "{texel:?}: {e}");
    }
    let mut lit = synthetic_capture([13, 13, 25], 64, &keys);
    lit.region[7] = [90, 120, 200];
    assert_eq!(check(f, false, &lit), Ok(()));
}

/// Every fixture name is unique and every lit-store producer has a hot fixture.
#[test]
fn every_lit_store_producer_has_a_hot_fixture() {
    for (i, f) in FIXTURES.iter().enumerate() {
        assert!(FIXTURES[i + 1..].iter().all(|g| g.name != f.name), "duplicate fixture {}", f.name);
    }
    for p in LIT_STORE {
        assert!(FIXTURES.iter().any(|f| f.name == p && f.hot != Hot::None), "{p} has no hot variant");
    }
}
