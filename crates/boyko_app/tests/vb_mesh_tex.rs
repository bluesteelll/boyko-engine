//! Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3 / `docs/VB-P2-CLASSIFICATION-PLAN.md`) —
//! the VisibilityBuffer TEXTURED golden dump: real PBR PNG textures shaded through the
//! material-classified `vb_shade_tex.comp.spv` pipeline.
//!
//! Clones [`vb_mesh.rs`]'s five-sphere scene verbatim (SAME mesh, SAME camera/sun/sky), with ONE
//! delta: four of the five spheres carry a REAL textured material loaded from
//! `crates/boyko_app/assets/pbr_fixtures/synth_bumps` (the committed in-repo oracle texture set
//! — `pbr_fixtures/README.md`'s own doc) instead of a flat color. Sharing ONE textured material
//! across four instances exercises the classified pipeline's core invariant: every pixel in a
//! `vb_shade_tex` group shares the SAME uniform bindless-texture index
//! (`docs/VB-P2-CLASSIFICATION-PLAN.md`'s P2b debug-assert), regardless of which of the four
//! instances a given group's pixels came from.
//!
//! Textured materials force the VB selector (`GBufferScene::vb_use_classified`'s own doc) onto
//! the classified `vb_shade`/`vb_shade_tex` pipeline automatically (`GBufferScene::vb_tex_active`)
//! — no `BOYKO_VB_FORCE_CLASSIFIED` env var needed.
//!
//! Windowed-test conventions (mirrors `vb_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_mesh_tex]` pin (UNBLESSED — the orchestrator renders + blesses
//! after visually comparing against the Deferred textured showcase).

#![cfg(windows)]

use std::path::PathBuf;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{
    AaConfig, AaMode, BindlessTextureTable, GeometryLegs, Material, MaterialGpu, MeshAssetsVbExt,
    MeshGeometryTableSlot, RenderPath, RenderPathConfig, TextureGpu, load_material_folder,
};

/// The sun direction TO the light (byte-identical to `vb_mesh.rs`'s / `grand_showcase_2mat.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// The env var that overrides the default texture folder (mirrors
/// `pbr_material_showcase.rs`'s `BOYKO_PBR_TEXTURE_DIR` convention).
const TEXTURE_DIR_ENV: &str = "BOYKO_PBR_TEXTURE_DIR";
/// The default texture folder — the committed `synth_bumps` oracle set (the normal-map
/// GREEN-CHANNEL CONVENTION fixture, `pbr_fixtures/README.md`'s own doc), resolved relative to
/// this crate's manifest at compile time (repo-relative regardless of the test binary's working
/// directory).
const DEFAULT_TEXTURE_DIR: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pbr_fixtures/synth_bumps");

/// Verbatim copy of `grand_showcase_2mat.rs::uv_sphere` / `vb_mesh.rs::uv_sphere` (see that
/// file's NOTE for why this is a local copy rather than a shared `tests/common` helper).
/// `generate_tangents` bakes the tangent basis TV0's `vb_geom_fetch.hlsli` `TEXTURED` arm reads.
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi; // 0..π, north pole to south
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32; // phi / π
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi); // 0..2π
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st]; // unit outward normal
            let u = j as f32 / slices as f32; // theta / 2π
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, color);
            vertex.uv = [u, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// The five-sphere scene (`vb_mesh.rs::setup`'s layout, textured materials swapped in for the
/// four non-default spheres) — the sphere mesh is registered via
/// [`MeshAssetsVbExt::register_mesh_vb`] (falling back to the plain
/// [`MeshAssetsExt::register_mesh`](boyko_render::MeshAssetsExt::register_mesh) when the
/// geometry table is not armed), and the SAME textured material is shared by four instances
/// (the multi-instance-same-material classified-group coverage this rung's doc explains).
///
/// **The golden's scene** — [`vb_mesh_tex_screenshot_dump`] pins its bytes, so this system's
/// output must never drift. It is a thin `Solo::No` forward to [`build_scene`]; the solo
/// close-up variant is a SEPARATE system ([`setup_solo`]) precisely so no eval-only knob can
/// reach this path.
fn setup(
    commands: Commands,
    meshes: NonSendResMut<Assets<MeshGpu>>,
    materials: ResMut<Assets<Material>>,
    textures: NonSendResMut<Assets<TextureGpu>>,
    bindless: NonSendResMut<BindlessTextureTable>,
    geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    build_scene(Solo::No, commands, meshes, materials, textures, bindless, geo_table, dev);
}

/// **The solo close-up scene** (owner-facing eval only, never a golden): ONE textured sphere
/// filling the frame, so normal-map relief is judged on real texels instead of a 90-pixel
/// thumbnail. Same mesh / material / sun / sky as [`setup`] — only the instance count and the
/// camera distance differ.
fn setup_solo(
    commands: Commands,
    meshes: NonSendResMut<Assets<MeshGpu>>,
    materials: ResMut<Assets<Material>>,
    textures: NonSendResMut<Assets<TextureGpu>>,
    bindless: NonSendResMut<BindlessTextureTable>,
    geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    build_scene(Solo::Yes, commands, meshes, materials, textures, bindless, geo_table, dev);
}

/// Selects [`build_scene`]'s instance layout + camera framing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Solo {
    /// The golden's five-sphere row, camera pulled back to frame all five.
    No,
    /// One textured sphere, camera pulled in to fill the frame (eval only).
    Yes,
}

// The seven params after `solo` ARE `setup`/`setup_solo`'s system signature, threaded through
// verbatim so the two layouts share one scene body. Bundling them behind a struct would hide the
// declarative signature the ECS reads (the same reasoning `csm_caster::gather_shadow_casters`
// gives for its own allow).
#[allow(clippy::too_many_arguments)]
fn build_scene(
    solo: Solo,
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let texture_dir = std::env::var(TEXTURE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_TEXTURE_DIR));
    println!("vb_mesh_tex: reading textures from {}", texture_dir.display());
    let material_textures =
        load_material_folder(&mut textures, dev.get(), &mut bindless, &texture_dir);
    println!(
        "vb_mesh_tex: resolved slots — albedo={} normal={} metal_rough={} ao={} emissive={} \
         (0 = fallback)",
        material_textures.albedo,
        material_textures.normal,
        material_textures.metal_rough,
        material_textures.ao,
        material_textures.emissive
    );
    let textured_mat = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0),
        material_textures,
    ));

    let spacing = 1.55;
    // The middle sphere (index 2) stays the DEFAULT (untextured) material — a visible contrast
    // baseline against its four textured neighbors, mirroring `vb_mesh.rs`'s "one default + four
    // materials" layout shape. `Solo::Yes` keeps only that row's first (textured) sphere, moved
    // to x=0 so the eval camera frames it head-on.
    let materials_row: [Option<u16>; 5] = [
        Some(textured_mat.index() as u16),
        Some(textured_mat.index() as u16),
        None,
        Some(textured_mat.index() as u16),
        Some(textured_mat.index() as u16),
    ];
    let count = if solo == Solo::Yes { 1 } else { materials_row.len() };
    for (i, mat) in materials_row.iter().take(count).enumerate() {
        let x = if solo == Solo::Yes { 0.0 } else { (i as f32 - 2.0) * spacing };
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.1),
    });

    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    // `Solo::Yes` frames the single sphere head-on at ~2.1 units: with radius 0.62 and the 52°
    // vertical fov below, the sphere subtends ~35° and fills ~2/3 of the frame height — the point
    // of the solo dump (relief judged on real texels, not on a ~90-px thumbnail).
    let (eye, target) = match solo {
        Solo::Yes => (Vec3::new(0.0, 0.6, 2.1), Vec3::new(0.0, 0.6, 0.0)),
        Solo::No => (Vec3::new(0.0, 1.1, 7.8), Vec3::new(0.0, 0.55, 0.0)),
    };
    let pose = Affine3A::look_at_rh(eye, target, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// **The VisibilityBuffer TEXTURED golden dump.** The `vb_mesh.rs` five-sphere scene with a real
/// textured material shared across four instances, rendered through `RenderPath::VisibilityBuffer
/// × GeometryLegs::Mesh` — the owner's RTX visual sign-off gate for rung TV0
/// (`goldens/PINS.toml`'s `[vb_mesh_tex]`, blessed against the Deferred textured showcase).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the VisibilityBuffer TEXTURED screenshot"]
fn vb_mesh_tex_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb mesh textured", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R8 / TV0: request `VisibilityBuffer × Mesh` —
    // inserted AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so
    // this override wins, mirroring `vb_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    app.run();
}

/// **TEXTURED anti-aliasing / render-path eval dump** (owner-facing, NOT a golden). The SAME
/// five-sphere textured scene as [`vb_mesh_tex_screenshot_dump`] (reuses [`setup`]), rendered with
/// the post-process / supersample AA mode selected by `BOYKO_AA` (`fxaa` → [`AaMode::Fxaa`],
/// `smaa` → [`AaMode::Smaa`], `ssaa` → [`AaMode::Ssaa`] + `EnginePlugins::with_ssaa_scale(2)`,
/// else [`AaMode::Off`]) at a `BOYKO_WIN`² window (default `640`). Its purpose is to answer
/// whether the AA resolve chain applies under VB: at this harness's writing TAA was still capped
/// off under VB (that cap has since been DELETED — TAA now runs on every VB leg set, the
/// `[vb_taa]`/`[vb_both_taa]` pins), but FXAA/SMAA/SSAA were never in the cap, so
/// this dump reveals whether they actually run on the VB `lit` output.
///
/// `BOYKO_PATH=deferred` renders the identical scene through `RenderPath::Deferred` instead — the
/// TV0 bless reference. `[vb_mesh_tex]`'s parity check is VB-vs-Deferred *on the same scene*, and
/// this fn is the only harness that can produce both halves: the launcher's own
/// `BOYKO_RENDER_PATH` seam is applied inside `EnginePlugins::build()`, so the post-`add_plugins`
/// `insert_resource(RenderPathConfig)` below would override it (per that seam's documented
/// precedence). Reading the path here, before the insert, is what keeps the two dumps
/// scene-identical.
///
/// `BOYKO_SOLO=1` swaps [`setup`] for [`setup_solo`]: ONE sphere filling the frame instead of the
/// five-sphere row, for judging normal-map relief at a useful pixel count.
///
/// `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it on the GPU. Set
/// `BOYKO_PBR_TEXTURE_DIR` to a real material pack (e.g. `assets/materials/alley-brick-wall/pbr`)
/// so the AA effect reads on high-frequency albedo/normal detail; `BOYKO_HOST_DUMP=<path.bmp>`
/// arms the capture.
#[test]
#[ignore = "needs a real windowed GPU device; orchestrator-run textured AA / render-path eval dump"]
fn vb_mesh_tex_aa_screenshot_dump() {
    let win: u32 = std::env::var("BOYKO_WIN").ok().and_then(|s| s.parse().ok()).unwrap_or(640);
    let aa_mode = match std::env::var("BOYKO_AA").ok().as_deref() {
        Some("fxaa") => AaMode::Fxaa,
        Some("smaa") => AaMode::Smaa,
        Some("ssaa") => AaMode::Ssaa,
        _ => AaMode::Off,
    };
    let path = match std::env::var("BOYKO_PATH").ok().as_deref() {
        Some("deferred") => RenderPath::Deferred,
        _ => RenderPath::VisibilityBuffer,
    };
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine mesh textured AA", win, win);
    // SSAA is a boot render-scale commitment (mirrors `pbr_material_showcase.rs`): the 2× extent
    // must be requested BEFORE `WindowHost::boot`'s device probe, so `aa_mode` alone can't arm it.
    let plugins = if aa_mode == AaMode::Ssaa { plugins.with_ssaa_scale(2) } else { plugins };
    app.add_plugins(plugins);
    if std::env::var("BOYKO_SOLO").is_ok() {
        app.add_startup_system(setup_solo);
    } else {
        app.add_startup_system(setup);
    }
    app.insert_resource(RenderPathConfig { path, legs: GeometryLegs::Mesh });
    app.insert_resource(AaConfig { mode: aa_mode });
    app.run();
}
