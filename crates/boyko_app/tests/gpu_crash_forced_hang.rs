//! Render rung GB, gate **G3** — a forced device loss is reported with the injected pass in the
//! in-flight set. **Nobody runs this by default**: a forced GPU hang makes Windows reset the
//! display driver (TDR, ~2 s of black screens), so it is a `workstation` gate run once, on
//! demand, with the owner's yes.
//!
//! # The two-key interlock (critique W3)
//!
//! - The DRIVER is red, never a pass, unless `BOYKO_GPU_HANG_OPT_IN=i-accept-a-display-reset` is
//!   in its environment — a sweep of the workstation leg cannot fire it by accident.
//! - The WORKER returns at once unless the driver's own marker (`BOYKO_GB_G3_DRIVEN`) AND the
//!   opt-in are both set — the census forces it into the `gpu-windowed` class, and a sweep of that
//!   leg must find it inert. (`boot_validation_clean`'s driver scrubs every `BOYKO_*`, so its
//!   sweep cannot arm it either.)
//! - The runner arms the hang only for the `inject_gpu_hang` force key AND the opt-in value, and
//!   `Renderer::arm_gpu_hang` refuses unless the build admits breadcrumbs and they armed.
//!
//! # What it asserts, on the child's stderr (`BOYKO_LOG` removed, so `boyko-E3013` falls back
//! to stderr)
//!
//! - a `boyko-E3013:` report exists;
//! - one slot line lists `test_gpu_hang` in its in-flight set;
//! - when the header's `pieces=` names `fault`, a `device fault:` record line exists.
//!
//! # Its three outcomes, kept apart
//!
//! 1. **green** — the report names the injected pass;
//! 2. **no device loss** — the child is still alive at the deadline (killed) or exits without a
//!    report: the injection primitive did not reach a TDR. At this rung the primitive is a seam
//!    (`Renderer::record_test_gpu_hang` records nothing — GB cut D10 / Q2: the hang shader lands
//!    with the shader-SDK lane), so this is the expected outcome until it is filled;
//! 3. **report wrong** — a report exists but does not name the pass (or lacks the fault record).
//!
//! An independent second oracle for the run that matters is `VK_LAYER_LUNARG_crash_diagnostic`
//! (installed with the SDK), enabled by hand for that one run.
//!
//! The vendor binary is written to the child's own temp directory (`BOYKO_GPU_CRASH_DIR`), never
//! the crate directory.
#![cfg(windows)]

#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use boyko_app::gpu_crash::{GPU_CRASH_DIR_ENV, GPU_HANG_OPT_IN_ENV, GPU_HANG_OPT_IN_VALUE, gpu_hang_opted_in};
use boyko_app::prelude::*;
use boyko_app::{GpuCrashDiag, GpuCrashDiagLevel};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_macros::Resource;
use boyko_render::RenderPath;
use graph_receipt::{ObsArming, build_obs_app};

/// The driver's marker, the worker's first key.
const DRIVEN: &str = "BOYKO_GB_G3_DRIVEN";
/// How long the driver waits for the child to report or exit.
const DEADLINE: Duration = Duration::from_secs(90);
/// The worker's frame budget: far past `INJECT_GPU_HANG_FRAME`, so a run whose hang did not fire
/// exits on its own.
const FRAMES: u32 = 200;

#[derive(Resource)]
struct Budget(u32);

fn exit_after_budget(mut budget: ResMut<Budget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// **THE WORKER** — inert unless the driver spawned it with the opt-in.
#[test]
#[ignore = "gpu-windowed: the GB G3 forced-hang worker, inert unless spawned by forced_hang_report_names_the_injected_pass (BOYKO_GB_G3_DRIVEN and BOYKO_GPU_HANG_OPT_IN set); run with --test-threads=1"]
fn gb_g3_worker() {
    if std::env::var_os(DRIVEN).is_none() || !gpu_hang_opted_in() {
        eprintln!("gb_g3_worker: {DRIVEN} or {GPU_HANG_OPT_IN_ENV} is unset -- SKIPPED (spawned by its driver only).");
        return;
    }
    let arming = ObsArming {
        path: RenderPath::Deferred,
        interp: false,
        particles: false,
        upload: false,
        taa: false,
        ddgi: false,
    };
    let mut app = build_obs_app("boyko_app GB G3 worker", arming);
    app.insert_resource(GpuCrashDiag { level: GpuCrashDiagLevel::Checkpoints });
    app.insert_resource(boyko_app::test_force::parse("inject_gpu_hang").expect("invariant: the key exists"));
    app.insert_resource(Budget(FRAMES));
    app.add_systems(exit_after_budget);
    app.run();
}

/// **THE DRIVER** — run once, with the owner's yes.
#[test]
#[ignore = "workstation: forced GPU crash resets the display driver (~2 s); run once, with the owner's yes: BOYKO_GPU_HANG_OPT_IN=i-accept-a-display-reset cargo test -p boyko-app --test gpu_crash_forced_hang -- --ignored --exact forced_hang_report_names_the_injected_pass --test-threads=1 --nocapture"]
fn forced_hang_report_names_the_injected_pass() {
    assert!(
        gpu_hang_opted_in(),
        "REFUSED, not passed: this gate forces a GPU hang, which resets the display driver. Set \
         {GPU_HANG_OPT_IN_ENV}={GPU_HANG_OPT_IN_VALUE} to run it, with the owner's yes."
    );
    let dir = std::env::temp_dir().join(format!("boyko_gb_g3_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test setup: the child's temp dir");

    let exe = std::env::current_exe().expect("test setup: this test binary's own path");
    let mut child = Command::new(exe)
        .args(["gb_g3_worker", "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVEN, "1")
        .env(GPU_HANG_OPT_IN_ENV, GPU_HANG_OPT_IN_VALUE)
        .env(GPU_CRASH_DIR_ENV, &dir)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env_remove("BOYKO_LOG")
        .env_remove("BOYKO_GPU_CRASH_DIAG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("test setup: spawn the worker");
    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("test setup: poll the worker") {
            Some(s) => break Some(s),
            None if start.elapsed() >= DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let mut stderr = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let mut stdout = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut stdout);
    }
    let _ = std::fs::write(dir.join("worker.stderr.txt"), &stderr);
    let _ = std::fs::write(dir.join("worker.stdout.txt"), &stdout);

    let report: Vec<&str> = stderr.lines().filter_map(|l| l.strip_prefix("boyko-E3013: ")).collect();
    let Some(status) = status else {
        panic!(
            "NO DEVICE LOSS within {DEADLINE:?}: the worker was still running and was killed -- the \
             injection primitive did not reach a TDR (at this rung it is a seam; GB cut Q2). Output: {}",
            dir.display()
        );
    };
    if report.is_empty() {
        panic!(
            "NO DEVICE LOSS: the worker exited ({status:?}) without a boyko-E3013 report -- the \
             injection primitive did not hang the device (at this rung it is a seam; GB cut Q2). \
             Output: {}",
            dir.display()
        );
    }
    for line in &report {
        println!("[G3] {line}");
    }
    let names_pass = report
        .iter()
        .any(|l| l.starts_with("slot ") && l.split_once("in flight:").is_some_and(|(_, rest)| rest.contains("test_gpu_hang")));
    assert!(names_pass, "REPORT WRONG: no slot line lists test_gpu_hang in flight:\n{}", report.join("\n"));
    let header = report[0];
    if header.split_once("pieces=").is_some_and(|(_, p)| p.split(' ').next().is_some_and(|ps| ps.contains("fault"))) {
        assert!(
            report.iter().any(|l| l.starts_with("device fault: \"")),
            "REPORT WRONG: the fault piece was armed but no device-fault record was printed:\n{}",
            report.join("\n")
        );
    }
}
