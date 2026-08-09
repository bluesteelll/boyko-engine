//! **Profiling rung 6 — the gbuffer + SV0 brackets, ported and executed.**
//!
//! Rung 5c ported `record_vb`'s ten brackets to the `GpuZoneRecorder` and G10 compared them against
//! the old collector. Rung 6 ports `record_gbuffer`'s: the four software-ray passes
//! (`TimedPass::{DdgiUpdate, DeferredResolve, CsmDepth, PunctualDepth}`) and the Deferred fine
//! marcher (`Sv0TimedPass::Marcher`). This file is the gate that those brackets **run**, on a real
//! `Deferred × Both` frame, and land where the id scheme says they should.
//!
//! # Why this is not "G10 for the gbuffer passes", and what is missing
//!
//! G10's shape is a cross-leg comparison, and the gbuffer family has **no host arming path for its
//! old leg**: `TimestampCollector` is constructed in exactly two places in the whole tree
//! (`boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs` and
//! `.../tests/window_present_gbuffer.rs`), never from `boyko_app`. There is therefore no
//! `boyko_app` worker that can play leg A for `TimedPass`, and the two-process A/B that rung 5c
//! settled on has nothing to put on the other side. Extending G10 to these passes needs either a
//! host arming path for the R0 collector or the A/B moved into the RHI test that already owns one —
//! a scope fork, recorded in `docs/OPEN-QUESTIONS.md` rather than guessed at here.
//!
//! What this gate DOES claim is the half that has no fork: the ported brackets execute, retire
//! `Measured`, and carry ids in their own family's range.
//!
//! # The id ranges are the subject, not a detail
//!
//! `TimedPass::DdgiUpdate`, `Sv0TimedPass::Marcher` and `VbTimedPass::CullReset` are **all slot 0**,
//! and the first two are recorded into the SAME frame's ring slot by the same `record_gbuffer`. The
//! bases (`ZONE_BASE_VB` / `ZONE_BASE_GBUFFER` / `ZONE_BASE_SV0`, const-asserted disjoint) are what
//! keeps an id a name. A count of retired pairs cannot see a collision; the printed ids can, which
//! is why the runner prints them.
//!
//! # What it cannot claim
//!
//! Nothing about the DURATIONS, which no rung before 8 has a band for. Nothing about WHICH passes a
//! frame brackets — that is scene-dependent (`DdgiUpdate` needs a DDGI grid, `PunctualDepth` needs a
//! spot or point light), so the gate asserts a non-empty set in the right ranges rather than a
//! pinned membership it would have to re-derive whenever the fixture moves. And nothing about the
//! old collectors, which this configuration does not arm.
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko-app --test gbuffer_zone_port_gate -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! with `BOYKO_DISABLE_VALIDATION=1`.

#![cfg(windows)]

use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
    generate_tangents,
};

/// The worker's test name.
const WORKER: &str = "gbuffer_zone_port_worker";

/// The driver's private marker — `vb_bench_totality_gate.rs`'s mechanism, for its reason: booted
/// without a leg knob this worker arms no recorder and nothing ends its frame loop.
const DRIVER_MARKER: &str = "BOYKO_GBUF_ZONE_DRIVEN";

/// Retired frames the run collects past the runner's warm-up before it exits.
const BENCH_FRAMES: &str = "10";

/// The automated-run frame cap.
const FRAME_CAP: &str = "400";

/// The boot notice printed when the device cannot serve timestamps — neither green nor red.
const NO_TIMESTAMPS: &str = "device timestamps are unusable";

/// Family bases, mirrored from `boyko_rhi_vulkan::present::gpu_zone`.
///
/// ⚠️ MIRRORED, not imported, and deliberately: `boyko-app` does not depend on the RHI crate's
/// zone module for anything else, and a gate that imported the constant it checks would assert the
/// constant equals itself. These three numbers are the gate's own statement of what the ids should
/// be; if the engine's bases move, this file must be edited to agree, which is the point.
const ZONE_BASE_VB: u16 = 0;
const ZONE_BASE_GBUFFER: u16 = 16;
const ZONE_BASE_SV0: u16 = 32;
const ZONE_FAMILY_WIDTH: u16 = 16;

// ===============================================================================================
// The worker
// ===============================================================================================

/// One UV sphere, procedurally — no asset file.
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let v = i as f32 / stacks as f32;
        let phi = v * core::f32::consts::PI;
        for j in 0..=slices {
            let u = j as f32 / slices as f32;
            let theta = u * core::f32::consts::TAU;
            let (sp, cp) = phi.sin_cos();
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            verts.push(Vertex {
                position: [n[0] * radius, n[1] * radius, n[2] * radius],
                normal: n,
                color,
                uv: [u, v],
                tangent: [1.0, 0.0, 0.0, 1.0],
            });
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

/// A mesh + directional sun, so the CSM depth pass and the deferred resolve both record.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    for i in 0..3 {
        let x = (i as f32 - 1.0) * 1.55;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        commands.entity(e).insert(MaterialHandle(gold.index() as u16));
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

/// **THE WORKER** — `Deferred × Both`, the path whose recorder is `record_gbuffer`.
#[test]
#[ignore = "needs a real windowed GPU device; the gate spawns it with BOYKO_VB_ZONE set"]
fn gbuffer_zone_port_worker() {
    if std::env::var(DRIVER_MARKER).is_err() {
        eprintln!(
            "{WORKER}: {DRIVER_MARKER} unset -- SKIPPED. This worker exists to be spawned by its \
             gate, which sets BOYKO_VB_ZONE; booted without it nothing ends its frame loop."
        );
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine gbuffer zone port", 512, 512));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Both });
    app.run();
}

// ===============================================================================================
// The gate
// ===============================================================================================

/// Every `VB-ZONE zones frame=N ids=[..]` line the worker printed.
fn parse_zone_ids(output: &str) -> Vec<(u32, Vec<u16>)> {
    let mut rows = Vec::new();
    for line in output.lines().filter(|l| l.contains("VB-ZONE zones ")) {
        let frame: u32 = line
            .split_whitespace()
            .find_map(|t| t.strip_prefix("frame="))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("a VB-ZONE zones line carries no frame=:\n  {line}"));
        let open = line
            .find("ids=[")
            .unwrap_or_else(|| panic!("a VB-ZONE zones line carries no ids=[:\n  {line}"));
        let rest = &line[open + "ids=[".len()..];
        let close = rest
            .find(']')
            .unwrap_or_else(|| panic!("an unterminated ids= list:\n  {line}"));
        let body = &rest[..close];
        let ids: Vec<u16> = if body.is_empty() {
            Vec::new()
        } else {
            body.split(',')
                .map(|t| {
                    t.trim()
                        .parse()
                        .unwrap_or_else(|_| panic!("an unparseable zone id {t:?}:\n  {line}"))
                })
                .collect()
        };
        rows.push((frame, ids));
    }
    rows
}

/// The family a zone id belongs to, or `None` when it belongs to none — which is the failure.
fn family_of(id: u16) -> Option<&'static str> {
    let in_range = |base: u16| id >= base && id < base + ZONE_FAMILY_WIDTH;
    if in_range(ZONE_BASE_VB) {
        Some("vb")
    } else if in_range(ZONE_BASE_GBUFFER) {
        Some("gbuffer")
    } else if in_range(ZONE_BASE_SV0) {
        Some("sv0")
    } else {
        None
    }
}

/// **The gate.** A Deferred frame's ported brackets execute, retire `Measured`, and carry ids in
/// the gbuffer/SV0 ranges.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); run with --test-threads=1"]
fn the_ported_gbuffer_brackets_execute_and_carry_their_own_family_ids() {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out = Command::new(&exe)
        .args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVER_MARKER, "1")
        .env("BOYKO_VB_ZONE", "1")
        .env("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES)
        .env("BOYKO_WINDOW_FRAMES", FRAME_CAP)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // `GpuSceneBundles::boot` refuses the two A/B knobs together; an operator's shell would
        // otherwise fail this gate at boot for a configuration it never asked for.
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_SV0_BENCH_NULL")
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_VG_SCENE")
        .env_remove("BOYKO_VG_OCC")
        .env_remove("BOYKO_VG_HZB")
        .output()
        .expect("invariant: the gbuffer zone worker process spawns");
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));

    if output.contains(NO_TIMESTAMPS) {
        eprintln!(
            "rung 6 port gate: INSTRUMENT-DEAD -- this device reports unusable timestamps, so the \
             zone recorder is never built and nothing here is a finding. Re-run on a \
             timestamp-capable device."
        );
        return;
    }

    // ---- clause 1: the run completed and retired frames --------------------------------------
    let rows = parse_zone_ids(&output);
    assert!(
        !rows.is_empty(),
        "the Deferred worker retired no zone frame at all. Either it never reached a presented \
         frame, or `record_gbuffer`'s brackets are not recording -- and note that a run which \
         brackets NOTHING releases its ring slots with `pairs == 0` and prints no ids, which is \
         exactly what an unported recorder looks like.\n---- worker output ----\n{output}"
    );

    // ---- clause 2: NON-VACUITY. Some frame actually bracketed something -------------------------
    let bracketed: Vec<&(u32, Vec<u16>)> = rows.iter().filter(|(_, ids)| !ids.is_empty()).collect();
    assert!(
        !bracketed.is_empty(),
        "every retired frame carried an EMPTY id list. Clause 3 below is a statement about ids, \
         and it is vacuously true of a frame that has none.\n---- worker output ----\n{output}"
    );

    // ---- clause 3: THE CLAIM. Every id names a family, and it is not VB's -----------------------
    //
    // `record_gbuffer` is the only recorder a Deferred frame runs, so a VB-range id here would mean
    // the bases are not doing their job -- which is precisely the collision they exist to prevent
    // (`TimedPass::DdgiUpdate`, `Sv0TimedPass::Marcher` and `VbTimedPass::CullReset` are all slot 0).
    let mut seen_gbuffer = 0usize;
    let mut seen_sv0 = 0usize;
    for (frame, ids) in &rows {
        for &id in ids {
            match family_of(id) {
                Some("gbuffer") => seen_gbuffer += 1,
                Some("sv0") => seen_sv0 += 1,
                Some("vb") => panic!(
                    "frame {frame}: zone id {id} is in the VB family's range on a DEFERRED frame, \
                     whose only recorder is `record_gbuffer`. Either a base is wrong or a bracket \
                     is using a bare pass slot as its id -- the collision `ZONE_BASE_*` exists to \
                     prevent.\n  ids: {ids:?}\n---- worker output ----\n{output}"
                ),
                _ => panic!(
                    "frame {frame}: zone id {id} belongs to NO declared family range. Every id is \
                     `base + slot` with `slot < ZONE_FAMILY_WIDTH`, so an id outside all three \
                     ranges means a family outgrew its width without the const-assert catching \
                     it.\n  ids: {ids:?}\n---- worker output ----\n{output}"
                ),
            }
        }
    }
    assert!(
        seen_gbuffer > 0,
        "not one bracket landed in the GBUFFER family range, so the four software-ray passes are \
         still unported on this configuration -- the SV0 marcher alone would satisfy clause 2.\n\
         ---- worker output ----\n{output}"
    );

    // ---- clause 4: the pairs came back MEASURED, not LOST or TORN -------------------------------
    let lost_or_torn: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("VB-ZONE retired "))
        .filter(|l| !l.contains("lost=0") || !l.contains("torn=0"))
        .collect();
    assert!(
        lost_or_torn.is_empty(),
        "retired frames report LOST or TORN pairs on a healthy device. TORN means a bracket opened \
         and never closed -- a port defect; LOST means the query never became available, which \
         against a working driver means the recorder wrote a query it never submitted.\n  {:?}\n\
         ---- worker output ----\n{output}",
        lost_or_torn
    );

    println!(
        "rung 6 port gate: {} retired frame(s), {seen_gbuffer} gbuffer-family and {seen_sv0} \
         SV0-family bracket(s), every id inside its own base range, no LOST and no TORN.",
        rows.len()
    );
}
