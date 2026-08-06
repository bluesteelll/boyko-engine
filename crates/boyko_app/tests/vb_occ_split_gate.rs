//! VG R3 piece 2, step P2-6 — **gate G2: the split RAN.**
//!
//! `goldens/PINS.toml`'s `[vb_occ_split]` (gate G1) proves the marked scene's pixels are the
//! unmarked scene's pixels. It is satisfied just as well **by not splitting at all** — that is
//! not a weakness of the fixture, it is what a hash of an unchanged image can mean. This file is
//! the leg that says the recorder recorded a second raster scope, and the pair is the evidence:
//! **G1 green + G2 red under the same corruption.**
//!
//! # The number comes from the RECORDER
//!
//! `Renderer::record_vb` fills a `boyko_rhi_vulkan::present::VbRecordProbe` **at** the
//! `vkCmdBeginRendering`/`vkCmdDrawIndexedIndirect` calls it counts, the host frame loop writes it
//! to `BOYKO_VB_PROBE=<path.toml>`, and this driver reads it back. A host that re-derived `scopes`
//! from `GBufferScene::vb_occlusion_instances` would agree with itself no matter what the recorder
//! did — the tautology this campaign has shipped as a gate five times. The host's own numbers
//! (`draw_batches`, `occlusion_instances`) travel in a SEPARATE `[host]` table of the same file and
//! are used as an INDEPENDENT cross-check, never as the source.
//!
//! # TWO fixtures, because one of them cannot falsify the per-batch clause
//!
//! * [`vb_occ_probe_dump_marked`] / [`vb_occ_probe_dump_unmarked`] — `[vb_mesh]`'s five spheres,
//!   which share ONE `MeshHandle` and therefore ONE `DrawBatch`. `late_draws == 1` here is
//!   satisfiable by a hard-coded draw or a `take(1)`: on this fixture alone the per-batch loop,
//!   the `i * DRAW_INDEXED_INDIRECT_STRIDE` offset and the `batch_count` bound are all
//!   unfalsifiable.
//! * [`vb_occ_probe_dump_multi`] — **`vb_occ_multi`**: two REGISTERED meshes (a floor and a
//!   sphere) with a STRICT SUBSET marked, so `draw_batches == 2`, the record offset is evaluated
//!   at `i > 0`, and the mixed-archetype gather path runs. Deliberately **not** a golden pin: a
//!   pin would buy a second byte-identity claim at the price of a new `VB_PINS` name and a
//!   blessing ceremony, while G2's counts plus G3 and G4 already cover what the multi-batch case
//!   adds.
//!
//! # What this gate CANNOT claim
//!
//! * **That the GPU EXECUTED the late scope.** It proves the host RECORDED it. A scope whose every
//!   draw carries `instanceCount = 0` has no observable consequence of execution, so no gate in
//!   this repository can close that gap; the nearest independent evidence is the validation leg
//!   (G3), and on this machine that leg sees static legality only (the plan's "P2-0 RESOLVED").
//! * **Anything about barriers.** These are host counts. A missing barrier leaves every one of
//!   them exactly as it is — measured, on this machine: a genuine missing barrier produced 19
//!   validation messages, no `SYNC-HAZARD` and a byte-identical image. Gate G4
//!   (`boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs`) is the only leg that can see one.
//! * **Anything about pixels.** `vb_occ_multi` has no golden, so **no golden covers a multi-BATCH
//!   late scope.** That is piece 3's first gate.
//!
//! # Red controls (executed at P2-7, listed here so a reader can re-run them)
//!
//! | corruption | expected |
//! |---|---|
//! | force `GBufferScene::path_vb_occlusion_split()` to `false` | `scopes == 1` on the marked scene: **G2 reds while G1 stays green** — the pair that proves G1 needs G2 |
//! | set one late record's `instanceCount = 1` | G1 stays green by construction (`GREATER` rejects a redraw at identical depth); **G2 reds** on `late_instances` |
//! | `take(1)` in the late draw loop | green on the single-batch fixture, **red on `vb_occ_multi`** — which is the whole reason that fixture exists |
//!
//! # Run
//!
//! `cargo test -p boyko-app --test vb_occ_split_gate -- --ignored --nocapture --test-threads=1`
//! with `BOYKO_DISABLE_VALIDATION=1`. The driver spawns the three workers itself; a worker run
//! directly needs `BOYKO_VB_PROBE=<path.toml>` and SKIPS (rather than looping forever) without it.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, OcclusionCulling, RenderPath,
    RenderPathConfig, generate_tangents,
};

/// The env knob that arms `boyko_app::vb_probe_dump` — the value is the output path.
const ENV_PROBE: &str = "BOYKO_VB_PROBE";

/// The worker window's client extent — `[vb_mesh]`'s own 512², so the single-batch fixture is the
/// pinned scene at the pinned size rather than a lookalike.
const EXTENT: u32 = 512;

/// The sun direction TO the light (byte-identical to `vb_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Spheres in the single-batch fixture — `[vb_mesh]`'s five, marked or not as a block.
const SINGLE_SPHERES: u32 = 5;

/// Marked spheres in `vb_occ_multi`. The floor stays UNMARKED, which is what makes the marked set
/// a STRICT subset and puts the two mesh families in two archetypes.
const MULTI_MARKED_SPHERES: u32 = 3;

/// Registered meshes — and therefore `DrawBatch`es — in `vb_occ_multi`: the floor and the sphere.
/// Batches are bucketed per `MeshHandle`, so this is the host's own expectation for
/// `draw_batches`, derived from the FIXTURE rather than read back from the frame.
const MULTI_BATCHES: u32 = 2;

// ===============================================================================================
// The scenes
// ===============================================================================================

/// Verbatim copy of `vb_mesh.rs::uv_sphere` — a local copy for the reason that file gives: a
/// fixture compared against a recorded measurement keeps its own mesh generation instead of moving
/// under it when a shared helper is edited for someone else.
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

/// A flat `size`×`size` quad at y = 0 — the SECOND registered mesh of `vb_occ_multi`, and the one
/// that stays unmarked. Two triangles is enough: this gate counts batches, and a batch's cost here
/// is one record and one draw regardless of its geometry.
fn floor_plane(size: f32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let h = size * 0.5;
    let n = [0.0, 1.0, 0.0];
    let mut verts = vec![
        Vertex::new([-h, 0.0, -h], n, color),
        Vertex::new([h, 0.0, -h], n, color),
        Vertex::new([h, 0.0, h], n, color),
        Vertex::new([-h, 0.0, h], n, color),
    ];
    let idx = vec![0, 2, 1, 0, 3, 2];
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// The lighting + camera every fixture here shares — `vb_mesh.rs`'s, so a worker's frame is an
/// ordinary VB frame rather than a special one.
fn spawn_view_and_lights(commands: &mut Commands) {
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

/// `[vb_mesh]`'s five spheres, ONE registered mesh, marked or not as a block.
///
/// `marked` is a parameter of the fixture rather than of the frame: both workers below call this
/// with a constant, so the two runs differ in exactly one component's presence — and the marker is
/// in the BUNDLE, because an `insert` migrates the archetype at the next command flush and would
/// arm the split one frame late.
fn setup_single(
    commands: &mut Commands,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    geo_table: &mut MeshGeometryTableSlot,
    dev: &GpuDevice,
    marked: bool,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };
    let mat = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let mat_index = mat.index() as u16;

    let spacing = 1.55;
    for i in 0..SINGLE_SPHERES {
        let x = (i as f32 - 2.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        // ⚠️ Queued into the SAME command flush as the spawn — NOT into the bundle. This kernel
        // has no tuple `Bundle` impl (`Bundle` is sealed and implemented per type), so
        // `spawn((MeshBundle, OcclusionCulling))` does not compile. What arms the split one frame
        // late is an insert from a LATER frame; queued together, spawn and insert are applied by
        // ONE flush before any gather runs — the route `MaterialHandle` below already takes.
        if marked {
            commands.entity(e).insert(OcclusionCulling);
        }
        commands.entity(e).insert(MaterialHandle(mat_index));
    }
    spawn_view_and_lights(commands);
}

/// The `vb_occ_multi` scene: TWO registered meshes, a STRICT SUBSET marked.
///
/// The floor is a second `MeshHandle` and therefore a second `DrawBatch`, which is the whole point
/// — `late_draws == draw_batches` is unfalsifiable at one batch. It is also the row that keeps the
/// marked set strict: with the floor unmarked, the mesh family occupies two archetypes and the
/// gather walks the mixed path.
fn setup_multi(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (fv, fi) = floor_plane(12.0, [0.4, 0.4, 0.42, 1.0]);
    let (sv, si) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let (floor_mesh, sphere) = match geo_table.0.as_mut() {
        Some(table) => (
            meshes.register_mesh_vb(dev.get(), &fv, &fi, table),
            meshes.register_mesh_vb(dev.get(), &sv, &si, table),
        ),
        None => (
            meshes.register_mesh(dev.get(), &fv, &fi),
            meshes.register_mesh(dev.get(), &sv, &si),
        ),
    };
    let mat = materials.add(Material::new([0.55, 0.55, 0.58, 1.0], 0.0, 0.45, 0.5, [0.0; 3], 0));
    let mat_index = mat.index() as u16;

    // The floor: UNMARKED, so the marked set is a strict subset of the ring.
    let floor = commands
        .spawn(MeshBundle::new(floor_mesh, Transform::from_translation(Vec3::new(0.0, -0.4, 0.0))))
        .id();
    commands.entity(floor).insert(MaterialHandle(mat_index));

    let spacing = 1.55;
    for i in 0..MULTI_MARKED_SPHERES {
        let x = (i as f32 - 1.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        // Same flush as the spawn — see `setup_single`'s note on why this is not the bundle.
        commands.entity(e).insert(OcclusionCulling);
        commands.entity(e).insert(MaterialHandle(mat_index));
    }
    spawn_view_and_lights(&mut commands);
}

/// `setup_single` with the marker — the `vb_occ_split` scene, as an ECS startup system.
fn setup_single_marked(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    setup_single(&mut commands, &mut meshes, &mut materials, &mut geo_table, &dev, true);
}

/// `setup_single` WITHOUT the marker — the control the whole gate rests on: same scene, same
/// binary, one component's presence apart.
fn setup_single_unmarked(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    setup_single(&mut commands, &mut meshes, &mut materials, &mut geo_table, &dev, false);
}

// ===============================================================================================
// The workers: one process, one boot, one probe file
// ===============================================================================================

/// The probe path, or `None` after announcing a SKIP.
///
/// A worker booted without [`ENV_PROBE`] arms no capture, so the host loop has nothing to complete
/// and `app.run()` never returns — a hang, the worst failure mode a test sweep can have. The
/// driver always sets the knob; a human running `--ignored` over the whole binary gets a named
/// skip instead of a hung window.
fn probe_path_or_skip(label: &str) -> Option<String> {
    match std::env::var(ENV_PROBE) {
        Ok(path) => {
            eprintln!("{label}: probing to {path}");
            Some(path)
        }
        Err(_) => {
            eprintln!(
                "{label}: {ENV_PROBE} is unset -- SKIPPED. This worker exists to be spawned by \
                 `vb_occ_split_records_two_scopes`; booted without the knob it would render \
                 forever, since no armed capture could ever complete."
            );
            None
        }
    }
}

/// The `RenderPathConfig` every worker here inserts AFTER `add_plugins`, so it overrides
/// `RenderPathPlugin`'s `Deferred` default — `vb_mesh.rs`'s own post-plugins override discipline.
const VB_MESH_PATH: RenderPathConfig =
    RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh };

/// **G2 worker — the MARKED single-batch scene** (`vb_occ_split`'s own).
#[test]
#[ignore = "needs a real windowed GPU device; the G2 driver spawns it with BOYKO_VB_PROBE set"]
fn vb_occ_probe_dump_marked() {
    if probe_path_or_skip("vb_occ_probe_dump_marked").is_none() {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb occ G2 marked", EXTENT, EXTENT));
    app.add_startup_system(setup_single_marked);
    app.insert_resource(VB_MESH_PATH);
    app.run();
}

/// **G2 worker — the UNMARKED single-batch scene.** The control: `scopes == 1` here is what makes
/// `scopes == 2` on the marked run a measurement rather than a constant.
#[test]
#[ignore = "needs a real windowed GPU device; the G2 driver spawns it with BOYKO_VB_PROBE set"]
fn vb_occ_probe_dump_unmarked() {
    if probe_path_or_skip("vb_occ_probe_dump_unmarked").is_none() {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb occ G2 unmarked", EXTENT, EXTENT));
    app.add_startup_system(setup_single_unmarked);
    app.insert_resource(VB_MESH_PATH);
    app.run();
}

/// **G2 worker — `vb_occ_multi`**: two registered meshes, a strict subset marked.
#[test]
#[ignore = "needs a real windowed GPU device; the G2 driver spawns it with BOYKO_VB_PROBE set"]
fn vb_occ_probe_dump_multi() {
    if probe_path_or_skip("vb_occ_probe_dump_multi").is_none() {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb occ G2 multi", EXTENT, EXTENT));
    app.add_startup_system(setup_multi);
    app.insert_resource(VB_MESH_PATH);
    app.run();
}

// ===============================================================================================
// The artifact, decoded
// ===============================================================================================

/// One decoded `BOYKO_VB_PROBE` file: the recorder's counts and, separately, the host's.
struct Probe {
    /// `[probe]` — written by `Renderer::record_vb` at the `vkCmd*` calls it counts.
    scopes: u32,
    late_draws: u32,
    late_instances: u32,
    /// `[host]` — derived on the host, at other sites, for the cross-check.
    draw_batches: u32,
    occlusion_instances: u32,
    vb_path: bool,
    mesh_leg: bool,
}

/// The raw right-hand side of `table.key`.
///
/// A local reader rather than a shared one: this file is a gate on its own artifact, and a gate
/// that borrows another gate's parser inherits that gate's future edits. Fifteen lines is a
/// cheaper price than that coupling. Same flat, section-scoped TOML subset the writer emits.
fn field(src: &str, path: &str, file: &std::path::Path) -> String {
    let (table, key) = path.split_once('.').expect("a probe path is `table.key`");
    let mut inside = false;
    for line in src.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') && l.ends_with(']') {
            inside = l.trim_start_matches('[').trim_end_matches(']') == table;
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return v.trim().to_string();
        }
    }
    panic!(
        "the probe file {} has no `{path}` -- a gate that reads a missing field asserts nothing",
        file.display()
    )
}

fn field_u32(src: &str, path: &str, file: &std::path::Path) -> u32 {
    field(src, path, file).parse().unwrap_or_else(|_| panic!("`{path}` is not an integer"))
}

/// Strict: anything that is neither `true` nor `false` PANICS rather than reading as `false`. A
/// silent `false` here would fire the instrument clause and report "this is not a VB frame" about
/// a frame that was one.
fn field_bool(src: &str, path: &str, file: &std::path::Path) -> bool {
    match field(src, path, file).as_str() {
        "true" => true,
        "false" => false,
        other => panic!("`{path}` is `{other}`, which is not a boolean"),
    }
}

/// Spawns one worker and returns the probe it wrote.
fn run_worker(worker: &str) -> Probe {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out: PathBuf = std::env::temp_dir().join(format!("vb_occ_probe_{worker}.toml"));
    // A stale file from a previous run that this run failed to overwrite would be read as this
    // run's evidence.
    let _ = std::fs::remove_file(&out);

    let status = Command::new(&exe)
        .args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(ENV_PROBE, &out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The record probe is the only capture this run is for. Another armed capture would render
        // the same frames for no reason AND hold the host loop open until it too completed.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_HZB_DUMP")
        .status()
        .expect("invariant: the worker process spawns");
    assert!(status.success(), "the G2 worker `{worker}` exited {status}");

    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "the G2 worker `{worker}` wrote no probe at {}: {e}. A worker that renders and \
             produces nothing is an instrument failure, not an unsplit frame.",
            out.display()
        )
    });
    Probe {
        scopes: field_u32(&text, "probe.scopes", &out),
        late_draws: field_u32(&text, "probe.late_draws", &out),
        late_instances: field_u32(&text, "probe.late_instances", &out),
        draw_batches: field_u32(&text, "host.draw_batches", &out),
        occlusion_instances: field_u32(&text, "host.occlusion_instances", &out),
        vb_path: field_bool(&text, "host.vb_path", &out),
        mesh_leg: field_bool(&text, "host.mesh_leg", &out),
    }
}

/// The clause every other clause depends on: this boot really did resolve `VisibilityBuffer ×
/// Mesh`. A device that fails the VB capability probe degrades to `Deferred`, `record_vb` never
/// runs, and every count would be zero for an INSTRUMENT reason that reads exactly like "the split
/// did not record".
fn assert_is_a_vb_mesh_frame(label: &str, p: &Probe) {
    assert!(
        p.vb_path && p.mesh_leg,
        "{label}: the probed frame resolved vb_path={} mesh_leg={} -- it is not a \
         `VisibilityBuffer × Mesh` frame, so its counts say nothing about the split. This is an \
         instrument failure (a device that failed the VB capability probe degrades to Deferred), \
         not a gate result.",
        p.vb_path,
        p.mesh_leg
    );
}

// ===============================================================================================
// The gate
// ===============================================================================================

/// **GATE G2** — on a MARKED scene the recorder records TWO raster scopes, `draw_batches` late
/// indirect draws, and a late `instanceCount` sum of ZERO; on the UNMARKED scene it records ONE.
///
/// Read the module header for what this cannot claim (the GPU never enters the picture) and for
/// the three red controls.
#[test]
#[ignore = "live GPU gate (spawns three windowed workers); the orchestrator runs it with --test-threads=1"]
fn vb_occ_split_records_two_scopes() {
    let marked = run_worker("vb_occ_probe_dump_marked");
    let unmarked = run_worker("vb_occ_probe_dump_unmarked");
    let multi = run_worker("vb_occ_probe_dump_multi");

    for (label, p) in
        [("marked", &marked), ("unmarked", &unmarked), ("vb_occ_multi", &multi)]
    {
        assert_is_a_vb_mesh_frame(label, p);
    }

    // ---- the fixtures are the fixtures ---------------------------------------------------------
    //
    // Asserted BEFORE the counts: a marked scene whose marker never reached the ring would report
    // `scopes == 1` for a FIXTURE reason, and that must not read as a recorder defect.
    assert_eq!(
        marked.occlusion_instances, SINGLE_SPHERES,
        "marked: {} of the {SINGLE_SPHERES} spheres carried `OcclusionCulling` into the ring. The \
         marker is in the SPAWN BUNDLE, so it is visible to the very first gather; a shortfall \
         means the gather's `Option<&OcclusionCulling>` lane is not reading what the fixture wrote.",
        marked.occlusion_instances
    );
    assert_eq!(
        unmarked.occlusion_instances, 0,
        "unmarked: {} instances carried the marker in a scene that spawns none -- the control is \
         not a control",
        unmarked.occlusion_instances
    );
    assert_eq!(
        multi.occlusion_instances, MULTI_MARKED_SPHERES,
        "vb_occ_multi: {} marked instances, expected the {MULTI_MARKED_SPHERES} spheres and NOT \
         the floor. A strict subset is what puts the two mesh families in two archetypes.",
        multi.occlusion_instances
    );
    assert_eq!(
        marked.draw_batches, 1,
        "marked: {} draw batches. `[vb_mesh]`'s five spheres share one `MeshHandle`, so this \
         fixture has exactly ONE batch -- which is why `vb_occ_multi` exists.",
        marked.draw_batches
    );
    assert_eq!(
        multi.draw_batches, MULTI_BATCHES,
        "vb_occ_multi: {} draw batches, expected {MULTI_BATCHES} (one per registered mesh). \
         Without at least two, `late_draws == draw_batches` is satisfied by a hard-coded draw.",
        multi.draw_batches
    );

    // ---- the load-bearing clause: the recorder recorded a SECOND scope ---------------------------
    assert_eq!(
        marked.scopes, 2,
        "marked: the recorder reported {} raster scope(s). This number is incremented AT the \
         `vkCmdEndRendering` of each scope, so a 1 here means the late scope was NOT recorded -- \
         and `[vb_occ_split]`'s golden would still be green, because a scope that draws nothing \
         and a scope that does not exist produce the same pixels.",
        marked.scopes
    );
    assert_eq!(
        unmarked.scopes, 1,
        "unmarked: the recorder reported {} raster scopes on a scene that marks NOTHING. Every \
         shipping frame and all 25 golden pins take this path; a second scope here is the split \
         arming itself on the unarmed path.",
        unmarked.scopes
    );
    assert_eq!(multi.scopes, 2, "vb_occ_multi: expected two scopes, got {}", multi.scopes);

    // ---- the per-batch clause, falsifiable only on the multi fixture ------------------------------
    //
    // `late_draws` is counted per ISSUED draw inside the loop and `draw_batches` is the host's own
    // `mesh_draw.len()`, derived at a different site — so this compares two derivations, not one
    // with itself. They are equal here because both fixtures sit far below every capacity the
    // recorder's `batch_count` is min-ed against (the record array, the descriptor array and the
    // survivor-list clamp are all 1024); a divergence would be a real finding about that bound,
    // and this gate is where it would surface.
    assert_eq!(
        marked.late_draws, marked.draw_batches,
        "marked: {} late draws against {} batches",
        marked.late_draws, marked.draw_batches
    );
    assert_eq!(
        multi.late_draws, multi.draw_batches,
        "vb_occ_multi: {} late draws against {} batches. THIS is the clause the single-batch \
         fixture cannot falsify: a `take(1)` in the late loop, a hard-coded draw, or an offset \
         that is only ever evaluated at `i == 0` all pass there and fail here.",
        multi.late_draws, multi.draw_batches
    );
    assert_eq!(
        unmarked.late_draws, 0,
        "unmarked: {} late draws recorded with no late scope",
        unmarked.late_draws
    );

    // ---- the inertness clause, and the tripwire piece 3 must delete deliberately -------------------
    for (label, p) in
        [("marked", &marked), ("unmarked", &unmarked), ("vb_occ_multi", &multi)]
    {
        assert_eq!(
            p.late_instances, 0,
            "{label}: the late records sum to {} instances. PIECE 2 ONLY: the late scope draws \
             NOTHING, so every late record's `instanceCount` is the inert 0. Piece 3 makes the \
             late cull the producer of that word and must retire this clause DELIBERATELY, in the \
             same change -- not let it quietly stop holding.",
            p.late_instances
        );
    }

    println!(
        "vb occlusion split G2: marked scopes={} late_draws={} (batches={}), unmarked scopes={}, \
         vb_occ_multi scopes={} late_draws={} (batches={}). Every count is the RECORDER's; \
         `draw_batches` and `occlusion_instances` are the host's, for cross-check. This says the \
         host RECORDED the scope -- never that the GPU executed it.",
        marked.scopes,
        marked.late_draws,
        marked.draw_batches,
        unmarked.scopes,
        multi.scopes,
        multi.late_draws,
        multi.draw_batches
    );
}
