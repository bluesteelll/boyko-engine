//! Rung VB-P1d — the VisibilityBuffer froxel light-cull GPU-timestamp bench
//! (`docs/VB-PERFORMANCE-TRACK.md`'s VB-P1). [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s
//! five-sphere `grand_showcase_2mat` scene, verbatim, PLUS a PROCEDURALLY-generated
//! `N_ps`-light rig (`BOYKO_VB_BENCH_LIGHTS`) instead of that file's fixed 14-light row —
//! rendered through `RenderPath::VisibilityBuffer × GeometryLegs::Mesh`, with the runner's
//! VB-P1d bench collector (`BOYKO_VB_BENCH`) bracketing the froxel cull dispatch and the
//! `vb_shade`/`vb_resolve` lit-producer dispatch so `boyko_app::runner` can print their
//! averaged GPU wall-clock cost.
//!
//! # Why this measures only ONE leg per process
//!
//! `ResolvedRenderPath::froxel_light_cull` is a BOOT-FROZEN decision (resolved once from
//! `LightingConfig::clusters_enabled` before the window opens, never re-derived per frame —
//! see `GpuSceneBundles::scene`'s own doc) — the froxel arm's GPU pipelines either exist for
//! the WHOLE process or not at all. A single `app.run()` therefore measures exactly one leg
//! (flat OR froxel) of a given `N_ps`; comparing legs needs TWO process runs of this SAME
//! test, exactly as [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s own `BOYKO_VB_FROXEL_FORCE_OFF`
//! knob already establishes for its equality golden. The orchestrator runs this bench twice
//! per `N_ps` and reads the break-even from the two printed `VB-P1d ...` lines.
//!
//! # Env knobs
//!
//! - `BOYKO_VB_BENCH_LIGHTS=<n>` — the point/spot light count `N_ps` this run's [`setup`]
//!   spawns (default 14, matching [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s own base rig).
//!   Read TWICE, independently, by this file's [`setup`] (to spawn the lights) and by
//!   `boyko_app::runner`'s frame loop (as a print label only) — a single source of truth.
//! - `BOYKO_VB_BENCH=1` (any value) — arms the runner's timestamp collector + the bench
//!   accumulation/print loop (`boyko_app::runner`). Unset ⇒ this test behaves exactly like an
//!   ordinary windowed dump (no bench print, no query pools — byte-identical command stream).
//! - `BOYKO_VB_BENCH_FRAMES=<n>` — the TIMED frame budget (default 220, `VB_BENCH_DEFAULT_FRAMES`
//!   in `runner.rs`); the first 20 (`VB_BENCH_WARMUP`) are discarded as warm-up.
//! - `BOYKO_VB_FROXEL_FORCE_OFF` (any value; presence is the trigger) forces
//!   `LightingConfig::clusters_enabled = false` — the flat baseline leg. Unset (the default)
//!   arms clustering — the froxel leg. Mirrors [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s own
//!   knob exactly.
//!
//! Windowed-test conventions (mirrors `vb_mesh_froxel.rs`): `#[ignore]` (needs a real windowed
//! GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//!
//! Invoke (one leg, one `N_ps`):
//! ```text
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=64 \
//!   cargo test -p boyko-app --test vb_p1d_cull_shade_bench -- --ignored --nocapture --test-threads=1
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=64 BOYKO_VB_FROXEL_FORCE_OFF=1 \
//!   cargo test -p boyko-app --test vb_p1d_cull_shade_bench -- --ignored --nocapture --test-threads=1
//! ```
//! prints one `VB-P1d N_ps=64 config=froxel cull_reset_ns=.. cull_dispatch_ns=..
//! froxel_cull_ns=.. froxel_shade_ns=.. froxel_total_ns=..` line and one
//! `VB-P1d N_ps=64 config=flat flat_shade_ns=..` line respectively. (VB-P1e's rung H0 split the
//! cull bracket in two; `froxel_cull_ns` is now the sum of the first two fields.)
//!
//! ⚠️ **This bench does NOT reproduce across sessions above `N_ps` ≈ 128.** Re-measured on the
//! same RTX 3060 against the table committed at `e7a4767` (the provenance doc-comment on
//! `boyko_render::light_policy::CLUSTER_LO`): `N_ps` ≤ 128 reproduces within ~6%, `N_ps=256` is
//! +21%, and `N_ps=512` is **+125% on the flat leg** / +55% on the froxel leg. Run-to-run spread
//! at `N_ps=512` is ~21% (1.29 / 1.33 / 1.57 ms across three runs), while `BOYKO_VB_BENCH_FRAMES`
//! 40 vs 220 differ by 0.13% — so the pass is stable WITHIN a run and unstable ACROSS runs (GPU
//! power/clock state is the leading suspect; not identified). Consequences: a single-sample
//! threshold comparison at high `N_ps` is not decidable on this harness — repeat runs and state a
//! variance band — and `CLUSTER_LO`/`CLUSTER_HI` derive from exactly the rows that do not
//! reproduce (they stay conservative, since the divergence favours clustering more, but they are
//! not supported at the precision the table claims).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{
    ClusterConfig, GeometryLegs, LightingConfig, MeshAssetsVbExt, MeshGeometryTableSlot,
    RenderPath, RenderPathConfig,
};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s / `vb_mesh.rs`'s
/// / `vb_mesh_froxel.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// The default `N_ps` when `BOYKO_VB_BENCH_LIGHTS` is unset — matches `vb_mesh_froxel.rs`'s own
/// base rig (10 points + 4 spots).
const DEFAULT_N_PS: u32 = 14;

/// Verbatim copy of `vb_mesh_froxel.rs::uv_sphere` (itself a verbatim copy of
/// `grand_showcase_2mat.rs::uv_sphere` via `vb_mesh.rs`) — see that file's NOTE for why this is
/// a local copy rather than a shared `tests/common` helper.
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

/// A small fixed warm/cool palette the procedural rig cycles through (index `i % PALETTE.len()`)
/// — mirrors `vb_mesh_froxel.rs`'s own varied per-row colors without needing a per-light table.
const PALETTE: [[f32; 3]; 6] = [
    [1.0, 0.75, 0.5],
    [0.6, 0.8, 1.0],
    [1.0, 0.6, 0.6],
    [0.7, 1.0, 0.7],
    [0.8, 0.8, 1.0],
    [1.0, 0.9, 0.6],
];

/// Low-discrepancy (golden-ratio Kronecker-sequence) 3D placement for light `i` of `n` —
/// spreads `N_ps` lights across a volume that grows mildly with `n` (cube-root scaling keeps
/// the AVERAGE per-froxel light density, and so the per-cluster light count, roughly constant
/// regardless of `N_ps`), so a 1024-light sweep neither piles every light into the same handful
/// of clusters (risking `MAX_LIGHTS_PER_CLUSTER`/`INDEX_LIST_CAP` overflow) nor spreads so thin
/// that most froxels see zero lights (which would defeat the point of a cull-cost sweep).
///
/// The three fractional-part multipliers (`g`, `g^2`, `g^3` for the golden ratio conjugate `g`)
/// are mutually irrational, so the sequence never repeats/aliases across the three axes for any
/// `i` in `[0, MAX_LIGHTS)`.
fn light_position(i: u32, n: u32) -> [f32; 3] {
    let scale = (f64::from(n) / f64::from(DEFAULT_N_PS)).max(1.0).cbrt() as f32;
    let half_x = 4.5 * scale;
    let y_min = 0.3;
    let y_span = 3.3 * scale;
    let z_min = -2.0 * scale;
    let z_span = 6.0 * scale;

    let t = f64::from(i);
    let fx = (t * 0.618_033_988_75).fract() as f32;
    let fy = (t * 0.381_966_011_25).fract() as f32;
    let fz = (t * 0.236_067_977_5).fract() as f32;
    [(fx * 2.0 - 1.0) * half_x, y_min + fy * y_span, z_min + fz * z_span]
}

/// A small jittered range in `[1.2, 2.0]` — kept modest (well below the 2.5-4.0 the fixed
/// `vb_mesh_froxel.rs` rig uses) so each light's froxel footprint stays bounded even at a
/// large `N_ps` (`light_position`'s own doc explains the companion volume-scaling half).
fn light_range(i: u32) -> f32 {
    1.2 + ((f64::from(i) * 0.142_857).fract() as f32) * 0.8
}

/// Verbatim copy of `vb_mesh_froxel.rs::setup`'s five-sphere geometry + sun + sky, PLUS a
/// PROCEDURALLY-generated `N_ps`-light rig (`BOYKO_VB_BENCH_LIGHTS`, default [`DEFAULT_N_PS`])
/// in place of that file's fixed 14-row table — every 4th light (`i % 4 == 3`) is a spot,
/// aimed down at the sphere row exactly as `vb_mesh_froxel.rs`'s own spots are; the rest are
/// points. No shadow-casting flags (this bench measures cull/shade cost, not the atlas).
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

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

    // The procedural `N_ps` point/spot rig (this file's own module doc).
    let n_ps: u32 = std::env::var("BOYKO_VB_BENCH_LIGHTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_N_PS);
    debug_assert!(
        n_ps < boyko_render::MAX_LIGHTS,
        "invariant: N_ps must stay below MAX_LIGHTS (the point/spot table capacity)"
    );
    let aim = Vec3::new(0.0, 0.6, 0.0);
    for i in 0..n_ps {
        let pos = light_position(i, n_ps);
        let color = PALETTE[(i as usize) % PALETTE.len()];
        let range = light_range(i);
        let power = 65.0;
        if i % 4 == 3 {
            let p = Vec3::new(pos[0], pos[1], pos[2]);
            let pose = Affine3A::look_at_rh(p, aim, Vec3::new(0.0, 1.0, 0.0));
            commands.spawn(SpotLightObject {
                transform: Transform {
                    translation: p,
                    rotation: Quat::from_mat3(pose.matrix3),
                    scale: Vec3::ONE,
                },
                global: GlobalTransform::IDENTITY,
                light: SpotLight::new(pos, [0.0, -1.0, 0.0], color, power, range, 15.0, 30.0),
            });
        } else {
            commands.spawn(PointLightObject {
                transform: Transform::from_translation(Vec3::new(pos[0], pos[1], pos[2])),
                global: GlobalTransform::IDENTITY,
                light: PointLight::new(pos, color, power, range),
            });
        }
    }

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

/// **The VB-P1d froxel cull/shade GPU-timestamp bench (one leg, one `N_ps`, per process).**
/// This file's own module doc covers the env knobs + why two runs are needed per `N_ps`.
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`,
/// `BOYKO_VB_BENCH=1`, `BOYKO_VB_BENCH_LIGHTS=<n>`, optionally `BOYKO_VB_FROXEL_FORCE_OFF`;
/// the orchestrator sweeps `N_ps ∈ {8, 64, 256, 1024}` × `{froxel, flat}`.
#[test]
#[ignore = "needs a real windowed GPU device; BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=<n> \
            [BOYKO_VB_FROXEL_FORCE_OFF=1] BOYKO_DISABLE_VALIDATION=1 -- --ignored --nocapture \
            --test-threads=1; the orchestrator sweeps N_ps and both legs"]
fn vb_p1d_cull_shade_bench() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb-p1d cull/shade bench", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    // The clusters on/off knob (this file's module doc): unset arms clustering (the froxel
    // leg), `BOYKO_VB_FROXEL_FORCE_OFF` forces it off (the flat baseline leg) — the SAME
    // env-toggle convention `vb_mesh_froxel.rs` uses.
    let clusters_enabled = std::env::var("BOYKO_VB_FROXEL_FORCE_OFF").is_err();
    app.insert_resource(LightingConfig { clusters_enabled, ..LightingConfig::default() });
    app.insert_resource(ClusterConfig::default());
    app.run();
}
