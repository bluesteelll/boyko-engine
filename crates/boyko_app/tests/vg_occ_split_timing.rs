//! **VG R3 piece 3 step P3-8 — the FORCE_KEEP / ARMED / DISARMED timing triple, with an
//! INTERLEAVED zero control, in ONE sitting.**
//!
//! Piece 3 makes **no performance claim and pins no benchmark**. What it owes, and what this file
//! produces, is the *measurement piece 4 starts from* — published as prose in the commit message,
//! never as a threshold.
//!
//! # ⚠️ READ THIS BEFORE READING ANY NUMBER THIS FILE PRINTS
//!
//! `docs/VG-DECIDABILITY-FLOOR.md` records four runs of ONE protocol on this box producing floors of
//! **6.3 / 14.3 / 4.7 / 13.5 %**. The floor is **not a constant**: it drifts on a timescale shorter
//! than the gap between two runs. Its operational conclusion is a rule, not a threshold —
//!
//! > a claimed GPU-timing delta below ~15 % is not defensible without a NULL CONTROL measured in the
//! > same sitting
//!
//! — so this harness carries its zero control **inside every round**, never as a separate run, and
//! reports every delta beside it. A delta that does not clear its own sitting's control is printed
//! as **NOT RESOLVED**, and that is a result rather than a failure to produce one.
//!
//! # ⚠️ TWO INSTRUMENT FACTS THAT BOUND EVERYTHING BELOW — both verified in this tree
//!
//! 1. **The swapchain is `VK_PRESENT_MODE_FIFO_KHR`, unconditionally**
//!    (`crates/boyko_rhi_vulkan/src/present/swapchain.rs`, the `present_mode` field of
//!    `VkSwapchainCreateInfoKHR`; the mode is not a choice — the file's own comment calls the query
//!    "belt-and-braces"). So the host loop is throttled to the display refresh, and
//!    **[`Channel::WallClock`] cannot resolve any GPU cost that fits inside the frame budget.** The
//!    harness MEASURES the per-frame period and prints the implied refresh rate, so this is a
//!    reading rather than an assumption.
//! 2. **No timestamp bracket exists over the occlusion cull.** `VbTimedPass` has exactly three
//!    members — `CullReset`, `CullDispatch` (the FROXEL light cull's two halves) and `VbShade` (the
//!    lit-producer dispatch). Nothing brackets `vb_batch_cull`, `vb_cull_late` or the late raster
//!    scope. Adding one is a change to the shipping recorder, which step P3-8's own boundary
//!    ("it adds a scene and tests, and touches no shipped path") excludes. ⇒ **[`Channel::VbShade`]
//!    is a NULL channel by construction**: it measures a pass whose work is identical in all three
//!    regimes, so it bounds the GPU-side instrument rather than the cull's cost.
//!
//! **Consequence, stated plainly rather than worked around: this repository has no instrument that
//! can see the occlusion split's GPU cost.** What it has is (a) an end-to-end channel that is
//! present-limited on this fixture and (b) a GPU-side null. Both are reported. Piece 4's first job,
//! if it wants a number, is a timestamp bracket around the two cull dispatches and the late scope.
//!
//! # The protocol
//!
//! Per ROUND, four legs are run **back to back in one order**, `A0 → B → C → A1`:
//!
//! | leg | configuration |
//! |---|---|
//! | `A0` | DISARMED (`vb_occ_mixed` geometry, no markers) — the baseline |
//! | `B` | FORCE_KEEP (split fully armed, early phase defers nothing) |
//! | `C` | ARMED (the shipping decision) |
//! | `A1` | DISARMED again — **the zero control**, whose difference from `A0` is a true zero |
//!
//! `A0` and `A1` bracket the round, so the zero control spans exactly the drift the `B`/`C` deltas
//! are exposed to. A control run as a separate session would bound a different sitting's noise.
//!
//! [`Channel::WallClock`] is TWO-POINT SUBTRACTED: each leg is run at two frame budgets and the
//! per-frame period is `(t(N₂) − t(N₁)) / (N₂ − N₁)`. That cancels the constant boot cost — device
//! creation, window creation, shader modules, the first-frame pipeline warm-up — which on a windowed
//! boot dwarfs the frames themselves and which no single-budget measurement can separate out.
//!
//! # What this harness ASSERTS
//!
//! Only instrument-level facts: every session produced a number, and the zero control is not exactly
//! zero (a spread of exactly zero across separate processes is evidence the measurement did not
//! happen, not evidence of a perfect instrument). **It asserts no performance property**, because a
//! perf assertion whose threshold nobody can defend is the failure this campaign already paid for.
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko-app --test vg_occ_split_timing -- --ignored --nocapture --test-threads=1
//! ```
//! with `BOYKO_DISABLE_VALIDATION=1`. Optional: `BOYKO_VG_OCC_TIMING_ROUNDS` (default 5),
//! `BOYKO_VG_OCC_TIMING_FRAMES` (the long budget N₂, default 300).

#![cfg(windows)]

use std::process::Command;
use std::time::Instant;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};

mod vb_occ_mixed_scene;

/// The worker every leg re-executes.
const WORKER: &str = "vg_occ_split_timing_worker";

/// Rounds of the whole four-leg sequence. Five rather than three: three samples estimate a spread
/// very poorly, and the spread is the only thing this harness can honestly report.
const DEFAULT_ROUNDS: usize = 5;

/// The LONG frame budget, `N₂`.
const DEFAULT_LONG_FRAMES: u32 = 300;

/// The SHORT frame budget, `N₁` — the two-point subtraction's other end. Large enough to be past
/// every first-frame effect (pipeline creation, descriptor writes, swapchain image acquisition) and
/// small enough that `N₂ − N₁` is most of the measurement.
const SHORT_FRAMES: u32 = 60;

/// The env knob the worker reads to know which regime to boot in.
const ENV_LEG: &str = "BOYKO_VG_OCC_TIMING_LEG";

// ===============================================================================================
// The legs
// ===============================================================================================

/// One configuration of the four-leg round.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Leg {
    /// The mixed geometry with NO markers: `path_vb_occlusion_split()` is false, one raster scope.
    Disarmed,
    /// The split fully armed with `VB_CULL_OCC_FORCE_KEEP` — every mechanism runs, the decision
    /// defers nothing. The one-variable baseline of the decision.
    ForceKeep,
    /// The shipping configuration: armed and unforced.
    Armed,
}

impl Leg {
    fn name(self) -> &'static str {
        match self {
            Leg::Disarmed => "disarmed",
            Leg::ForceKeep => "force_keep",
            Leg::Armed => "armed",
        }
    }

    /// `true` iff this leg puts `OcclusionCulling` in the spawn flush.
    fn marked(self) -> bool {
        !matches!(self, Leg::Disarmed)
    }

    /// The `BOYKO_VG_OCC_FORCE` value, if any.
    fn force(self) -> Option<&'static str> {
        match self {
            Leg::ForceKeep => Some("keep"),
            _ => None,
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "disarmed" => Leg::Disarmed,
            "force_keep" => Leg::ForceKeep,
            "armed" => Leg::Armed,
            other => panic!("`{other}` is not a timing leg"),
        }
    }
}

/// Which instrument a sample came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Channel {
    /// Host wall clock across the frame loop, two-point subtracted. **Present-limited**: see the
    /// module header's instrument fact 1.
    WallClock,
    /// The `VbTimedPass::VbShade` GPU timestamp — the lit-producer dispatch. **A NULL channel by
    /// construction**: identical work in all three legs, so it bounds the instrument and not the
    /// cull. See instrument fact 2.
    VbShade,
}

impl Channel {
    fn unit(self) -> &'static str {
        match self {
            Channel::WallClock => "us/frame",
            Channel::VbShade => "ns",
        }
    }
}

// ===============================================================================================
// The worker
// ===============================================================================================

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let leg = Leg::parse(&std::env::var(ENV_LEG).expect("the timing worker is told its leg"));
    vb_occ_mixed_scene::spawn_mixed(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut geo_table,
        &dev,
        leg.marked(),
    );
}

/// **THE TIMING WORKER** — one boot of the `vb_occ_mixed` scene in the leg the driver names, running
/// until `BOYKO_WINDOW_FRAMES` caps the loop (or, on the GPU-channel pass, until the shipped
/// `BOYKO_VB_BENCH` collector prints its own summary and returns).
///
/// No capture is armed: this run measures the frame loop, and an armed capture would both change it
/// and hold it open.
#[test]
#[ignore = "needs a real windowed GPU device; the timing driver spawns it once per (leg, budget)"]
fn vg_occ_split_timing_worker() {
    if std::env::var(ENV_LEG).is_err() {
        eprintln!(
            "{WORKER}: {ENV_LEG} is unset -- SKIPPED. This worker exists to be spawned by \
             `vg_occ_split_timing_triple`; booted without a leg it would render forever."
        );
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(
        "boyko_engine vg occ timing",
        vb_occ_mixed_scene::EXTENT,
        vb_occ_mixed_scene::EXTENT,
    ));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    // The pyramid is armed on EVERY leg, including the disarmed one. Its cost is then a constant of
    // the comparison rather than a variable of it — the same reason `vb_occ_split_gate.rs` arms it
    // on its unmarked control, and the same reason `[vb_occ_mixed_off]` carries `BOYKO_VG_HZB`.
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    app.run();
}

// ===============================================================================================
// The statistics
// ===============================================================================================

/// One configuration's samples on one channel.
#[derive(Debug, Clone)]
struct Samples {
    label: String,
    values: Vec<f64>,
}

impl Samples {
    fn median(&self) -> f64 {
        let mut v = self.values.clone();
        v.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        let n = v.len();
        if n.is_multiple_of(2) { (v[n / 2 - 1] + v[n / 2]) * 0.5 } else { v[n / 2] }
    }

    /// `σ / mean`, sample standard deviation. Stable in `n`, unlike peak-to-peak — the correction
    /// `vg_decidability_floor.rs` measured against its own first design.
    fn cv(&self) -> f64 {
        let n = self.values.len() as f64;
        let mean = self.values.iter().sum::<f64>() / n;
        if mean <= 0.0 || self.values.len() < 2 {
            return f64::INFINITY;
        }
        let var = self.values.iter().map(|s| (s - mean) * (s - mean)).sum::<f64>() / (n - 1.0);
        var.sqrt() / mean
    }
}

/// The relative difference of two medians, signed: positive means `b` is SLOWER than `a`.
fn relative(a: &Samples, b: &Samples) -> f64 {
    let (ma, mb) = (a.median(), b.median());
    if ma <= 0.0 { f64::INFINITY } else { (mb - ma) / ma }
}

// ===============================================================================================
// Driving the worker
// ===============================================================================================

/// Runs ONE worker process and returns its wall clock, in microseconds.
fn wall_clock_us(leg: Leg, frames: u32) -> f64 {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1"])
        .env(ENV_LEG, leg.name())
        .env("BOYKO_WINDOW_FRAMES", frames.to_string())
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // Every capture removed: an armed capture changes the frame loop it is being used to time,
        // and holds it open past the frame cap.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_VG_OCC_FORCE");
    if let Some(f) = leg.force() {
        cmd.env("BOYKO_VG_OCC_FORCE", f);
    }
    let t0 = Instant::now();
    let status = cmd.status().expect("invariant: the timing worker spawns");
    let dt = t0.elapsed();
    assert!(status.success(), "the timing worker (`{}`, {frames} frames) exited {status}", leg.name());
    dt.as_secs_f64() * 1.0e6
}

/// Runs ONE worker process with the shipped `BOYKO_VB_BENCH` collector armed and returns its
/// reported lit-producer GPU time in nanoseconds.
///
/// The bench prints ONE `VB-P1d …` line and returns on its own, so no frame cap is set. ⚠️ The
/// marker is matched ANYWHERE in the line, not at its start: under `--nocapture` libtest writes its
/// progress without a trailing newline, so the bench's `println!` lands on the SAME line — the
/// measured correction `vg_decidability_floor.rs` records.
fn vb_shade_ns(leg: Leg) -> Option<f64> {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([WORKER, "--ignored", "--exact", "--nocapture", "--test-threads=1"])
        .env(ENV_LEG, leg.name())
        .env("BOYKO_VB_BENCH", "1")
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env_remove("BOYKO_WINDOW_FRAMES")
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_VG_OCC_FORCE");
    if let Some(f) = leg.force() {
        cmd.env("BOYKO_VG_OCC_FORCE", f);
    }
    let out = cmd.output().expect("invariant: the timing worker spawns");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().find(|l| l.contains("VB-P1d "))?;
    // Either leg of the shipped bench reports the lit producer under its own key.
    for key in ["flat_shade_ns=", "froxel_shade_ns="] {
        if let Some(at) = line.find(key) {
            let v = line[at + key.len()..]
                .split_whitespace()
                .next()?
                .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.');
            return v.parse().ok();
        }
    }
    None
}

// ===============================================================================================
// The measurement
// ===============================================================================================

/// **The three numbers, and the zero control that says whether any of them is a number at all.**
///
/// See the module header for the protocol, for the two instrument facts that bound it, and for why
/// this test asserts nothing about performance.
#[test]
#[ignore = "live GPU measurement (spawns many windowed workers); the orchestrator runs it with --test-threads=1"]
fn vg_occ_split_timing_triple() {
    let rounds: usize = std::env::var("BOYKO_VG_OCC_TIMING_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ROUNDS);
    let long_frames: u32 = std::env::var("BOYKO_VG_OCC_TIMING_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_LONG_FRAMES);
    assert!(
        long_frames > SHORT_FRAMES,
        "the two-point subtraction needs N2 ({long_frames}) > N1 ({SHORT_FRAMES})"
    );
    let span = f64::from(long_frames - SHORT_FRAMES);

    let mut a0 = Samples { label: "disarmed (A0)".into(), values: Vec::with_capacity(rounds) };
    let mut a1 = Samples { label: "disarmed (A1, zero control)".into(), values: Vec::with_capacity(rounds) };
    let mut keep = Samples { label: "force_keep".into(), values: Vec::with_capacity(rounds) };
    let mut armed = Samples { label: "armed".into(), values: Vec::with_capacity(rounds) };

    // ---- channel W: end-to-end wall clock, two-point subtracted, A0 -> B -> C -> A1 -------------
    //
    // The ORDER is fixed and the zero control BRACKETS the round: A0 and A1 are the same
    // configuration measured at the two ends of the span the B/C deltas are exposed to, so whatever
    // drift happened during the round appears in the control as well. Interleaving is the whole
    // design — a control run as its own session bounds a different sitting.
    for r in 0..rounds {
        let per_frame = |leg: Leg| {
            (wall_clock_us(leg, long_frames) - wall_clock_us(leg, SHORT_FRAMES)) / span
        };
        a0.values.push(per_frame(Leg::Disarmed));
        keep.values.push(per_frame(Leg::ForceKeep));
        armed.values.push(per_frame(Leg::Armed));
        a1.values.push(per_frame(Leg::Disarmed));
        eprintln!(
            "VG-P3-8 timing: round {} of {rounds} -- A0={:.1} keep={:.1} armed={:.1} A1={:.1} us/frame",
            r + 1,
            a0.values[r],
            keep.values[r],
            armed.values[r],
            a1.values[r]
        );
    }

    // Refuse to REPORT before the baseline is a number: a `relative()` over a non-positive median is
    // infinite, and an infinite delta prints as "RESOLVED" — a number that looks like a win, from a
    // measurement that did not happen.
    assert!(
        a0.median().is_finite() && a0.median() > 0.0,
        "the baseline per-frame period is {:.3} us, which is not a measurement",
        a0.median()
    );

    // ---- channel G: the GPU-side NULL, same sitting, same interleave ----------------------------
    let mut g: Vec<(Leg, Samples)> = [Leg::Disarmed, Leg::ForceKeep, Leg::Armed]
        .into_iter()
        .map(|l| (l, Samples { label: format!("vb_shade {}", l.name()), values: Vec::new() }))
        .collect();
    for _ in 0..rounds {
        for (leg, s) in &mut g {
            if let Some(ns) = vb_shade_ns(*leg) {
                s.values.push(ns);
            }
        }
    }

    // ---- the report -----------------------------------------------------------------------------
    let control = relative(&a0, &a1).abs();
    let cv_band = 3.0 * a0.cv();
    // THE BAND a delta must clear to be reported as resolved: the larger of this sitting's own
    // zero-control delta and three sigma on a single reading of the baseline. Both are properties of
    // THIS sitting; neither is a constant carried in from a previous run, because the floor is
    // measured NOT to be one.
    let band = control.max(cv_band);

    println!("=== VG R3 P3-8 — the timing triple, one sitting, {rounds} rounds ===");
    println!(
        "CHANNEL W ({}) — end-to-end wall clock, two-point subtracted over N2={long_frames} minus \
         N1={SHORT_FRAMES} frames.",
        Channel::WallClock.unit()
    );
    for s in [&a0, &keep, &armed, &a1] {
        println!(
            "  {:<28} median={:>10.1}  CV={:>6.2}%  n={}",
            s.label,
            s.median(),
            100.0 * s.cv(),
            s.values.len()
        );
    }
    println!(
        "  ZERO CONTROL |A1-A0| = {:.2}%   3-sigma(A0) = {:.2}%   =>   RESOLUTION BAND = {:.2}%",
        100.0 * control,
        100.0 * cv_band,
        100.0 * band
    );
    for (label, s) in [("force_keep vs disarmed", &keep), ("armed vs disarmed", &armed)] {
        let d = relative(&a0, s);
        let verdict = if d.abs() > band { "RESOLVED" } else { "NOT RESOLVED" };
        println!("  {label:<26} delta = {:+.2}%   [{verdict}]", 100.0 * d);
    }
    let d_decision = relative(&keep, &armed);
    let verdict = if d_decision.abs() > band { "RESOLVED" } else { "NOT RESOLVED" };
    println!(
        "  {:<26} delta = {:+.2}%   [{verdict}]  <- the ONE-BIT contrast: same scopes, same \
         dispatches, same descriptor sets; only the decision differs",
        "armed vs force_keep",
        100.0 * d_decision
    );

    // The present-limit reading, taken rather than assumed.
    let period_ms = a0.median() / 1000.0;
    println!(
        "  ⚠️ PRESENT LIMIT: the measured baseline period is {period_ms:.3} ms/frame ({:.1} Hz). The \
         swapchain is created with VK_PRESENT_MODE_FIFO_KHR unconditionally, so this channel is \
         bounded BELOW by the display refresh period. If the number above is at the refresh period, \
         the frame is present-limited and channel W cannot see ANY GPU cost that fits inside the \
         frame budget -- every delta it reports is then instrument noise BY CONSTRUCTION, and \
         NOT RESOLVED is the only honest verdict this fixture can return.",
        1000.0 / period_ms.max(f64::MIN_POSITIVE)
    );

    println!(
        "CHANNEL G ({}) — VbTimedPass::VbShade, the lit-producer dispatch. ⚠️ A NULL CHANNEL BY \
         CONSTRUCTION: the three legs render byte-identical images, so this pass does identical \
         work in all of them. It bounds the GPU-side instrument; it is NOT the cull's cost. NO \
         timestamp bracket exists over `vb_batch_cull`, `vb_cull_late` or the late raster scope, \
         and adding one is a change to the shipping recorder that step P3-8's boundary excludes.",
        Channel::VbShade.unit()
    );
    for (_, s) in &g {
        if s.values.is_empty() {
            println!("  {:<28} NO SAMPLES (the shipped bench printed no line)", s.label);
        } else {
            println!(
                "  {:<28} median={:>10.1}  CV={:>6.2}%  n={}",
                s.label,
                s.median(),
                100.0 * s.cv(),
                s.values.len()
            );
        }
    }
    if g.iter().all(|(_, s)| s.values.len() >= 2) {
        let base = &g[0].1;
        for (leg, s) in g.iter().skip(1) {
            println!(
                "  vb_shade {:<19} delta vs disarmed = {:+.2}%  (expected ZERO — this is the null)",
                leg.name(),
                100.0 * relative(base, s)
            );
        }
    }
    println!(
        "=== No threshold is pinned and no perf property is asserted. These are the numbers piece 4 \
         starts from. ==="
    );

    // ---- the ONLY assertions: instrument-level -------------------------------------------------
    for s in [&a0, &keep, &armed, &a1] {
        assert_eq!(
            s.values.len(),
            rounds,
            "`{}` produced {} of {rounds} samples. Pooling only the survivors would UNDERSTATE the \
             spread, which is the one quantity this harness exists to report.",
            s.label,
            s.values.len()
        );
    }
    assert!(
        control > 0.0,
        "the zero control is EXACTLY 0.00% across {rounds} pairs of separate processes. That is not \
         a perfect instrument; it is evidence the two legs did not vary because they did not run."
    );
}
