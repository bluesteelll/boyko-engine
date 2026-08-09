//! **Profiling rung 6 — the gbuffer + SV0 brackets, ported and executed.**
//!
//! Rung 5c ported `record_vb`'s ten brackets to the `GpuZoneRecorder` and G10 compared them against
//! the old collector. Rung 6 ports `record_gbuffer`'s: the four software-ray passes
//! (`TimedPass::{DdgiUpdate, DeferredResolve, CsmDepth, PunctualDepth}`) and the Deferred fine
//! marcher (`Sv0TimedPass::Marcher`). This file is the gate that those brackets **run**, on a real
//! `Deferred × Both` frame, and land where the id scheme says they should.
//!
//! # Two gates here, and the fork the second one resolved
//!
//! G10's shape is a cross-leg comparison, and when the port landed the gbuffer family had **no host
//! arming path for its old leg**: `TimestampCollector` was constructed in exactly two RHI *test*
//! files and never from `boyko_app`, so there was no worker to play leg A. The two branches were
//! (a) give the R0 collector a host knob, or (b) move the A/B into the RHI test that owns it.
//!
//! **(a), chosen on performance.** (b) needs the scene to hold `&GpuZoneRecorder` across 220 frames
//! while the ring slot changes per frame, so `open_frame` — and with it `retire` — would have to
//! take `&self`. That deletes clause (c) of `FrameSlot`'s `Sync` argument (*"`retire` takes
//! `&mut self`, so no recording call can be in flight against the same slot"*) **permanently, in
//! shipped code**, and pushes `set_mark` toward a locked read-modify-write in a hot recorder — a
//! cost paid by the engine to serve a test's borrow shape. (a) costs one boot-time `Option` that is
//! `None` in every shipped run and one predicate in `GpuSceneBundles::scene`, which is exactly what
//! the two existing bench knobs already are; and rung 7 deletes the collector and the knob together.
//!
//! So this file carries both halves: the PORT gate (ids in their own family range, nothing lost or
//! torn) and the WITNESS clause of G10 for these passes (same brackets, same stream positions).
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

// ===============================================================================================
// G10's witness clause, for the gbuffer + SV0 families
// ===============================================================================================

/// The first frame index the comparison trusts — `vb_zone_ab_witness_gate.rs`'s constant and its
/// reason: the earliest frames have no previous depth pyramid and record a different pass set.
const FIRST_STEADY_FRAME: u32 = 4;

/// One leg's census, keyed by frame.
struct Leg {
    name: &'static str,
    output: String,
    frames: Vec<(u32, Vec<u32>)>,
    stream_pos: Vec<(u32, u32)>,
    resets: Vec<(u32, u32)>,
}

/// `key=<value>` on `line`, anchored to a whole TOKEN.
///
/// Token-anchored for the reason `vb_zone_ab_witness_gate.rs` records against itself: a
/// `find("pairs=")` matches inside `repairs=0` and reads the repair count as the pair count.
fn key_u32(line: &str, key: &str) -> Option<u32> {
    line.split_whitespace().find_map(|t| t.strip_prefix(key)).and_then(|v| v.parse().ok())
}

/// Parses every `VB-CENSUS` line, strictly: an unreadable list is a PARSE failure, not an empty
/// frame — silently treating it as empty is how two legs come to "agree".
fn parse_leg(name: &'static str, output: String) -> Leg {
    let mut frames = Vec::new();
    let mut stream_pos = Vec::new();
    let mut resets = Vec::new();
    for line in output.lines().filter(|l| l.contains("VB-CENSUS ")) {
        let frame = key_u32(line, "frame=")
            .unwrap_or_else(|| panic!("{name}: a VB-CENSUS line carries no frame=:\n  {line}"));
        let open = line
            .find("positions=[")
            .unwrap_or_else(|| panic!("{name}: a VB-CENSUS line carries no positions=[:\n  {line}"));
        let rest = &line[open + "positions=[".len()..];
        let close = rest
            .find(']')
            .unwrap_or_else(|| panic!("{name}: an unterminated positions= list:\n  {line}"));
        let body = &rest[..close];
        let positions: Vec<u32> = if body.is_empty() {
            Vec::new()
        } else {
            body.split(',')
                .map(|t| {
                    t.trim().parse().unwrap_or_else(|_| {
                        panic!("{name}: an unparseable stream position {t:?}:\n  {line}")
                    })
                })
                .collect()
        };
        frames.push((frame, positions));
        stream_pos.push((frame, key_u32(line, "stream_pos=").unwrap_or(0)));
        resets.push((frame, key_u32(line, "resets=").unwrap_or(0)));
    }
    Leg { name, output, frames, stream_pos, resets }
}

/// Spawns the Deferred worker with one leg's knobs set.
///
/// `knobs` is a LIST because leg A is two collectors: the zone recorder brackets every family
/// `record_gbuffer` records, so a leg-A worker arming only the R0 collector would record one bracket
/// against leg B's two and the comparison would red on the arming rather than on the port.
fn spawn_leg(name: &'static str, knobs: &[&str]) -> Leg {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVER_MARKER, "1")
        .env("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES)
        .env("BOYKO_WINDOW_FRAMES", FRAME_CAP)
        // The SV0 bench asserts, BEFORE the first frame, that the frame cap is at least
        // `20 + 4 * quads`; at its 200-quad default that is 820 frames, well past this gate's cap.
        // Lowering the quad count is the knob its own message names, and this gate reads a CENSUS
        // rather than the bench's statistics, so five quadruples is plenty: they exist to give the
        // A/B enough presented frames, not to measure anything.
        .env("BOYKO_SV0_BENCH_QUADS", "5")
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // Start from a clean slate and set only this leg's knobs, so an operator's shell cannot
        // arm the other side — `GpuSceneBundles::boot` refuses zone-plus-old with an assert, and a
        // gate that failed at boot would look exactly like a gate that failed at its claim.
        .env_remove("BOYKO_VB_ZONE")
        .env_remove("BOYKO_GBUF_BENCH")
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_SV0_BENCH_NULL")
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_VG_SCENE")
        .env_remove("BOYKO_VG_OCC")
        .env_remove("BOYKO_VG_HZB");
    for k in knobs {
        cmd.env(k, "1");
    }
    let out = cmd.output().expect("invariant: the A/B worker process spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    parse_leg(name, merged)
}

/// **G10's witness clause, extended to the gbuffer + SV0 passes.**
///
/// Same claim as rung 5c's for the VB passes: the old collectors and the `GpuZoneRecorder` put
/// their brackets at the same positions in the same recorded command stream. The TIMING clause
/// stays deferred to rung 8, where its band exists.
#[test]
#[ignore = "live GPU gate (spawns two windowed workers); run with --test-threads=1"]
fn the_gbuffer_collectors_put_their_brackets_at_the_same_stream_positions() {
    // Leg A is BOTH old collectors — see `spawn_leg`.
    let a = spawn_leg("leg A (TimestampCollector + Sv0TimestampCollector)", &[
        "BOYKO_GBUF_BENCH",
        "BOYKO_SV0_BENCH",
    ]);
    if a.output.contains(NO_TIMESTAMPS) {
        eprintln!(
            "G10 (gbuffer): INSTRUMENT-DEAD -- this device reports unusable timestamps, so neither \
             leg arms a collector and the comparison is not a finding about rung 6."
        );
        return;
    }
    let b = spawn_leg("leg B (GpuZoneRecorder)", &["BOYKO_VB_ZONE"]);

    // ---- clause 1: both legs produced a census -------------------------------------------------
    for leg in [&a, &b] {
        assert!(
            !leg.frames.is_empty(),
            "{} printed no VB-CENSUS line. Either the worker never reached a presented frame, or \
             the census was not threaded into `record_gbuffer` on this leg -- and two legs that \
             printed nothing would agree perfectly.\n---- worker output ----\n{}",
            leg.name,
            leg.output
        );
    }

    // ---- clause 2: the census was COMPILED IN --------------------------------------------------
    for leg in [&a, &b] {
        assert!(
            leg.stream_pos.iter().any(|(_, p)| *p > 0),
            "{}: every VB-CENSUS line reports stream_pos=0, so the 125 increments at \
             `gbuffer.rs`'s record sites compiled to nothing -- `profiling-census` did not reach \
             the recorder, and the positions below would all be [] and compare EQUAL.\n\
             ---- worker output ----\n{}",
            leg.name,
            leg.output
        );
    }

    // ---- clause 3: NON-VACUITY --------------------------------------------------------------
    for leg in [&a, &b] {
        let bracketed = leg
            .frames
            .iter()
            .filter(|(f, _)| *f >= FIRST_STEADY_FRAME)
            .filter(|(_, p)| !p.is_empty())
            .count();
        assert!(
            bracketed > 0,
            "{}: not one steady frame recorded a bracket. An equality over empty streams is not \
             evidence.\n---- worker output ----\n{}",
            leg.name,
            leg.output
        );
    }

    // ---- clause 4: THE CLAIM, with the ONE difference the port actually makes ------------------
    //
    // MEASURED on the first live run: leg A's positions were each EXACTLY ONE higher than leg B's.
    // Not a port defect — a true difference in the recorded stream, and precisely the one this rung
    // exists to make. The old side records TWO `vkCmdResetQueryPool`s per frame, one per collector
    // (`TimestampCollector` and `Sv0TimestampCollector` own separate pools); the zone side records
    // ONE, because it is one recorder with one pool. The census counts a reset as the recorded
    // command it is, so every later position shifts by the difference.
    //
    // A plain equality reds on that, which is why rung 5c's VB gate never saw it: there the old
    // side is one collector against one recorder, one reset each.
    //
    // The honest form is TWO clauses, and neither hides anything:
    //   (i) the per-frame offset is CONSTANT across the frame's brackets — no bracket moved
    //       relative to its neighbours, which is the port claim;
    //  (ii) that constant EQUALS `resets_A - resets_B`, taken from the census's own counters — the
    //       prologue is the whole of the difference, and nothing else moved.
    // Comparing `p[i] - p[0]` would have satisfied (i) alone and quietly accepted any prologue
    // difference at all, including one nobody intended.
    let mut compared = 0usize;
    let mut stamps_seen = 0usize;
    for (frame, pa) in a.frames.iter().filter(|(f, _)| *f >= FIRST_STEADY_FRAME) {
        let Some((_, pb)) = b.frames.iter().find(|(f, _)| f == frame) else { continue };
        assert_eq!(
            pa.len(),
            pb.len(),
            "frame {frame}: leg A recorded {} bracket timestamps and leg B recorded {}. The two \
             sides bracket a DIFFERENT NUMBER of passes -- and note leg A arms BOTH old collectors \
             precisely so this cannot differ because of the arming.\n  A: {pa:?}\n  B: {pb:?}",
            pa.len(),
            pb.len()
        );
        if pa.is_empty() {
            continue;
        }
        // (i) one offset for the whole frame.
        let offset = pa[0] as i64 - pb[0] as i64;
        for (i, (x, y)) in pa.iter().zip(pb.iter()).enumerate() {
            assert_eq!(
                *x as i64 - *y as i64,
                offset,
                "frame {frame}: bracket timestamp {i} sits {} record site(s) apart between the two \
                 sides while the frame's other brackets sit {offset} apart. A bracket moved \
                 RELATIVE to its neighbours, which no difference in the frame prologue can \
                 explain.\n  A: {pa:?}\n  B: {pb:?}",
                *x as i64 - *y as i64
            );
        }
        // (ii) and that offset is the reset-count difference, from the instrument's own counters.
        let ra = a.resets.iter().find(|(f, _)| f == frame).map(|(_, r)| *r).unwrap_or(0) as i64;
        let rb = b.resets.iter().find(|(f, _)| f == frame).map(|(_, r)| *r).unwrap_or(0) as i64;
        assert_eq!(
            offset,
            ra - rb,
            "frame {frame}: the two sides' brackets are {offset} record site(s) apart, but their \
             query-pool resets differ by {} ({ra} on the old side, {rb} on the zone side). The \
             prologue is supposed to be the WHOLE of the difference; an offset larger than the \
             reset delta means something else was recorded on one side and not the other.\n  \
             A: {pa:?}\n  B: {pb:?}",
            ra - rb
        );
        compared += 1;
        stamps_seen += pa.len();
    }
    assert!(
        compared > 0,
        "the two legs share no steady frame index (A: {} frames, B: {} frames). With nothing \
         compared, clause 4 is vacuous.",
        a.frames.len(),
        b.frames.len()
    );

    println!(
        "G10 witness clause (gbuffer + SV0): {compared} frame(s) compared, {stamps_seen} bracket \
         timestamp(s). Every bracket sits at the same position on both sides once the frame \
         prologue is accounted for -- the old side records one query-pool reset per collector and \
         the zone side records one for the frame, and that difference is asserted to be the WHOLE \
         of the offset. The TIMING clause is deferred to rung 8."
    );
}
