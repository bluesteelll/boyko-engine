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
//! | `BOYKO_TONEMAP` | resolve tonemapper (`neutral` \| `reinhard` \| else ACES) | ACES |
//! | `BOYKO_WRAP` | diffuse terminator softening, `[0,1]` | `0.0` |
//! | `BOYKO_WIN` | window width/height in pixels (square) | `1280` |
//! | `BOYKO_CSM=off` | disables the CSM shadow-cascade insert | enabled (3 cascades) |
//! | `BOYKO_HOST_DUMP=<path.bmp>` | HOST-LEVEL (`boyko_app::host_dump`): arms the screenshot capture | disabled |
//! | `BOYKO_DISABLE_VALIDATION=1` | operator convention — NOT read by `run_windowed` (which hardcodes `enable_validation: false` already); kept for uniformity with `boyko_rhi_vulkan`/`boyko_render` test harnesses that DO read it | n/a |
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
    AaConfig, AaMode, BindlessTextureTable, LightingConfig, Material, MaterialGpu, TextureGpu,
    Tonemapper, load_material_folder,
};

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
    /// `BOYKO_TONEMAP` — the resolve tonemapper (`neutral` | `reinhard` | else ACES).
    tonemapper: Tonemapper,
    /// `BOYKO_WRAP` — the diffuse terminator softening, `[0, 1]` (default `0.0`).
    terminator_softening: f32,
    /// `BOYKO_WIN` — the window width/height in pixels (default `1280`).
    win: u32,
    /// `BOYKO_CSM=off` — disables the CSM shadow-cascade insert (default: enabled).
    csm_off: bool,
    /// `BOYKO_AA` — the post-process anti-aliasing mode (`fxaa` → [`AaMode::Fxaa`], else
    /// [`AaMode::Off`], the default). The Stage-1 AA visual-oracle knob: unset ⇒ Off ⇒
    /// byte-identical to the no-AA dump.
    aa_mode: AaMode,
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
            Some("neutral") => Tonemapper::Neutral,
            Some("reinhard") => Tonemapper::ReinhardJodie,
            _ => Tonemapper::Aces,
        };
        let terminator_softening =
            std::env::var("BOYKO_WRAP").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
        let win = std::env::var("BOYKO_WIN").ok().and_then(|s| s.parse().ok()).unwrap_or(1280);
        let csm_off = std::env::var("BOYKO_CSM").ok().as_deref() == Some("off");
        let aa_mode = match std::env::var("BOYKO_AA").ok().as_deref() {
            Some("fxaa") => AaMode::Fxaa,
            _ => AaMode::Off,
        };
        Self {
            texture_dir,
            sun_intensity,
            jitter,
            tonemapper,
            terminator_softening,
            win,
            csm_off,
            aa_mode,
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

    // ── The environment: a large matte non-textured floor (the default material).
    let (fv, fi) = floor_plane(40.0, [0.4, 0.4, 0.42, 1.0]);
    let floor_mesh = meshes.register_mesh(dev.get(), &fv, &fi);
    let floor_mat = materials.add(Material::new([0.4, 0.4, 0.42, 1.0], 0.0, 0.6, 0.5, [0.0; 3], 0));
    let floor = commands
        .spawn(MeshBundle::new(floor_mesh, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))))
        .id();
    commands.entity(floor).insert(MaterialHandle::from_handle(floor_mat));

    // ── The textured sphere: one high-res mesh with a generated tangent basis.
    let (sv, si) = uv_sphere(1.0, 48, 64, [1.0, 1.0, 1.0, 1.0]);
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
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.88], knobs.sun_intensity),
    });

    // ── Warm-neutral sky: a MODERATE ambient fill (enough to keep shadowed/downward-
    //    facing areas from crushing to black, but dim enough that the strong key still
    //    yields deep relief) over a dark-warm ground. NOT blue (a blue sky reflected off
    //    gold cancels its yellow tint → pale metal). Balances the Bevy-like key:ambient.
    commands.spawn(SkyLight::new([1.05, 0.98, 0.88], [0.55, 0.52, 0.48]));

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
/// [`LightingConfig`] — the resolve tonemapper (`BOYKO_TONEMAP`: `neutral` →
/// Khronos PBR Neutral, `reinhard` → Reinhard-Jodie, anything else → ACES) AND the
/// diffuse terminator softening (`BOYKO_WRAP`, a float in `[0,1]`, default `0`).
/// Leaves every other field (exposure / sky / gates) at its default.
fn apply_eval_lighting_knobs(mut cfg: ResMut<LightingConfig>) {
    let knobs = EvalKnobs::from_env();
    cfg.tonemapper = knobs.tonemapper;
    cfg.terminator_softening = knobs.terminator_softening;
}

/// **The owner-facing PBR material showcase dump.** Loads whatever PBR PNG texture
/// set the owner's folder provides (see the module doc for the convention) and
/// applies it to a sphere on a plain floor — the scene for judging real materials.
///
/// `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it on the GPU.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the owner's PBR material showcase screenshot"]
fn pbr_material_showcase_screenshot_dump() {
    let knobs = EvalKnobs::from_env();
    let mut app = App::new();
    // 1280² — higher than Bevy's 960 (its 768 window hi-DPI-scaled), so texture detail
    // reads at least as sharply as the reference.
    // DIAGNOSTIC: env-tunable render size (BOYKO_WIN) to test supersampling vs aliasing.
    app.add_plugins(EnginePlugins::window("boyko_engine PBR material showcase", knobs.win, knobs.win));
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
    app.run();
}
