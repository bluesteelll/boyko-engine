//! VG rung R2c-tail: **the gate the goldens cannot be** — proof, read off the GPU, that the
//! per-batch camera cull actually rejects.
//!
//! # Why this test has to exist
//!
//! Every pinned VB scene is entirely on-screen. A cull that rejects NOTHING therefore renders a
//! byte-identical image to a correct one, so the nine golden pins say only that rung R2c broke
//! nothing — they cannot distinguish an armed cull from an inert one. `vb_batch_cull_spv_sync.rs`
//! proves the module CONTAINS a decision and `boyko_render::frustum`'s oracle proves the HOST math
//! rejects an off-screen box, but neither observes the GPU making the call.
//!
//! The observable that does is the visible COUNT the cull's `InterlockedAdd` produces. Rung R2c0
//! built that counter and its compacted list and left them deliberately unread; this is their first
//! consumer.
//!
//! # The fixture
//!
//! TWO distinct meshes, hence two `DrawBatch`es (`gather_mesh_draws` buckets by `mesh_id`, so two
//! spawns of the SAME handle would be one batch of two instances and would prove nothing about
//! per-BATCH rejection):
//!
//! * a sphere at the origin, squarely in frame;
//! * a second, slightly different sphere at `x = 40` — far outside the 52°-fov cone at that depth,
//!   so it is wholly in the left/right plane's negative half-space.
//!
//! The assertion is `batches == 2, visible == 1`, and it is two-sided by construction: `visible ==
//! 2` means the cull kept something it should have rejected (armed but inert — exactly what a
//! golden would wave through), `visible == 0` means it rejected something visible (which the
//! goldens WOULD catch, but this names it precisely).
//!
//! `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1` and
//! `--test-threads=1`, the same conventions every windowed test here follows.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

/// The sun direction TO the light — `vb_mesh.rs`'s value, so the fixture lights the same way the
/// pinned scene does and a visual glance at it is comparable.
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// The off-screen sphere's world X. Chosen against the camera below rather than picked round: the
/// rig sits at `z = 7.8` with a 52° vertical fov, so the view cone's half-width near the origin
/// plane is under 4 units. At 40 the sphere is an order of magnitude outside it, which keeps the
/// test insensitive to small changes in the fixture's framing — it is testing the CULL, not the
/// exact aspect ratio.
const OFFSCREEN_X: f32 = 40.0;

/// `vb_mesh.rs`'s `uv_sphere`, copied for the same reason that file copies it: a fixture scene
/// keeps its own mesh generation, so a later edit to a shared helper cannot silently re-shape it.
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi;
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32;
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi);
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            let u = j as f32 / slices as f32;
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

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let mut register = |verts: &[Vertex], idx: &[u32], geo: &mut MeshGeometryTableSlot| {
        match geo.0.as_mut() {
            Some(table) => meshes.register_mesh_vb(dev.get(), verts, idx, table),
            None => meshes.register_mesh(dev.get(), verts, idx),
        }
    };

    // TWO registrations ⇒ two mesh ids ⇒ two batches. The radii differ so the two are
    // unmistakably distinct geometry, not an accidental de-duplication.
    let (v_a, i_a) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere_in = register(&v_a, &i_a, &mut geo_table);
    let (v_b, i_b) = uv_sphere(0.70, 24, 32, [0.7, 0.7, 0.72, 1.0]);
    let sphere_out = register(&v_b, &i_b, &mut geo_table);

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let e_in = commands
        .spawn(MeshBundle::new(sphere_in, Transform::from_translation(Vec3::new(0.0, 0.6, 0.0))))
        .id();
    commands.entity(e_in).insert(MaterialHandle(red.index() as u16));

    // The one the cull must reject.
    let e_out = commands
        .spawn(MeshBundle::new(
            sphere_out,
            Transform::from_translation(Vec3::new(OFFSCREEN_X, 0.6, 0.0)),
        ))
        .id();
    commands.entity(e_out).insert(MaterialHandle(blue.index() as u16));

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

/// Boots the fixture with the readback probe armed, then asserts on what the GPU reported.
///
/// The probe path (`BOYKO_VB_CULL_READBACK=<path>`) makes the runner wait the device idle after a
/// presented frame, copy the cull's DEVICE-LOCAL counter and visible list into host-visible
/// staging, write one line, and stop. The counter and the list are NOT relocated for the probe —
/// what is read is a transfer copy of the buffers exactly as they ship, so this proves the cull in
/// the configuration that renders rather than in one built for the test.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to read the batch cull's visible count"]
fn vb_cull_rejects_the_offscreen_batch() {
    let out = std::env::temp_dir().join("boyko_vb_cull_readback.txt");
    let _ = std::fs::remove_file(&out);
    // SAFETY: single-threaded test setup, before any engine thread exists. Windowed tests in this
    // crate run with `--test-threads=1` by convention, so no sibling test observes this write.
    unsafe {
        std::env::set_var("BOYKO_VB_CULL_READBACK", &out);
    }

    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb cull offscreen", 512, 512));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    app.run();

    let line = std::fs::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "the cull-readback probe wrote no file at {}: {e}. The run ended without reaching the \
             readback, so nothing here is evidence about the cull.",
            out.display()
        )
    });

    assert!(
        line.contains("batches=2"),
        "the fixture must produce exactly TWO draw batches (two distinct mesh ids) — got {line:?}. \
         One batch means the two spheres were bucketed together and the test cannot say anything \
         about per-BATCH rejection."
    );
    assert!(
        line.contains("visible=1"),
        "the GPU cull must report exactly ONE visible batch — got {line:?}.\n\
         `visible=2` means the cull is armed but rejects nothing: the off-screen sphere at x={} \
         survived every frustum plane. That state renders a byte-identical image on every pinned \
         scene, so no golden would catch it — which is why this test exists.\n\
         `visible=0` means it rejected the sphere in front of the camera.",
        OFFSCREEN_X
    );
    // The compacted list must name the batch that survived, not merely count it: a correct count
    // with a garbage list would still satisfy the assertion above.
    assert!(
        line.contains("list=[0]") || line.contains("list=[1]"),
        "the visible list must carry the surviving batch's index — got {line:?}"
    );
}
