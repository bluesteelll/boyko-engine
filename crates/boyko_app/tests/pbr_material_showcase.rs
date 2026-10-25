//! Textured-PBR rung T7 — the OWNER-FACING material showcase: loads REAL PBR PNG
//! textures from disk (not procedurally generated, unlike T6c's `textured_smoke.rs`)
//! and applies them to a sphere so the owner can visually evaluate real materials.
//!
//! # Texture folder convention (READ THIS to know where to put files)
//!
//! The scene reads its texture set from a folder, resolved in this order:
//!
//!   1. `BOYKO_PBR_TEXTURE_DIR` env var, if set (an absolute path).
//!   2. Otherwise `crates/boyko_app/assets/pbr_test/` (this crate's `assets/pbr_test`,
//!      resolved relative to the crate manifest at compile time — see
//!      `assets/pbr_test/README.txt`, which documents this same convention for the
//!      owner).
//!
//! Every filename below is OPTIONAL — any subset may be present, in any
//! combination, and a missing/unreadable/undecodable file for a given slot falls
//! back to that channel's material default (never a panic; a one-line `eprintln!`
//! note explains what was skipped). See
//! [`boyko_render::load_material_folder`]'s own doc for the authoritative
//! candidate/color-space table (mirrored here for the owner):
//!
//!   - `albedo.png` (alias `base_color.png`) — sRGB color space.
//!   - `normal.png` — LINEAR color space, tangent-space normal map.
//!   - `metallic_roughness.png` (alias `mr.png`) — LINEAR color space, glTF ORM
//!     channel convention: G = roughness, B = metallic.
//!   - `ao.png` — LINEAR color space, R = occlusion.
//!   - `emissive.png` — sRGB color space.
//!
//! An empty folder renders a plain default-material sphere (white base color,
//! metallic 1.0, roughness 0.5); a partial set applies exactly the maps present.
//!
//! # Decode path
//!
//! Each file is decoded via [`PngTextureLoader::decode`](boyko_render::PngTextureLoader) —
//! the SAME in-house PNG decode path (`boyko_image::decode_png`, 8/16-bit
//! narrowing, RGBA expansion) a host-authored `.png` asset takes through the
//! loader registry — with the resulting `TextureData::color_space` overridden
//! per-slot (the loader itself has no per-material-slot context).
//!
//! # Eval knobs (env vars — see [`EvalKnobs::from_env`])
//!
//! | var | meaning | default |
//! |-----|---------|---------|
//! | `BOYKO_PBR_TEXTURE_DIR` | texture folder (see above) | `crates/boyko_app/assets/pbr_test/` |
//! | `BOYKO_SUN` | directional-light intensity | `5.5` |
//! | `BOYKO_JITTER=jx,jy` | sub-pixel camera pan, in pixels (rotated-grid supersampling) | `0,0` |
//! | `BOYKO_STAGE` | the stage preset: `studio` (dark floor 0.05, the ground term DERIVED from the floor incl. its specular sheen — [`studio_ground`]) \| `daylight` (the pre-IBL1 stage: floor 0.4, sky (1.05, 0.98, 0.88), ground (0.55, 0.52, 0.48)) | `studio` |
//! | `BOYKO_FLOOR=r,g,b` | overrides the preset's floor albedo (LINEAR) | the preset's |
//! | `BOYKO_SKY=r,g,b` / `BOYKO_GROUND=r,g,b` | override the preset's `SkyLight` lanes (LINEAR) | the preset's |
//! | `BOYKO_SLICES` | the sphere's longitude segments (latitude bands = 3/4 of it; verify F1's tessellation knob) | `64` |
//! | `BOYKO_IBL` | the environment: `studio` (the procedural studio, IBL1 — `EnvironmentConfig { Studio }` + an `EnvironmentLight`, no `SkyLight`) \| `off` (the stage's `SkyLight`) | `studio` |
//! | `BOYKO_IBL_INTENSITY` / `BOYKO_IBL_YAW` | the `EnvironmentLight`'s intensity and yaw (degrees) | `1.0` / `0` |
//! | `BOYKO_TONEMAP` | the tonemapper: `aces` \| `agx` \| `neutral` \| `reinhard`; unset ⇒ the ENGINE default (Khronos PBR Neutral since render wave PXt) | engine default |
//! | `BOYKO_WRAP` | diffuse terminator softening, `[0,1]` | `0.0` |
//! | `BOYKO_WIN` | window width/height in pixels (square) | `1280` |
//! | `BOYKO_CSM=off` | disables the CSM shadow-cascade insert | enabled (3 cascades) |
//! | `BOYKO_AA` | post-process AA mode (`fxaa` \| `smaa` \| `ssaa`) | off |
//! | `BOYKO_SSAO` | Render P7-Q2 SSAO quality (`low` \| `medium` \| `high`) | off |
//! | `BOYKO_HOST_DUMP=<path.bmp>` | HOST-LEVEL (`boyko_app::host_dump`): arms the screenshot capture | disabled |
//! | `BOYKO_DISABLE_VALIDATION=1` | withholds the validation layer in the backend (`validation_requested`, `boyko_rhi_vulkan`'s `device.rs`); `run_windowed` requests the layer only when `BOYKO_ENABLE_VALIDATION` is set, so this matters only alongside that variable | unset |
//!
//! Windowed-eval conventions (mirrors `textured_smoke.rs`/`pbr_showcase.rs`):
//! `#[ignore]` (needs a real windowed GPU device), run with
//! `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`;
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the screenshot capture.

#![cfg(windows)]

mod common;

use std::path::PathBuf;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{NonSendResMut, ResMut};
use boyko_render::{
    AaConfig, AaMode, BindlessTextureTable, LightingConfig, Material, MaterialGpu, SsaoConfig,
    SsaoQuality, TextureGpu, Tonemapper, load_material_folder,
};
use boyko_render::environment::{EnvSource, EnvironmentConfig, EnvironmentLight};

use common::{floor_plane, uv_sphere};

/// The sun direction TO the light (shared with the other showcase scenes).
const SUN_DIR: [f32; 3] = [-0.55, 0.30, 0.42];

/// The env var that overrides the default texture folder (see the module doc).
const TEXTURE_DIR_ENV: &str = "BOYKO_PBR_TEXTURE_DIR";
/// The default texture folder — `crates/boyko_app/assets/pbr_test/`, resolved
/// relative to this crate's manifest at compile time (repo-relative regardless of
/// the test binary's working directory).
const DEFAULT_TEXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pbr_test");

/// Owner-eval `BOYKO_*` env-var knobs this scene reads (see the module doc's
/// table), resolved once via [`Self::from_env`] (mirrors `HostDump::from_env`'s
/// idiom — `boyko_app::host_dump`). Each system that needs a knob calls
/// [`Self::from_env`] independently: env reads are cheap and setup-time-only, so
/// no `OnceLock`/cross-system threading is worth the complexity for a handful of
/// `env::var` calls.
struct EvalKnobs {
    /// `BOYKO_PBR_TEXTURE_DIR`, else the crate-relative default (module doc).
    texture_dir: PathBuf,
    /// `BOYKO_SUN` — the directional-light intensity (default `5.5`).
    sun_intensity: f32,
    /// `BOYKO_JITTER=jx,jy` — the sub-pixel camera pan, in pixels (default `(0.0, 0.0)`).
    jitter: (f32, f32),
    /// `BOYKO_TONEMAP` — an explicit tonemapper, or `None` (unset / unknown) for the engine default.
    tonemapper: Option<Tonemapper>,
    /// `BOYKO_STAGE` + `BOYKO_FLOOR` / `BOYKO_SKY` / `BOYKO_GROUND` — the resolved stage.
    stage: Stage,
    /// `BOYKO_SLICES` — the sphere's longitude segments (default `64`).
    slices: u32,
    /// `BOYKO_IBL` — `Some((intensity, yaw_radians))` arms the procedural studio environment
    /// (the default), `None` (`off`) keeps the stage's `SkyLight`.
    ibl: Option<(f32, f32)>,
    /// `BOYKO_WRAP` — the diffuse terminator softening, `[0, 1]` (default `0.0`).
    terminator_softening: f32,
    /// `BOYKO_WIN` — the window width/height in pixels (default `1280`).
    win: u32,
    /// `BOYKO_CSM=off` — disables the CSM shadow-cascade insert (default: enabled).
    csm_off: bool,
    /// `BOYKO_AA` — the post-process anti-aliasing mode (`fxaa` → [`AaMode::Fxaa`], `smaa` →
    /// [`AaMode::Smaa`], `ssaa` → [`AaMode::Ssaa`] — ALSO arms
    /// `EnginePlugins::with_ssaa_scale(2)` at the app-build call site below, since the mode
    /// alone cannot scale the boot-fixed render resolution — else [`AaMode::Off`], the
    /// default). The AA campaign visual-oracle knob: unset ⇒ Off ⇒ byte-identical to the
    /// no-AA dump.
    aa_mode: AaMode,
    /// `BOYKO_SSAO` — the Render P7-Q2 SSAO quality (`low` → [`SsaoQuality::Low`],
    /// `medium` → [`SsaoQuality::Medium`], `high` → [`SsaoQuality::High`] — else
    /// [`SsaoQuality::Off`], the default). The SSAO visual-oracle knob: unset ⇒ Off ⇒
    /// byte-identical to the no-SSAO dump.
    ssao_quality: SsaoQuality,
}

impl EvalKnobs {
    /// Resolves every owner-eval knob from its `BOYKO_*` env var, falling back to
    /// this scene's documented default when unset or unparsable.
    fn from_env() -> Self {
        let texture_dir = std::env::var(TEXTURE_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_TEXTURE_DIR));
        let sun_intensity =
            std::env::var("BOYKO_SUN").ok().and_then(|s| s.parse().ok()).unwrap_or(5.5);
        let jitter = std::env::var("BOYKO_JITTER")
            .ok()
            .and_then(|s| {
                let mut it = s.split(',');
                let a = it.next()?.trim().parse::<f32>().ok()?;
                let b = it.next()?.trim().parse::<f32>().ok()?;
                Some((a, b))
            })
            .unwrap_or((0.0, 0.0));
        let tonemapper = match std::env::var("BOYKO_TONEMAP").ok().as_deref() {
            Some("aces") => Some(Tonemapper::Aces),
            Some("agx") => Some(Tonemapper::AgX),
            Some("neutral") => Some(Tonemapper::Neutral),
            Some("reinhard") => Some(Tonemapper::ReinhardJodie),
            _ => None,
        };
        let stage = Stage::from_env(sun_intensity);
        let slices = std::env::var("BOYKO_SLICES").ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(64).max(8);
        let ibl = match std::env::var("BOYKO_IBL").ok().as_deref() {
            Some("off") => None,
            _ => {
                let intensity =
                    std::env::var("BOYKO_IBL_INTENSITY").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0);
                let yaw_deg = std::env::var("BOYKO_IBL_YAW").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                Some((intensity, yaw_deg.to_radians()))
            }
        };
        let terminator_softening =
            std::env::var("BOYKO_WRAP").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
        let win = std::env::var("BOYKO_WIN").ok().and_then(|s| s.parse().ok()).unwrap_or(1280);
        let csm_off = std::env::var("BOYKO_CSM").ok().as_deref() == Some("off");
        let aa_mode = match std::env::var("BOYKO_AA").ok().as_deref() {
            Some("fxaa") => AaMode::Fxaa,
            Some("smaa") => AaMode::Smaa,
            Some("ssaa") => AaMode::Ssaa,
            _ => AaMode::Off,
        };
        let ssao_quality = match std::env::var("BOYKO_SSAO").ok().as_deref() {
            Some("low") => SsaoQuality::Low,
            Some("medium") => SsaoQuality::Medium,
            Some("high") => SsaoQuality::High,
            _ => SsaoQuality::Off,
        };
        Self {
            texture_dir,
            sun_intensity,
            jitter,
            tonemapper,
            stage,
            slices,
            ibl,
            terminator_softening,
            win,
            csm_off,
            aa_mode,
            ssao_quality,
        }
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    dev: NonSendRes<GpuDevice>,
) {
    let knobs = EvalKnobs::from_env();
    println!("pbr_test: reading textures from {}", knobs.texture_dir.display());

    // ── The stage: a large matte non-textured floor at the preset's albedo (studio 0.05 by
    //    default — verify F1: a light floor reflected into a metal reads as ceramic).
    let f = knobs.stage.floor;
    let (fv, fi) = floor_plane(40.0, [f[0], f[1], f[2], 1.0]);
    let floor_mesh = meshes.register_mesh(dev.get(), &fv, &fi);
    let floor_mat = materials.add(Material::new([f[0], f[1], f[2], 1.0], 0.0, FLOOR_ROUGHNESS, 0.5, [0.0; 3], 0));
    let floor = commands
        .spawn(MeshBundle::new(floor_mesh, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))))
        .id();
    commands.entity(floor).insert(MaterialHandle::from_handle(floor_mat));

    // ── The textured sphere: one high-res mesh with a generated tangent basis.
    let (sv, si) = uv_sphere(1.0, knobs.slices * 3 / 4, knobs.slices, [1.0, 1.0, 1.0, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &sv, &si);

    // ── Load whichever texture slots the owner's folder provides (each independent,
    //    none required — see the module doc / `load_material_folder`'s own doc for
    //    the exact filename/alias/color-space convention).
    let material_textures =
        load_material_folder(&mut textures, dev.get(), &mut bindless, &knobs.texture_dir);
    println!(
        "pbr_test: resolved slots — albedo={} normal={} metal_rough={} ao={} emissive={} \
         (0 = fallback)",
        material_textures.albedo,
        material_textures.normal,
        material_textures.metal_rough,
        material_textures.ao,
        material_textures.emissive
    );

    // ── The textured material: base_color white (the albedo texture drives color
    //    when present), metallic=1/roughness=0.5 fallback (used verbatim when no
    //    metallic_roughness map is present), reflectance 0.5, no emissive fallback.
    let textured_mat = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 1.0, 0.5, 0.5, [0.0; 3], 0),
        material_textures,
    ));

    let e = commands
        .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(0.0, 1.05, 0.0))))
        .id();
    commands.entity(e).insert(MaterialHandle::from_handle(textured_mat));
    // Cast a grounding shadow onto the floor (like the Bevy reference). The floor is
    // receiver-only (no ShadowCaster); the sphere is the structural caster.
    commands.entity(e).insert(ShadowCaster);

    // ── A warm, bright sun at a grazing angle so any normal/roughness detail reads
    //    clearly in the specular response.
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        // DIAGNOSTIC: `BOYKO_SUN` overrides the sun intensity (default 5.5) — probes the
        // direct:ambient contrast ratio's role in the harsh bump-terminator islands.
        light: DirectionalLight::new(SUN_DIR, SUN_COLOR, knobs.sun_intensity),
    });

    // ── The ambient: by default (`BOYKO_IBL` unset) the procedural studio environment (IBL1 —
    //    the metal mirrors a room of emissive panels; the runner arms `EnvironmentConfig`, the
    //    app inserts it below) and NO `SkyLight` (the environment row is the one sky row). With
    //    `BOYKO_IBL=off`, the stage's `SkyLight`: a warm-neutral sky — NOT blue (a blue sky
    //    reflected off gold cancels its yellow tint → pale metal) — over the preset's ground,
    //    which on the studio stage is DERIVED from the dark floor ([`studio_ground`]), so the
    //    metal's lower half mirrors the floor it stands on.
    match knobs.ibl {
        Some((intensity, yaw)) => {
            commands.spawn(EnvironmentLight::new(intensity, yaw));
        }
        None => {
            commands.spawn(SkyLight::new(knobs.stage.sky, knobs.stage.ground));
        }
    }

    // ── The camera: close on the sphere so any texture detail is clearly legible.
    // DIAGNOSTIC: `BOYKO_JITTER=jx,jy` pans the view by a sub-pixel amount (in pixels)
    // along the camera's right/up axes — render several jittered frames and average them
    // offline for rotated-grid supersampling (a stand-in for real engine AA).
    let eye = Vec3::new(0.0, 1.6, 4.2);
    let target = Vec3::new(0.0, 1.0, 0.0);
    let up = Vec3::new(0.0, 1.0, 0.0);
    let (jx, jy) = knobs.jitter;
    let fwd = (target - eye).normalize();
    let right = fwd.cross(up).normalize();
    let up_cam = right.cross(fwd).normalize();
    // World units per pixel at the sphere's depth (isotropic square pixels; 1133-px height).
    let world_per_px = 2.0 * (45.0_f32 * 0.5).to_radians().tan() * (target - eye).length() / 1133.0;
    let off = right * (jx * world_per_px) + up_cam * (jy * world_per_px);
    let pose = Affine3A::look_at_rh(eye + off, target + off, up);
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 45.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 200.0,
        },
    });
}

/// Startup system: applies the owner-eval lighting knobs onto the plugin-inserted
/// [`LightingConfig`] — an explicit tonemapper (`BOYKO_TONEMAP`: `aces` / `agx` / `neutral` /
/// `reinhard`; unset keeps the engine default) AND the diffuse terminator softening
/// (`BOYKO_WRAP`, a float in `[0,1]`, default `0`).
/// Leaves every other field (exposure / sky / gates) at its default.
fn apply_eval_lighting_knobs(mut cfg: ResMut<LightingConfig>) {
    let knobs = EvalKnobs::from_env();
    // Unset ⇒ the engine default curve stays (Neutral since render wave PXt, ruling 116).
    if let Some(t) = knobs.tonemapper {
        cfg.tonemapper = t;
    }
    cfg.terminator_softening = knobs.terminator_softening;
}

/// **The owner-facing PBR material showcase dump.** Loads whatever PBR PNG texture
/// set the owner's folder provides (see the module doc for the convention) and
/// applies it to a sphere on a plain floor — the scene for judging real materials.
///
/// `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it on the GPU.
#[test]
#[ignore = "gpu-windowed: needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the owner's PBR material showcase screenshot"]
fn pbr_material_showcase_screenshot_dump() {
    let knobs = EvalKnobs::from_env();
    let mut app = App::new();
    // 1280² — higher than Bevy's 960 (its 768 window hi-DPI-scaled), so texture detail
    // reads at least as sharply as the reference.
    // DIAGNOSTIC: env-tunable render size (BOYKO_WIN) to test supersampling vs aliasing.
    let plugins = EnginePlugins::window("boyko_engine PBR material showcase", knobs.win, knobs.win);
    // SSAA (AA campaign Stage 3) is boot-fixed, host-authoritative: `knobs.aa_mode` alone
    // cannot scale the render resolution — the builder must request the 2× composite
    // extent BEFORE `WindowHost::boot`'s device-capability probe runs (see the module doc
    // above + `EvalKnobs::aa_mode`'s doc).
    let plugins =
        if knobs.aa_mode == AaMode::Ssaa { plugins.with_ssaa_scale(2) } else { plugins };
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Owner-eval knobs: `apply_eval_lighting_knobs` sets the resolve tonemapper
    // (`BOYKO_TONEMAP`) AND the diffuse terminator softening (`BOYKO_WRAP`) on the
    // plugin-inserted `LightingConfig` (leaves exposure / sky / gates intact).
    app.add_startup_system(apply_eval_lighting_knobs);
    // Enable CSM so the sphere casts a grounding shadow onto the floor (CsmPlugin's
    // default is DISABLED; inserted AFTER add_plugins to overwrite it — the room_smoke
    // convention). DIAGNOSTIC: `BOYKO_CSM=off` skips the insert (isolates the shadow
    // term's contribution to the grazing-band artifact).
    if !knobs.csm_off {
        app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    }
    // Owner-eval AA knob: `BOYKO_AA=fxaa` overrides the `AaPlugin`-inserted default
    // (`AaMode::Off`) to arm the FXAA post-process pass — the Stage-1 AA visual oracle.
    // Unset ⇒ Off ⇒ the same command stream / pixels as the no-AA dump.
    app.insert_resource(AaConfig { mode: knobs.aa_mode });
    // Render P7-Q2 owner-eval SSAO knob: `BOYKO_SSAO=low`/`medium`/`high` overrides the
    // `SsaoPlugin`-inserted default (`SsaoQuality::Off`) to arm the SSAO compute pass —
    // the live consumer's visual oracle. Unset ⇒ Off ⇒ the same command stream / pixels
    // as the no-SSAO dump.
    if knobs.ssao_quality != SsaoQuality::Off {
        app.insert_resource(SsaoConfig { quality: knobs.ssao_quality, ..Default::default() });
    }
    // IBL1 R1 (ruling 121 Q2): the showcase arms the procedural studio by default; the boot reads
    // this once (boot-frozen), so it is inserted before `run` — AFTER `add_plugins`, overwriting
    // `EnvironmentPlugin`'s `Off` (the `CsmConfig` precedent above).
    if knobs.ibl.is_some() {
        app.insert_resource(EnvironmentConfig { source: EnvSource::Studio });
    }
    app.run();
}

// ===============================================================================================
// The stage (IBL1 P3, ruling 118; verify F1, M6)
// ===============================================================================================

/// The floor's perceptual roughness (both presets).
const FLOOR_ROUGHNESS: f32 = 0.6;
/// The studio preset's floor albedo (verify F1: ~0.05).
const STUDIO_FLOOR: [f32; 3] = [0.05, 0.05, 0.05];
/// The warm sky tint both presets keep (July's reason: not blue, so the gold keeps its yellow).
const WARM_SKY: [f32; 3] = [1.05, 0.98, 0.88];
/// The daylight preset (the pre-IBL1 stage, kept as a knob): floor, sky, ground.
const DAYLIGHT: ([f32; 3], [f32; 3], [f32; 3]) = ([0.4, 0.4, 0.42], WARM_SKY, [0.55, 0.52, 0.48]);
/// The sun's colour (the `DirectionalLight` this scene spawns).
const SUN_COLOR: [f32; 3] = [1.0, 0.96, 0.88];

/// The resolved stage: floor albedo and the `SkyLight` lanes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stage {
    floor: [f32; 3],
    sky: [f32; 3],
    ground: [f32; 3],
}

/// `r,g,b` → three floats.
fn parse_rgb(v: &str) -> Option<[f32; 3]> {
    let mut it = v.split(',').map(|c| c.trim().parse::<f32>());
    let rgb = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
    it.next().is_none().then_some(rgb)
}

impl Stage {
    /// `BOYKO_STAGE` (default `studio`) with the `BOYKO_FLOOR` / `BOYKO_SKY` / `BOYKO_GROUND`
    /// overrides. The studio ground is derived from the (possibly overridden) floor and sky.
    fn from_env(sun_intensity: f32) -> Self {
        let rgb = |var: &str| std::env::var(var).ok().as_deref().and_then(parse_rgb);
        let daylight = std::env::var("BOYKO_STAGE").ok().as_deref() == Some("daylight");
        let (floor, sky) = if daylight { (DAYLIGHT.0, DAYLIGHT.1) } else { (STUDIO_FLOOR, WARM_SKY) };
        let floor = rgb("BOYKO_FLOOR").unwrap_or(floor);
        let sky = rgb("BOYKO_SKY").unwrap_or(sky);
        let derived = if daylight { DAYLIGHT.2 } else { studio_ground(floor, sky, sun_intensity, sun_up()) };
        let ground = rgb("BOYKO_GROUND").unwrap_or(derived);
        Stage { floor, sky, ground }
    }
}

/// The sun's up component (`SUN_DIR` normalized, `.y` ≈ 0.398).
fn sun_up() -> f32 {
    let [x, y, z] = SUN_DIR;
    y / (x * x + y * y + z * z).sqrt()
}

/// Karis' analytic environment BRDF (no DFG LUT) — the `(scale, bias)` of the split sum,
/// transcribed from `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl`'s `env_brdf_approx`
/// (OPEN-ITEMS proposal PC-8(c): retired with the DFG LUT, like its six shader copies).
fn env_brdf_approx(roughness: f32, nov: f32) -> [f32; 2] {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r: [f32; 4] = core::array::from_fn(|i| roughness * c0[i] + c1[i]);
    let a004 = (r[0] * r[0]).min((-9.28 * nov).exp2()) * r[0] + r[1];
    [-1.04 * a004 + r[2], 1.04 * a004 + r[3]]
}

/// The studio stage's `SkyLight` ground lane, DERIVED from the floor (verify M6's correction): the
/// radiance an up-facing hemisphere sees from a floor of albedo `floor` lit by the sky and the sun —
/// INCLUDING the floor's own ambient specular sheen, which reflects the ground term itself, so the
/// lane is the fixed point
///
/// `g = (ρ·sky + ρ·E·sun_y/π·sun_color + s·ec·w·sky) / (1 − s·ec·(1 − w))`
///
/// with `s = f0·A + B` the floor's split-sum scale at `(roughness 0.6, NoV 0.3)` (`f0` 0.04,
/// reflectance 0.5), `ec` its energy compensation, and `w = smoothstep(0.65)` the engine's
/// reflected-hemisphere weight at `R.y = 0.3`. The sun's direct specular on the floor is
/// view-dependent and omitted. For ρ = 0.05 under the showcase sun this is ≈ 0.15 × the sky tint
/// (the diffuse-only derivation gave 0.088 — the floor's sheen is half the lane).
fn studio_ground(floor: [f32; 3], sky: [f32; 3], sun_intensity: f32, sun_y: f32) -> [f32; 3] {
    let f0 = 0.04;
    let [a, b] = env_brdf_approx(FLOOR_ROUGHNESS, 0.3);
    let s = f0 * a + b;
    let ec = 1.0 + f0 * (1.0 / (a + b).max(1e-4) - 1.0);
    let t = 0.65f32;
    let w = t * t * (3.0 - 2.0 * t);
    core::array::from_fn(|c| {
        let diffuse = floor[c] * sky[c] + floor[c] * sun_intensity * SUN_COLOR[c] * sun_y / core::f32::consts::PI;
        (diffuse + s * ec * w * sky[c]) / (1.0 - s * ec * (1.0 - w))
    })
}

/// IBL1 P3 (C13): the derived ground's closed form — at ρ = 0 the lane is the sheen's fixed point
/// alone, and at the studio's ρ = 0.05 under the showcase sun every channel lands in verify M6's
/// 0.13–0.18 × the sky tint. Dropping the sheen term gives 0.083 × (verify M6's 0.087 class): red.
#[test]
fn studio_ground_closed_form() {
    let [a, b] = env_brdf_approx(FLOOR_ROUGHNESS, 0.3);
    let s = 0.04 * a + b;
    let ec = 1.0 + 0.04 * (1.0 / (a + b) - 1.0);
    let w = 0.65f32 * 0.65 * (3.0 - 2.0 * 0.65);
    let g0 = studio_ground([0.0; 3], WARM_SKY, 5.5, sun_up());
    for c in 0..3 {
        let want = s * ec * w * WARM_SKY[c] / (1.0 - s * ec * (1.0 - w));
        assert!((g0[c] - want).abs() <= 1e-6, "rho 0, channel {c}: {} vs {want}", g0[c]);
    }
    let g = studio_ground(STUDIO_FLOOR, WARM_SKY, 5.5, sun_up());
    for c in 0..3 {
        let ratio = g[c] / WARM_SKY[c];
        assert!((0.13..=0.18).contains(&ratio), "rho 0.05, channel {c}: ground/sky = {ratio} (want 0.13..0.18)");
    }
    assert!((sun_up() - 0.398).abs() < 1e-3, "the showcase sun's up component");
}
