//! Multi-paradigm render-path plan, rung R5 — the ForwardPlus v1 mesh-only golden dump.
//!
//! Clones [`forward_mesh.rs`](../forward_mesh.rs) verbatim, with ONE delta: requests
//! `RenderPath::ForwardPlus` instead of `RenderPath::Forward`. Boots through the REAL,
//! PRODUCTION `boyko_app::runner`/`gpu_scene::GpuSceneBundles` path (the `grand_showcase_2mat`
//! pattern), which has genuine ForwardPlus wiring as of rung R5:
//! `GpuSceneBundles::boot`'s unconditional `forward_prepass_pipeline`/`forward_plus_layout0`/
//! `forward_plus_pipeline` creation, `GpuSceneBundles::scene`'s conditional
//! `forward_pipeline`/`forward_layout0` selection, and `declare_forward_graph`/`record_forward`'s
//! `depth_prepass`/`light_cull` passes (`boyko_rhi_vulkan::present`).
//!
//! # Expected pixel output
//!
//! This scene arms NO L1 clustered-cull machinery (`boyko_app::gpu_scene`'s scene-assembly seam
//! has never wired `GBufferScene::cluster_cull`/`cluster_grid`/`light_index`/`light_index_alloc`
//! to `Some` — that is a SEPARATE, unlanded L1-app-integration rung, out of R5's scope; see the
//! R5 developer report), so `light_cull`'s "4-buffers-Some" gate never fires: the froxel FS
//! variant's `ClusterGrid`/`LightIndexList` bindings are bound-but-unread placeholders
//! (`scene.light_table`, `ForwardTargets::build`'s doc), and `forward_opaque_froxel.fs.hlsl`'s
//! runtime `clusters_enabled` header bit reads `0` — the SAME L1 0%-gate the deferred resolve
//! uses — so the froxel FS takes the IDENTICAL flat-block branch `forward_opaque.fs.hlsl`'s base
//! (non-froxel) compile always takes. The ONLY behavioral difference from `forward_mesh` is
//! therefore the depth prepass (EARLY-Z zero-overdraw, Decision 4) + the `EQUAL`-depth
//! `forward_opaque` test — floating-point evaluation ORDER may differ (a `GREATER`+write pass
//! followed by an `EQUAL`+no-write pass vs. one `GREATER`+write pass), but the LIT pixel color
//! math is token-for-token the SAME shared body (`forward_opaque.fs.hlsl`'s point/spot loop is
//! identical whether or not `-D FROXEL=1`), so the dumped BMP should be pixel-identical (or, at
//! worst, differ by isolated ULP-level FP reordering noise) to `forward_mesh`'s
//! `f93b5aad9f799626e6d4abf0dd06d8596d7c6f4c6e9a758a3a6d202d22de71ad` — see
//! `goldens/PINS.toml`'s `[forwardplus_mesh]` pin, UNBLESSED pending the owner's real-GPU visual
//! sign-off (the same discipline every new pin in this file follows).
//!
//! Windowed-test conventions (mirrors `forward_mesh.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`. `BOYKO_HOST_DUMP=
//! <path.bmp>` arms the `boyko_app::host_dump` screenshot capture.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s /
/// `forward_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Verbatim copy of `forward_mesh.rs::uv_sphere` (itself a verbatim copy of
/// `grand_showcase_2mat.rs::uv_sphere`) — see that file's NOTE for why this is a local copy
/// rather than a shared `tests/common` helper (a pinned-golden scene keeps its exact mesh
/// generation frozen).
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

/// Verbatim copy of `forward_mesh.rs::setup` — the SAME five-sphere scene, so the dumped BMP is a
/// direct ForwardPlus-vs-Forward-vs-Deferred visual comparator.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = meshes.register_mesh(dev.get(), &verts, &idx);

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let spacing = 1.55;
    let materials_row: [Option<u16>; 5] =
        [None, Some(red.index() as u16), Some(green.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in materials_row.iter().enumerate() {
        let x = (i as f32 - 2.0) * spacing;
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

    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.1, 7.8),
        Vec3::new(0.0, 0.55, 0.0),
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
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// **The ForwardPlus v1 mesh-only golden dump.** The SAME five-sphere scene
/// [`forward_mesh_screenshot_dump`](../forward_mesh.rs) renders, through
/// `RenderPath::ForwardPlus × GeometryLegs::Mesh` instead of `RenderPath::Forward` — the owner's
/// RTX visual sign-off gate for rung R5 (`goldens/PINS.toml`'s `[forwardplus_mesh]`).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the ForwardPlus v1 mesh-only screenshot"]
fn forwardplus_mesh_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine forwardplus mesh-only", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Multi-paradigm render-path plan, rung R5: request `ForwardPlus × Mesh` — inserted AFTER
    // `add_plugins` (which installs `RenderPathPlugin`'s `Deferred`-default) so this override
    // wins, mirroring `forward_mesh.rs`'s own post-plugins owner-override insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::ForwardPlus, legs: GeometryLegs::Mesh });
    app.run();
}
