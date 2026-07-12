//! Textured-PBR rung T6c — the procedural-texture RENDER SMOKE.
//!
//! Proves the textured gbuffer pipeline samples end-to-end: a floor + one sphere carry a
//! PROCEDURALLY-GENERATED texture set (no external asset files — an in-code checkerboard
//! albedo, a tangent-space "quilted bump" normal map, and a roughness-gradient
//! metallic-roughness map), uploaded through `Assets<TextureGpu>` +
//! `BindlessTextureTable` and bound to the sphere's material via
//! `Material::with_textures`. A visible checkerboard albedo + normal-perturbed specular +
//! a left-to-right roughness sweep is the visual proof the TEXTURED `#ifdef` axis
//! (`gbuffer_mrt_tex.{vs,fs}`) is sampling real bindless textures, not falling back to
//! scalar `base_color`/`mrr`.
//!
//! This is SEPARATE from T7 (the owner's real texture assets) — a delegated-oracle smoke
//! scene the orchestrator judges visually, NOT byte-pinned in `goldens/PINS.toml`.
//!
//! Windowed-eval conventions (mirrors `pbr_showcase.rs`): `#[ignore]` (needs a real
//! windowed GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`;
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the screenshot capture.

#![cfg(windows)]

mod common;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{NonSendResMut, ResMut};
use boyko_render::{
    BindlessTextureTable, ColorSpace, Material, MaterialGpu, MaterialTextures, TextureAssetsExt,
    TextureData, TextureGpu,
};

use common::{floor_plane, uv_sphere};

/// The sun direction TO the light (shared with the other showcase scenes).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Procedural texture extent (a modest power-of-two — plenty to show the checker/bump/
/// gradient patterns at the sphere's on-screen size, no mip-chain needed for the proof).
const TEX_SIZE: u32 = 64;
/// Checkerboard cell size in texels (`TEX_SIZE / CELL` cells per axis).
const CELL: u32 = 8;

/// A `size`x`size` sRGB checkerboard albedo (2 distinct saturated colors, `CELL`-texel
/// cells) — the visible proof gAlbedo is sampling a REAL texture, not `base_color`.
fn checkerboard_albedo(size: u32, cell: u32) -> Vec<u8> {
    const COLOR_A: [u8; 4] = [235, 60, 40, 255]; // warm red
    const COLOR_B: [u8; 4] = [40, 120, 235, 255]; // cool blue
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let parity = (x / cell + y / cell) % 2;
            let c = if parity == 0 { COLOR_A } else { COLOR_B };
            out.extend_from_slice(&c);
        }
    }
    out
}

/// A `size`x`size` tangent-space "quilted bump" normal map (LINEAR color space): each
/// `CELL`-texel checker cell tilts the packed normal toward one of 4 diagonal directions
/// (by 2x2 parity), so the lit sphere shows a visible faceted/bump specular pattern
/// instead of a smooth highlight — the proof gNormal's tangent-space normal-mapping
/// block (world_T/Gram-Schmidt/TBN rotate) is live.
fn bump_normal_map(size: u32, cell: u32) -> Vec<u8> {
    let pack = |v: f32| ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u8;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let cx = (x / cell) % 2;
            let cy = (y / cell) % 2;
            let (nx, ny): (f32, f32) = match (cx, cy) {
                (0, 0) => (0.5, 0.5),
                (1, 0) => (-0.5, 0.5),
                (0, 1) => (0.5, -0.5),
                _ => (-0.5, -0.5),
            };
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            out.extend_from_slice(&[pack(nx), pack(ny), pack(nz), 255]);
        }
    }
    out
}

/// A `size`x`size` metallic-roughness map (LINEAR color space, glTF channel convention —
/// metallic = B, roughness = G): fully metallic, with roughness sweeping 0.05..1.0
/// left-to-right — the proof gPbr's per-pixel metallic/roughness (sampled, not the
/// material's scalar fallback) reaches the deferred resolve's Cook-Torrance BRDF.
fn roughness_gradient_mr(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for _y in 0..size {
        for x in 0..size {
            let u = x as f32 / (size - 1).max(1) as f32;
            let roughness = 0.05 + u * 0.95;
            let g = (roughness.clamp(0.0, 1.0) * 255.0).round() as u8;
            out.extend_from_slice(&[0, g, 255, 255]);
        }
    }
    out
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    dev: NonSendRes<GpuDevice>,
) {
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

    // ── The procedural texture set: generate → upload → register into the bindless
    //    table → resolve the returned handles' bindless slots.
    let albedo_data = TextureData {
        width: TEX_SIZE,
        height: TEX_SIZE,
        rgba8: checkerboard_albedo(TEX_SIZE, CELL),
        color_space: ColorSpace::Srgb,
    };
    let normal_data = TextureData {
        width: TEX_SIZE,
        height: TEX_SIZE,
        rgba8: bump_normal_map(TEX_SIZE, CELL),
        color_space: ColorSpace::Linear,
    };
    let mr_data = TextureData {
        width: TEX_SIZE,
        height: TEX_SIZE,
        rgba8: roughness_gradient_mr(TEX_SIZE),
        color_space: ColorSpace::Linear,
    };

    let albedo_handle = textures.register_texture(dev.get(), &mut bindless, &albedo_data);
    let normal_handle = textures.register_texture(dev.get(), &mut bindless, &normal_data);
    let mr_handle = textures.register_texture(dev.get(), &mut bindless, &mr_data);

    let albedo_slot = textures.texture(albedo_handle).bindless_slot;
    let normal_slot = textures.texture(normal_handle).bindless_slot;
    let mr_slot = textures.texture(mr_handle).bindless_slot;

    // ── The textured material: base_color white (the albedo texture drives color
    //    entirely), metallic=1/roughness=0.5 fallback (unread — the mr texture always
    //    overrides both channels here), reflectance 0.5, no emissive.
    let textured_mat = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 1.0, 0.5, 0.5, [0.0; 3], 0),
        MaterialTextures {
            albedo: albedo_slot,
            normal: normal_slot,
            metal_rough: mr_slot,
            ao: 0,
            emissive: 0,
        },
    ));

    let e = commands
        .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(0.0, 1.05, 0.0))))
        .id();
    commands.entity(e).insert(MaterialHandle::from_handle(textured_mat));

    // ── A warm, bright sun at a grazing angle so the bump normal map + roughness sweep
    //    both read clearly in the specular response.
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.88], 5.0),
    });

    // ── A bright, contrasty sky (the metal ball's environment response).
    commands.spawn(SkyLight::new([0.85, 1.05, 1.7], [0.04, 0.04, 0.05]));

    // ── The camera: close on the sphere so the checker/bump/gradient patterns are
    //    clearly legible.
    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.6, 4.2),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
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

/// **The procedural-texture smoke dump.** A checkerboard-albedo, bump-normal-mapped,
/// roughness-swept sphere on a plain floor — the scene for judging whether the TEXTURED
/// gbuffer pipeline actually samples bindless textures end-to-end.
///
/// `#[ignore]`: needs a real windowed GPU device; the orchestrator runs it on the GPU.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the textured-PBR smoke screenshot"]
fn textured_smoke_screenshot_dump() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine textured-PBR smoke", 640, 640));
    app.add_startup_system(setup);
    app.run();
}
