//! **G10 (profiling rung 5c) — the witness clause of the old-vs-new GPU collector A/B.**
//!
//! Two legs of one `VisibilityBuffer × Mesh` frame, recorded by two different collectors:
//!
//! | leg | knob | collector | readback |
//! |---|---|---|---|
//! | A | `BOYKO_VB_BENCH` | `VbTimestampCollector` (the one rung 7 deletes) | `VK_QUERY_RESULT_WAIT_BIT`, plus a totality epilogue |
//! | B | `BOYKO_VB_ZONE` | `GpuZoneRecorder` (the replacement) | `WITH_AVAILABILITY`, no wait, `NotBracketed` labels |
//!
//! The claim: **the two collectors put their brackets in the same places.** Measured as
//! `CommandWitness::stamp_positions` — the value of a monotone "record sites passed so far in
//! `record_vb`" counter at each bracket timestamp. It has **no vocabulary**, so unlike the record
//! order witness (`zone_open_order`, zone ids) it needs no hand-written `pass → zone` table to be
//! compared against a collector that has only `VbTimedPass` slots — and a table written alongside
//! the port would have made the comparison agree with itself.
//!
//! # Why TWO PROCESSES, where the corpus says "in one process"
//!
//! `05-LADDER-GATES.md`'s G10 row specifies *"K frames with only `VbTimestampCollector` armed, then
//! K frames with only `GpuZoneRecorder` armed, in one process, ABBA-ordered"*. This gate runs one
//! leg per process, and the deviation is not a shortcut — it is what keeps leg A honest.
//!
//! Leg A's readback is `read_vb_bench_ns`, which reads every pair with `VK_QUERY_RESULT_WAIT_BIT`.
//! A frame recorded by leg B writes leg A's queries **not at all**, so a single process alternating
//! legs would reach that readback on a frame whose pool was never written and **block forever** —
//! the exact hang class rung P4-1 removed and `vb_bench_totality_gate.rs` documents as the one
//! failure it cannot convert into a red. One boot, one leg, is what makes that unreachable.
//!
//! The ABBA ordering exists to cancel temporal drift between the two legs' **timings**, and G10's
//! timing clause is **deferred to rung 8**, where its band (`resolve`'s
//! `max(floor, twin, se_floor, measured quantum)`) exists — `Floor`/`Twin`/`resolve` are rung 8's
//! content, and a band invented here to be satisfiable would be a band picked to pass (F6 with the
//! sign flipped). A stream POSITION has no drift term: it is a function of the scene and the leg,
//! both of which are fixed in code. G10's own text settles which half licenses the deletion —
//! *"the witness clause, not the timing clause"* — and this file is that half.
//!
//! # What the RED has to be, and why the corpus's RED is not producible here
//!
//! The row prescribes *"shift a bracket by one command ⇒ one position differs"*. Injected at a
//! bracket SITE that would shift **both** legs equally and stay green, because the port did not
//! duplicate the sites: `TsWitness::begin`/`end` are one call each and dispatch to whichever
//! collector is armed underneath. That shared site is the design's strength (one instrument, two
//! recorders — the corpus's own point 4) and it means the RED must be **leg-specific**: add a
//! `ts.cmd()` inside `begin`'s `zr` arm only, and every zone-leg position moves by one while the
//! bench leg's stand still. Run at implementation, and recorded here because a RED that cannot be
//! produced as written is a gate nobody can re-verify.
//!
//! # What it cannot claim
//!
//! Nothing about the two collectors' NUMBERS: they write different queries in different pools, and
//! the P4-6 lesson is that timestamps cannot license record-order conclusions. Nothing about
//! pixels — no pin covers this fixture, and both legs render the same scene anyway. And nothing
//! about a frame the zone leg refused a ring slot for: that frame records no brackets, its census
//! line says `pairs=0`, and clause 3 below is what stops such a frame from greening the equality.
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko-app --features profiling-census --test vb_zone_ab_witness_gate -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! with `BOYKO_DISABLE_VALIDATION=1`. Without `--features profiling-census` the whole file
//! compiles to nothing: the ~200 census increments at `vb.rs`'s record sites are behind that
//! feature, so an unfeatured run would compare two all-zero streams and agree for the worst
//! possible reason.

#![cfg(all(windows, feature = "profiling-census"))]

use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
    generate_tangents,
};

/// The one worker. The DRIVER picks the leg by which knob it sets, so both legs run byte-identical
/// host code and the comparison is about the collectors rather than about two fixtures.
const WORKER: &str = "vb_zone_ab_worker";

/// Timed frames each leg collects past the runner's 20-frame warm-up.
const BENCH_FRAMES: &str = "10";

/// The automated-run frame cap, sized far above what either leg needs so it never pre-empts a
/// healthy run — and low enough that a leg which stops recording EXITS instead of spinning.
const FRAME_CAP: &str = "400";

/// The first frame index the comparison trusts.
///
/// Frame 0 has no previous frame's depth pyramid, so the occlusion legs record a different set of
/// passes than every steady frame does. That is a property of the SCENE, identical on both legs —
/// but comparing it would make the gate's subject "does frame 0 look like frame 0" rather than
/// "do the two collectors bracket the same places", and a fixture change could then red it for a
/// reason that has nothing to do with either collector.
const FIRST_STEADY_FRAME: u32 = 4;

/// The driver's private marker — `vb_bench_totality_gate.rs`'s own mechanism, for its own reason:
/// the operator running these gates has the leg knobs set in their shell, so an `--ignored` sweep
/// would otherwise boot the worker directly and render until killed.
const DRIVER_MARKER: &str = "BOYKO_VB_ZONE_AB_DRIVEN";

/// The boot notice printed when the DEVICE cannot serve timestamps. Neither leg is a finding then.
const NO_TIMESTAMPS: &str = "device timestamps are unusable";

// ===============================================================================================
// The worker
// ===============================================================================================

/// One UV sphere — `vb_mesh.rs`'s own generator, trimmed to one ring of stacks/slices.
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

/// A mesh scene, because the MESH LEG is where the brackets are.
///
/// `VisibilityBuffer × Sdf` (the totality gate's fixture) needs no assets at all — and brackets
/// only three of the ten passes, because the mesh-leg block never runs. Seven `NotBracketed` pairs
/// would make this gate's equality a statement about three brackets while reading as a statement
/// about the port. The geometry is procedural, so the stronger fixture still needs no asset file.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    // `register_mesh_vb` claims the Decision-0 geometry-table slot the VB geometry fetch needs;
    // the fallback is for a device whose table is not armed, where the resolver degrades to
    // Deferred and this gate skips on the missing census lines rather than on a wrong claim.
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

/// **THE WORKER**, run once per leg. It asserts nothing: only the driver can see a child's output.
#[test]
#[ignore = "needs a real windowed GPU device; the A/B driver spawns it once per leg"]
fn vb_zone_ab_worker() {
    if std::env::var(DRIVER_MARKER).is_err() {
        eprintln!(
            "{WORKER}: {DRIVER_MARKER} unset -- SKIPPED. This worker exists to be spawned by its \
             driver, which sets exactly one leg knob; booted without one it arms no collector and \
             nothing ends its frame loop."
        );
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb zone A/B", 512, 512));
    app.add_startup_system(setup);
    // Post-plugins owner override, the shape every render-path fixture uses.
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    app.run();
}

// ===============================================================================================
// The driver
// ===============================================================================================

/// One leg's census: `frame → the bracket stream positions that frame recorded`.
struct Leg {
    name: &'static str,
    output: String,
    frames: Vec<(u32, Vec<u32>)>,
    pairs: Vec<(u32, u32)>,
    stream_pos: Vec<(u32, u32)>,
}

/// `key=<value>` on `line`, as a `u32`.
///
/// Anchored to a whole whitespace-delimited TOKEN, not to a substring. A `line.find("pairs=")`
/// finds it inside `repairs=0` first and reads the repair count as the pair count — measured, on
/// the first live run of this gate, which then reported "every `pairs=` is 0" against a line
/// printing `pairs=10`. The instrument was wrong about which number it was reading, which is the
/// same defect class the census itself exists to catch one level down.
fn key_u32(line: &str, key: &str) -> Option<u32> {
    line.split_whitespace().find_map(|t| t.strip_prefix(key)).and_then(|v| v.parse().ok())
}

/// Every `VB-CENSUS` line this leg printed, parsed.
///
/// The parse is strict on purpose: a line whose `positions=[...]` cannot be read is a PARSE
/// failure, not an empty frame. Silently treating it as empty is how two legs come to "agree".
fn parse_leg(name: &'static str, output: String) -> Leg {
    let mut frames = Vec::new();
    let mut pairs = Vec::new();
    let mut stream_pos = Vec::new();
    for line in output.lines().filter(|l| l.contains("VB-CENSUS ")) {
        let frame = key_u32(line, "frame=")
            // `key_u32` is token-anchored, so `frame=` cannot match inside another key; the
            // `expect` below is about a MALFORMED line, not an ambiguous one.
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
        pairs.push((frame, key_u32(line, "pairs=").unwrap_or(0)));
        stream_pos.push((frame, key_u32(line, "stream_pos=").unwrap_or(0)));
    }
    Leg { name, output, frames, pairs, stream_pos }
}

/// Spawns the worker with exactly one leg knob set.
fn spawn_leg(name: &'static str, knob: &str) -> Leg {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVER_MARKER, "1")
        .env(knob, "1")
        .env("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES)
        .env("BOYKO_WINDOW_FRAMES", FRAME_CAP)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // THE OTHER LEG. `GpuSceneBundles::boot` refuses the two knobs together with an assert, so
        // an operator's shell variable would otherwise fail this gate at boot with a message about
        // a configuration the driver never asked for.
        .env_remove(if knob == "BOYKO_VB_BENCH" { "BOYKO_VB_ZONE" } else { "BOYKO_VB_BENCH" })
        // Every capture driver has its own exit rule; arming one here would give a red two causes.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_SV0_BENCH_NULL")
        // Scene-shape knobs from an ambient shell would change WHICH passes record, on one leg or
        // both depending on when they were set. Removed so the two legs see one scene.
        .env_remove("BOYKO_VG_SCENE")
        .env_remove("BOYKO_VG_OCC")
        .env_remove("BOYKO_VG_HZB")
        .env_remove("BOYKO_VB_BENCH_LIGHTS")
        .env_remove("BOYKO_VB_BENCH_GRID")
        .env_remove("BOYKO_VB_BENCH_RIG")
        .env_remove("BOYKO_VB_FROXEL_FORCE_OFF");
    let out = cmd.output().expect("invariant: the A/B worker process spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    parse_leg(name, merged)
}

/// **G10's witness clause.** The two collectors bracket the same places in the same stream.
#[test]
#[ignore = "live GPU gate (spawns two windowed workers); run with --test-threads=1"]
fn the_two_collectors_put_their_brackets_at_the_same_stream_positions() {
    let a = spawn_leg("leg A (VbTimestampCollector)", "BOYKO_VB_BENCH");
    if a.output.contains(NO_TIMESTAMPS) {
        eprintln!(
            "G10: INSTRUMENT-DEAD -- this device reports unusable timestamps, so neither leg arms \
             a collector and the comparison is not a finding about rung 5c. Re-run on a \
             timestamp-capable device."
        );
        return;
    }
    let b = spawn_leg("leg B (GpuZoneRecorder)", "BOYKO_VB_ZONE");

    // ---- clause 1: BOTH legs produced a census at all ------------------------------------------
    for leg in [&a, &b] {
        assert!(
            !leg.frames.is_empty(),
            "{} printed no VB-CENSUS line. Either the worker never reached a presented frame, or \
             the census was not threaded into `record_vb` on this leg at all -- and two legs that \
             printed nothing would agree perfectly.\n---- worker output ----\n{}",
            leg.name,
            leg.output
        );
    }

    // ---- clause 2: the census was COMPILED IN. Without the feature every counter is 0 -----------
    //
    // The positions are read from a build where `ts.cmd()` expands to nothing unless
    // `profiling-census` reached `boyko_rhi_vulkan`. This file is `#[cfg]`-gated on `boyko-app`'s
    // own forwarding feature -- but features unify per package, and "the flag I named reached the
    // crate that acts on it" is exactly the kind of thing that is true until someone edits a
    // manifest. A non-zero stream position is the evidence, taken from the run.
    for leg in [&a, &b] {
        let moved = leg.stream_pos.iter().any(|(_, p)| *p > 0);
        assert!(
            moved,
            "{}: every VB-CENSUS line reports stream_pos=0, so the ~200 increments at `vb.rs`'s \
             record sites compiled to nothing -- `boyko_rhi_vulkan/profiling-census` did not reach \
             the recorder. The positions below would all be [] and would compare EQUAL.\n\
             ---- worker output ----\n{}",
            leg.name,
            leg.output
        );
    }

    // ---- clause 3: NON-VACUITY. Steady frames actually bracketed something ----------------------
    //
    // Two empty position lists are equal. A zone leg that refused every ring slot, or a bench leg
    // whose collector was disarmed, would satisfy clause 4 perfectly while measuring nothing.
    for leg in [&a, &b] {
        let bracketed = leg
            .pairs
            .iter()
            .filter(|(f, _)| *f >= FIRST_STEADY_FRAME)
            .filter(|(_, p)| *p > 0)
            .count();
        assert!(
            bracketed > 0,
            "{}: not one steady frame recorded a bracket (every `pairs=` is 0 from frame \
             {FIRST_STEADY_FRAME} on). An equality over empty streams is not evidence.\n\
             ---- worker output ----\n{}",
            leg.name,
            leg.output
        );
    }

    // ---- clause 4: THE CLAIM. Same count, same positions, frame for frame ----------------------
    let mut compared = 0usize;
    let mut stamps_seen = 0usize;
    for (frame, pa) in a.frames.iter().filter(|(f, _)| *f >= FIRST_STEADY_FRAME) {
        let Some((_, pb)) = b.frames.iter().find(|(f, _)| f == frame) else { continue };
        assert_eq!(
            pa.len(),
            pb.len(),
            "frame {frame}: leg A recorded {} bracket timestamps and leg B recorded {}. The two \
             collectors bracket a DIFFERENT NUMBER of passes, which no position comparison can \
             excuse -- and note that the old collector's totality epilogue does NOT count here \
             (its fills go through `CommandWitness::repair`), so this is a difference in BRACKETS.\
             \n  A: {pa:?}\n  B: {pb:?}",
            pa.len(),
            pb.len()
        );
        assert_eq!(
            pa, pb,
            "frame {frame}: the two collectors' brackets sit at DIFFERENT positions in the \
             recorded command stream. Each entry is the number of record sites `record_vb` passed \
             before that timestamp, so a single differing entry means one leg opened or closed a \
             bracket at a different point in the same frame.\n  A: {pa:?}\n  B: {pb:?}"
        );
        compared += 1;
        stamps_seen += pa.len();
    }
    assert!(
        compared > 0,
        "the two legs share no steady frame index (A: {} frames, B: {} frames, first steady \
         {FIRST_STEADY_FRAME}). With nothing compared, clause 4 is vacuous.",
        a.frames.len(),
        b.frames.len()
    );

    println!(
        "G10 witness clause: {compared} frame(s) compared, {stamps_seen} bracket timestamp(s), \
         every position identical between VbTimestampCollector and GpuZoneRecorder. \
         The TIMING clause is deferred to rung 8, where its band exists."
    );
}
