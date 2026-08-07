//! **VG R3 piece 4 rung P4-1 — the two headline gates, committed.**
//!
//! P4-1 shipped two RELEASE-LIVE mechanisms, and each one is defined by the *reachable
//! configuration whose outcome it changes*. This file is those two configurations, executed:
//!
//! | gate | configuration | BEFORE P4-1 | AFTER P4-1 |
//! |---|---|---|---|
//! | A | `VisibilityBuffer × Sdf` + `BOYKO_VB_BENCH=1` | **PANICS** before frame 1 (`runner.rs`'s `mesh_leg` `assert!`) | completes; `vb_shade` reports `FALLBACK` |
//! | B | `Deferred × Both` + `BOYKO_VB_BENCH=1` | **HANGS FOREVER** (the pool is never even reset; the `WAIT_BIT` readback never returns) | exits immediately with a named message |
//!
//! # Why this file exists rather than a runbook line
//!
//! Both gates were first demonstrated ad hoc, from a shell. A gate that exists only as a chat-log
//! one-liner is a gate nobody re-runs — this campaign has measured that failure mode directly. The
//! mechanisms P4-1 ships (`TsWitness::finish`, the totality epilogue; and
//! `GpuSceneBundles::disarm_vb_bench_unless_vb`, the structural disarm) are both **release-live**
//! precisely because a `debug_assert!` cannot protect a release bench run, so their evidence must
//! be executable too.
//!
//! # What each gate CANNOT claim
//!
//! **Gate A cannot claim any number is meaningful.** It asserts only that the instrument LABELS
//! HONESTLY: a pass the recorder never bracketed comes back flagged `FALLBACK` with a duration at
//! the lattice floor, instead of being averaged into an aggregate as a small real cost. It says
//! nothing about whether the two MEASURED brackets measure the right extent, nothing about the
//! begin offsets being interpretable (that is `vg_occ_split_timing.rs`'s job, from rung P4-6), and
//! nothing about the frame's pixels — this fixture renders `vb_sdf_only.rs`'s sky and no golden
//! covers what it draws here.
//!
//! **Gate B cannot claim anything about the VB path.** It exercises the boot-time gate on a
//! NON-VisibilityBuffer path, where `record_vb` — and therefore every timestamp write, the pool
//! reset and the epilogue — never runs at all. It says the knob is refused loudly on a path that
//! cannot feed it. It does not say the bench works on a path that can.
//!
//! **Neither gate covers a golden frame.** No pin sets `BOYKO_VB_BENCH`, so on every pinned run
//! the collector is `None`, the witness records zero commands and the epilogue executes zero
//! times. These two workers are the only configurations in the tree that execute the epilogue at
//! all.
//!
//! # The one failure mode this file cannot convert into a red
//!
//! If the epilogue itself regresses, worker A does not fail — it **hangs**, inside
//! `vkGetQueryPoolResults`, waiting on a query its recorder never wrote. No host-side frame cap
//! can reach a driver call that never returns, and this repository has no kill-after-timeout
//! pattern to borrow. That hang IS the defect class P4-1 removes, and it is stated here rather
//! than papered over: a worker A that never terminates is a RED whose message is its own silence.
//! Worker B's cap (`BOYKO_WINDOW_FRAMES`) does help, because its regression mode is "runs
//! forever", not "blocks in a driver call" — unless BOTH halves regress, in which case B hangs
//! exactly as it did before this rung.
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko_app --test vb_bench_totality_gate -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! with `BOYKO_DISABLE_VALIDATION=1`. Each driver spawns its own worker and sets every variable
//! that worker needs, so the command above is safe to run with `BOYKO_VB_BENCH` already set in the
//! shell: the sweep reaches both workers too, and they SKIP unless their driver spawned them (see
//! [`DRIVER_MARKER`] — without that guard the sweep would boot worker B directly, which is
//! SUPPOSED to panic, failing the binary while both gates passed).

#![cfg(windows)]

use std::process::Command;

use boyko_app::prelude::*;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// The `VisibilityBuffer × Sdf` worker (gate A).
const WORKER_A: &str = "vb_bench_totality_vb_sdf_worker";

/// The `Deferred × Both` worker (gate B).
const WORKER_B: &str = "vb_bench_totality_deferred_worker";

/// TIMED frames each bench worker collects past `VB_BENCH_WARMUP` (20). Small on purpose: this is
/// a LABEL gate, not a measurement, so the only thing the count buys is the print.
const BENCH_FRAMES: &str = "10";

/// The automated-run frame cap (`boyko_app::runner`'s `BOYKO_WINDOW_FRAMES`), sized far above the
/// `20 + 10` PRESENTED frames the bench needs so it never pre-empts a healthy run. It exists so a
/// worker that renders but never completes its bench EXITS and reds on a missing line, instead of
/// spinning until the operator notices.
const FRAME_CAP: &str = "400";

/// The plan's own control-(iii) clause, in ns: a `FALLBACK` pair is two `BOTTOM_OF_PIPE` stamps
/// written back to back with NOTHING recorded between them, so its delta is the timestamp
/// counter's lattice quantisation.
///
/// ⚠️ Deliberately a BOUND and not `== 0.0`. The observed value on the machine this rung was
/// authored against is exactly `0.0`, but two adjacent stamps differing by one lattice tick is
/// legal hardware behaviour — pinning the literal would encode one driver's quantum into a gate
/// and red on a correct engine elsewhere. The observed value is PRINTED by the passing gate, so a
/// drift away from `0.0` is visible in the log without failing anything. Control (iii) of the plan
/// (implement the fallback with `write_begin`+`write_end` instead of `write_zero_pair`) reds
/// against this bound by orders of magnitude: it would report the whole frame's drain time.
const FALLBACK_MAX_NS: f64 = 1_000.0;

/// The `#[cold]` boot notice `disarm_vb_bench_unless_vb` prints, including the resolved path —
/// gate B's FIRST required substring.
const DISARM_NOTE: &str = "resolved render path is Deferred";

/// The panic's invariant text — gate B's SECOND required substring, deliberately DISTINCT from the
/// note so an unrelated crash (a device-lost, a plugin panic, a missing asset) cannot green this
/// gate by printing something that happens to mention the knob.
const DISARM_PANIC: &str = "invariant: BOYKO_VB_BENCH requires RenderPath::VisibilityBuffer";

/// The boot notice the `O2` decline path prints when the DEVICE cannot serve timestamps at all.
/// Neither gate is a finding on such a device — see [`instrument_dead`].
const NO_TIMESTAMPS: &str = "device timestamps are unusable";

// ===============================================================================================
// The two workers
// ===============================================================================================

/// `vb_sdf_only.rs`'s framing — a sun, a sky and a camera, and NO mesh entities.
///
/// Copied rather than shared because nothing here is load-bearing: these gates assert on printed
/// LABELS, never on pixels, so this framing may drift from `vb_sdf_only.rs` without either gate
/// meaning anything different. What IS load-bearing is that no mesh entity is spawned — under
/// `GeometryLegs::Sdf` the resolved path carries `mesh_leg == false`, which is the configuration
/// gate A exists to reach.
fn setup(mut commands: Commands) {
    const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

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

/// The driver's private marker. Not an engine knob and not read by any shipping code: it is how a
/// worker tells "my driver spawned me" from "an `--ignored` sweep reached me".
///
/// Keying the skip on `BOYKO_VB_BENCH` alone is not enough, and the difference is a FALSE RED, not
/// a nicety: the operator who runs these gates has that variable set in their own shell (it is the
/// knob under test), so a `-- --ignored` sweep would boot both workers directly — and worker B is
/// SUPPOSED to panic, which would fail the binary while both gates passed.
const DRIVER_MARKER: &str = "BOYKO_VB_TOTALITY_DRIVEN";

/// A worker booted without `BOYKO_VB_BENCH` arms no collector, so nothing terminates its frame
/// loop: it renders until killed. Both workers therefore SKIP unless their driver spawned them
/// with the knob set.
fn skip_unless_armed(worker: &str) -> bool {
    if std::env::var(DRIVER_MARKER).is_ok() && std::env::var("BOYKO_VB_BENCH").is_ok() {
        return false;
    }
    eprintln!(
        "{worker}: {DRIVER_MARKER} and/or BOYKO_VB_BENCH unset -- SKIPPED. This worker exists to \
         be spawned by its driver; booted without the knob it would render forever, because the \
         bench's own frame budget is the only thing that ends its loop. To run it by hand, set \
         BOTH variables."
    );
    true
}

/// **WORKER A** — `VisibilityBuffer × Sdf` with the bench armed.
///
/// The configuration that PANICKED before this rung: `record_vb` runs (the path is VB) but the
/// `mesh_leg` block never does, so no lit producer is bracketed and `VbTimedPass::VbShade` was
/// reset-but-never-written. The boot-time `assert!` that stood in for the missing per-frame
/// invariant has been replaced by the recorder's totality epilogue plus a printed scope note.
///
/// This worker asserts NOTHING itself — its whole contract is to complete and print. The
/// assertions live in the driver, which is the only side that can see the child's exit status.
#[test]
#[ignore = "needs a real windowed GPU device; the totality driver spawns it with BOYKO_VB_BENCH set"]
fn vb_bench_totality_vb_sdf_worker() {
    if skip_unless_armed(WORKER_A) {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb bench totality (vb x sdf)", 512, 512));
    app.add_startup_system(setup);
    // Inserted AFTER `add_plugins` so it wins over `RenderPathPlugin`'s `Deferred` default — the
    // post-plugins owner-override shape every render-path fixture uses.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Sdf });
    app.run();
}

/// **WORKER B** — `Deferred × Both` with the bench armed.
///
/// The configuration that HUNG before this rung, and the sibling hazard the epilogue structurally
/// cannot reach: the collector arms on the env knob plus device support ALONE, while every writer
/// — and the pool reset — lives inside `record_vb`, which the frame driver calls only under
/// `scene.path_is_vb()`. A Deferred boot therefore passed both of the runner's surviving
/// preconditions (`mesh_leg` is TRUE on `Deferred × Both`!), recorded nothing, reset nothing, and
/// blocked forever in the `WAIT_BIT` readback.
///
/// Expected outcome: the process FAILS at boot, having printed both the disarm note and the panic.
#[test]
#[ignore = "needs a real windowed GPU device; the totality driver spawns it with BOYKO_VB_BENCH set"]
fn vb_bench_totality_deferred_worker() {
    if skip_unless_armed(WORKER_B) {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb bench totality (deferred x both)", 512, 512));
    app.add_startup_system(setup);
    // `Deferred × Both` — `sv0_deferred_term_bench.rs`'s own configuration, minus its SV0 knobs
    // and its scene. The scene is irrelevant here: the gate fires in `run_windowed` the instant
    // the render path is resolved, before the first frame.
    app.insert_resource(RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Both });
    app.run();
}

// ===============================================================================================
// The drivers
// ===============================================================================================

/// Spawns `worker` in this same test binary with the bench armed, returning
/// `(stdout ++ stderr, exited_successfully)`.
///
/// Both streams are needed and are MERGED on purpose: the `VB-P4` lines are `println!` (stdout)
/// while the scope note, the disarm note and the panic are `eprintln!`/panic (stderr), and no
/// clause below cares which stream carried its evidence — only that the process emitted it. A gate
/// that read one stream would silently stop seeing half of what it asserts on.
///
/// The removals are as load-bearing as the settings. `BOYKO_SV0_BENCH` in particular MUST go: the
/// runner refuses the two benches together with a panic of its own, which would fail worker B for
/// a completely different reason while looking exactly like success.
fn spawn_worker(worker: &str, extra: &[(&str, &str)]) -> (String, bool) {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVER_MARKER, "1")
        .env("BOYKO_VB_BENCH", "1")
        .env("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // Every capture driver has its own exit rule; arming one here would give a red two
        // possible causes and could end the run before the bench prints.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VG_CENSUS")
        // The OTHER bench. See this fn's doc.
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_SV0_BENCH_NULL")
        // Bench-shape knobs from an operator's shell would change the printed `N_ps=` label and
        // the light rig; neither is asserted here, but a worker whose scene depends on the
        // ambient environment is not reproducible.
        .env_remove("BOYKO_VB_BENCH_LIGHTS")
        .env_remove("BOYKO_VB_BENCH_GRID")
        .env_remove("BOYKO_VB_BENCH_RIG")
        .env_remove("BOYKO_VB_FROXEL_FORCE_OFF");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("invariant: the totality worker process spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    (merged, out.status.success())
}

/// A device that cannot serve timestamps arms no collector at all, so NEITHER gate is a statement
/// about this rung there. Printed loudly and skipped — never green, never red (the
/// three-outcome discipline: an instrument that did not run is not evidence that it works).
fn instrument_dead(output: &str, gate: &str) -> bool {
    if !output.contains(NO_TIMESTAMPS) {
        return false;
    }
    eprintln!(
        "{gate}: INSTRUMENT-DEAD -- this device reports unusable timestamps, so BOYKO_VB_BENCH \
         arms no collector and neither the epilogue nor the disarm is exercised. This is not a \
         finding about VG R3 piece 4; re-run on a timestamp-capable device."
    );
    true
}

/// The `f64` after `key` on `line`, e.g. `median_ns=` → `0.0`.
fn key_f64(line: &str, key: &str) -> Option<f64> {
    let at = line.find(key)? + key.len();
    line[at..].split_whitespace().next()?.parse().ok()
}

/// **GATE A** — the epilogue labels an unbracketed pass honestly instead of hanging on it.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); the orchestrator runs it with --test-threads=1"]
fn vb_sdf_leg_completes_with_vb_shade_flagged_fallback() {
    let (output, success) = spawn_worker(WORKER_A, &[("BOYKO_WINDOW_FRAMES", FRAME_CAP)]);
    if instrument_dead(&output, "gate A") {
        return;
    }

    // ---- clause 1: it COMPLETED. Before this rung it panicked here, by design -----------------
    assert!(
        success,
        "the VisibilityBuffer x Sdf bench worker did not exit successfully.\n\
         BEFORE VG R3 piece 4 rung P4-1 this was the EXPECTED outcome: a release-live `assert!` in \
         `boyko_app::runner` refused the whole configuration because `record_vb` brackets no lit \
         producer without a mesh leg, and the WAIT_BIT readback would have blocked on the \
         unwritten pair. A failure here means either that assert is back, or the totality epilogue \
         (`TsWitness::finish`) stopped closing the pairs the leg leaves open.\n\
         ---- worker output ----\n{output}"
    );

    // ---- clause 2: the unbracketed pass is FLAGGED, not silently averaged in -------------------
    let shade = output
        .lines()
        .find(|l| l.contains("VB-P4 pass=vb_shade "))
        .unwrap_or_else(|| {
            panic!(
                "the worker completed but printed no `VB-P4 pass=vb_shade` line. The bench either \
                 never reached its {BENCH_FRAMES}-frame budget (the run hit its \
                 BOYKO_WINDOW_FRAMES={FRAME_CAP} cap first) or the per-pass summary is gone. A \
                 completed run with no per-pass line is an instrument failure, not a measurement.\n\
                 ---- worker output ----\n{output}"
            )
        });
    assert!(
        shade.contains("FALLBACK"),
        "the vb_shade pass came back UNFLAGGED on a leg that brackets no lit producer:\n  \
         {shade}\n\
         On `VisibilityBuffer x Sdf` the mesh-leg block never runs, so the recorder's witness must \
         report (begun=0, ended=0) for this pass and the epilogue must fill it at the frame end. \
         An unflagged line means a fabricated ~0 is about to be averaged into an aggregate as a \
         real cost -- the exact failure the FALLBACK label exists to prevent.\n\
         ---- worker output ----\n{output}"
    );
    let median = key_f64(shade, "median_ns=").unwrap_or_else(|| {
        panic!("the vb_shade line carries no parseable `median_ns=`:\n  {shade}")
    });
    assert!(
        median < FALLBACK_MAX_NS,
        "the FALLBACK vb_shade pair reports median_ns={median}, above the {FALLBACK_MAX_NS} ns \
         lattice bound. `write_zero_pair` writes BOTH queries at BOTTOM_OF_PIPE back to back with \
         nothing between them, so the delta can only be the counter's quantisation. A large value \
         means the fallback is being written as a TOP/BOTTOM pair again (the plan's control iii), \
         which reports the whole frame's DRAIN TIME as this pass's cost -- a large, \
         plausible-looking, fabricated number.\n  {shade}"
    );

    // ---- clause 3: NON-VACUITY. The epilogue did not simply zero everything --------------------
    //
    // Without this, an epilogue that filled ALL THREE pairs (a witness that never records a bit, a
    // collector wired to nothing) would green clause 2 while measuring nothing at all.
    let measured: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("VB-P4 pass="))
        .filter(|l| !l.contains("FALLBACK") && !l.contains("TORN"))
        .collect();
    assert!(
        !measured.is_empty(),
        "EVERY VB-P4 pass came back flagged -- the epilogue filled the whole frame. The \
         CullReset/CullDispatch brackets sit OUTSIDE the froxel arm's gate and outside the mesh-leg \
         block, so they are written on every VB frame whatever the leg; all three flagged means the \
         witness is recording no bits at all, and this gate's clause 2 would be green for a \
         collector that measures nothing.\n---- worker output ----\n{output}"
    );

    // ---- clause 4: the scope note replaced the assert, and says so -----------------------------
    assert!(
        output.contains("VB-P1d bench SCOPE"),
        "the worker completed without printing the `VB-P1d bench SCOPE` note. The note is what \
         replaced the release-live `mesh_leg` assert: a run whose vb_shade number is a frame-end \
         zero must SAY so at boot, not only in a flag on one line 30 frames later.\n\
         ---- worker output ----\n{output}"
    );

    println!(
        "VG R3 P4-1 gate A: VisibilityBuffer x Sdf COMPLETED with the bench armed -- \
         {} measured pass(es), vb_shade FALLBACK at median_ns={median} (bound {FALLBACK_MAX_NS}). \
         Before this rung the same configuration panicked before frame 1.",
        measured.len()
    );
}

/// **GATE B** — the disarm refuses the knob on a path whose recorder never runs, loudly.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); the orchestrator runs it with --test-threads=1"]
fn deferred_boot_with_the_bench_knob_exits_instead_of_hanging() {
    // The cap is belt-and-braces for the HALF-regressed case: if only the panic is removed
    // (the plan's control vi) the disarm still holds, the run has no bench to complete, and
    // without a cap it would render until the operator killed it. With the cap it exits cleanly
    // and clause 1 below reds on "the worker SUCCEEDED", which is the honest report.
    let (output, success) = spawn_worker(WORKER_B, &[("BOYKO_WINDOW_FRAMES", "8")]);
    if instrument_dead(&output, "gate B") {
        return;
    }

    // ---- clause 1: it FAILED. Success here means the refusal is gone -------------------------
    assert!(
        !success,
        "the Deferred x Both worker exited SUCCESSFULLY with BOYKO_VB_BENCH set. The bench knob \
         must be refused at boot on a non-VisibilityBuffer path: `record_vb` is the only writer of \
         these timestamps AND the only site that resets the pool, and the frame driver calls it \
         only under `scene.path_is_vb()`. A silently accepted knob here is the O2 failure the tree \
         refuses -- and before the disarm existed it was worse than silent, it was an infinite \
         wait in vkGetQueryPoolResults.\n---- worker output ----\n{output}"
    );

    // ---- clause 2: the DISARM ran, and named the path it saw ----------------------------------
    assert!(
        output.contains(DISARM_NOTE),
        "the worker failed, but never printed the disarm notice ({DISARM_NOTE:?}). This clause and \
         clause 3 are two DISTINCT substrings on purpose: without this one, any unrelated crash \
         (device-lost, a plugin panic, a missing asset) would green a gate whose whole subject is \
         one specific refusal.\n---- worker output ----\n{output}"
    );

    // ---- clause 3: and the PANIC named the invariant -------------------------------------------
    assert!(
        output.contains(DISARM_PANIC),
        "the worker failed and printed the disarm notice, but not the panic's invariant text \
         ({DISARM_PANIC:?}). The two halves are ordered disarm-then-panic and are NOT \
         interchangeable: the DISARM is what closes the hang class (delete the panic and this \
         configuration still terminates), while the panic is what keeps a declined bench from \
         being silent. Losing the panic alone leaves an operator waiting for numbers that will \
         never print.\n---- worker output ----\n{output}"
    );

    println!(
        "VG R3 P4-1 gate B: Deferred x Both + BOYKO_VB_BENCH exited (non-zero) with BOTH the \
         disarm notice and the panic invariant. Before this rung the same configuration hung \
         forever on a query pool that was never reset."
    );
}
