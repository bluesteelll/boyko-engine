//! **VG R3 piece 3 steps P3-5/P3-7 — the B2 PAIRING gate: both captures, ONE process, ONE frame.**
//!
//! `BOYKO_VB_CULL_READBACK` and `BOYKO_HZB_DUMP` armed together in one windowed boot must produce
//! BOTH files. Nothing in the tree had ever run that combination, and it did not work:
//!
//! > `runner.rs`'s cull-readback block `return`ed out of the frame loop on the **first presented
//! > frame**, from **outside** the exit conjunction the other four capture drivers share. Arming the
//! > two together therefore exited at frame 1 with the cull file written and the pyramid file
//! > **never** — a skip that named itself nowhere.
//!
//! Step P3-5 converted the readback to the settle → request → drain shape its siblings use, sharing
//! `hzb_dump`'s own `SETTLE_FRAMES`/`DRAIN_FRAMES`, and folded its exit into the conjunction. This
//! file is that fix, demonstrated — **before** anything depends on it.
//!
//! # Why it must be ONE process and not two runs
//!
//! A windowed boot owns the device singleton and the window, so "at the same time" can only ever
//! mean one sitting. Running the two captures in separate processes would not be a workaround
//! either: the pyramid the cull tests against is the one THIS frame's build wrote, so a cull payload
//! from one process and a pyramid from another describe two different frames of two different runs,
//! and the only way to green a comparison between them would be to relax it.
//!
//! # VG R3 piece 3 step P3-7 — the clause this file was waiting for
//!
//! Step P3-5 could not compare the two frame indices: the probe line carried `frame=`, the dump
//! header carried no such word, and a tripwire test
//! (`the_dump_header_still_carries_no_frame_index`) pinned the header's four-scalar-word shape so
//! that widening it would red HERE with the instruction attached. Step P3-7 widened it
//! (`HZB_DUMP_HEADER_SCALAR_WORDS` 4 → 6, the magic bumped, the RECORDER stamping the index inside
//! the copy frame's own command buffer), the tripwire fired, and its instruction was: write the
//! real clause and DELETE the test rather than update its number. That is what happened.
//!
//! **Clause 4 is now `probe.frame == dump_header.frame_index`** — the third and load-bearing half
//! of the plan's frame-index trap, and the only one that says the two captures describe ONE frame
//! rather than two frames of one run. Both numbers come from the SAME engine clock (the runner's
//! monotonic per-iteration counter): the probe latches it at its request frame, and the recorder
//! stamps it into the header from `GBufferScene::engine_frame_index` while recording that frame's
//! copies. Neither side is a host guess made after the fact.
//!
//! # ⚠️ What this gate still claims nothing about
//!
//! The cull's decisions. This fixture marks nothing, so `path_vb_occlusion_split()` is false, the
//! dump's early-depth region is not live and `gpu_frame=` still reads the boot prefill — the
//! `gpu_frame == frame` control (plan D6's F-M4a) needs a MARKED readback fixture and is step
//! P3-8's. What this file claims is that the two instruments can be read from one frame of one run,
//! and that they agree about which frame that was.
//!
//! # Run
//!
//! `cargo test -p boyko-app --test vb_cull_hzb_pairing -- --ignored --nocapture --test-threads=1`
//! with `BOYKO_DISABLE_VALIDATION=1`. The driver spawns the worker itself; a worker run directly
//! SKIPS (rather than looping forever) without the two knobs.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};
use boyko_rhi_vulkan::present::{
    HZB_DUMP_HEADER_BYTES, HZB_DUMP_MAGIC, HZB_DUMP_WORD_FRAME_INDEX,
};

mod vb_inst_cull_scene;

use vb_inst_cull_scene::{EXTENT, WIDE, parse_probe_line};

/// The worker test the driver re-executes.
const WORKER: &str = "vb_cull_hzb_pairing_worker";

/// The engine frame index the capture must be at least at.
///
/// The plan's own convergence clause: the pyramid's boot clear makes it all-zeros at birth, so a
/// cull reading it on frame 1 provably defers nothing, and D1's fixed-point argument holds from
/// frame 2. `>=`, never `==`: the driver counts PRESENTED frames while `frame_index` increments on
/// every loop iteration including recreate-skips, so the exact value is not predictable.
///
/// The settle window is 30 presented frames, so a healthy run reports far above this. The clause is
/// pinned at the value the PLAN states because that is the one that separates "converged" from the
/// defect — the old block captured the FIRST presented frame, i.e. `frame == 0`.
const MIN_CONVERGED_FRAME: u32 = 3;

// ===============================================================================================
// The worker: one process, one engine boot, TWO capture files
// ===============================================================================================

fn setup(
    commands: Commands,
    meshes: NonSendResMut<Assets<MeshGpu>>,
    materials: ResMut<Assets<Material>>,
    geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    vb_inst_cull_scene::fixture_setup_system(commands, meshes, materials, geo_table, dev, &WIDE);
}

/// **THE WORKER** — one `VisibilityBuffer × Mesh` boot with the pyramid armed and BOTH capture
/// knobs set by the driver.
///
/// `HzbMode::Build` is what makes `GBufferScene::hzb` `Some`, which is what the pyramid dump reads;
/// without it the dump driver stays in `Request` forever and the run would never end. The cull
/// readback needs no arming beyond its own variable — `vb_batch_cull` runs on every VB × Mesh frame.
///
/// The WIDE framing (nothing outside the frustum) is deliberate: this gate is about the two
/// instruments pairing, so a fixture that also rejected geometry would give a red two possible
/// causes.
#[test]
#[ignore = "needs a real windowed GPU device; the pairing driver spawns it with both knobs set"]
fn vb_cull_hzb_pairing_worker() {
    // A worker booted without the knobs arms no capture, so the host loop has nothing to complete
    // and `app.run()` never returns — a hang, the worst failure mode a sweep can have.
    if std::env::var("BOYKO_VB_CULL_READBACK").is_err() || std::env::var("BOYKO_HZB_DUMP").is_err()
    {
        eprintln!(
            "{WORKER}: BOYKO_VB_CULL_READBACK and/or BOYKO_HZB_DUMP unset -- SKIPPED. This worker \
             exists to be spawned by its driver; booted without both knobs it would render \
             forever, since no armed capture could ever complete."
        );
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb cull + hzb pairing", EXTENT.0, EXTENT.1));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    app.run();
}

// ===============================================================================================
// The gate
// ===============================================================================================

/// Little-endian `u32` at word index `i`.
fn word(bytes: &[u8], i: usize) -> u32 {
    let o = i * 4;
    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
}

/// **THE B2 GATE** — one process, both knobs, both files, and (since step P3-7) both stamped with
/// the same engine frame.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); the orchestrator runs it with --test-threads=1"]
fn both_captures_are_produced_by_one_process() {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let cull_out: PathBuf = std::env::temp_dir().join("boyko_vb_cull_hzb_pairing_cull.txt");
    let dump_out: PathBuf = std::env::temp_dir().join("boyko_vb_cull_hzb_pairing_hzb.bin");
    // A stale file from a previous run that this run failed to overwrite would be read as this
    // run's evidence — and on THIS gate that is the exact failure it exists to detect, since "the
    // second capture never ran" and "the second capture left last run's file" are the same bytes.
    let _ = std::fs::remove_file(&cull_out);
    let _ = std::fs::remove_file(&dump_out);

    let status = Command::new(&exe)
        .args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env("BOYKO_VB_CULL_READBACK", &cull_out)
        .env("BOYKO_HZB_DUMP", &dump_out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // ⚠️ NEITHER capture variable is removed, and that is the whole gate. `vb_occ_split_gate.rs`
        // removes `BOYKO_HZB_DUMP` and `vb_inst_cull_scene::run_cull_probe_worker` does not — an
        // asymmetry that was invisible while no driver ever armed the pair. The two knobs below are
        // set together ON PURPOSE; the two unrelated capture drivers are removed so a red here has
        // one possible cause.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .status()
        .expect("invariant: the worker process spawns");
    assert!(status.success(), "the pairing worker exited {status}");

    // ---- clause 1: BOTH files exist. This is the B2 defect, and it is what used to fail ---------
    let cull_text = std::fs::read_to_string(&cull_out).unwrap_or_else(|e| {
        panic!(
            "the CULL capture wrote no line at {} ({e}). A worker that renders and produces \
             nothing is an instrument failure, not an empty scene.",
            cull_out.display()
        )
    });
    let dump_bytes = std::fs::read(&dump_out).unwrap_or_else(|e| {
        panic!(
            "the PYRAMID capture wrote no file at {} ({e}), while the cull capture wrote one.\n\
             THIS IS THE B2 DEFECT: until VG R3 piece 3 step P3-5 the cull readback returned out of \
             the frame loop on the first presented frame, from outside the exit conjunction, so the \
             process ended before the pyramid dump's settle window had elapsed. A regression here \
             means the readback has been given its own exit again.",
            dump_out.display()
        )
    });

    // ---- clause 2: the cull line is a probe line, converged ------------------------------------
    let probe = parse_probe_line(cull_text.trim());
    assert!(
        probe.frame >= MIN_CONVERGED_FRAME,
        "the cull capture came from engine frame {}, below the converged floor \
         {MIN_CONVERGED_FRAME}. The probe settles 30 PRESENTED frames before capturing, so a small \
         value here means the capture is the first presented frame again -- the state in which the \
         pyramid is still its boot clear and the cull provably defers nothing, i.e. a payload that \
         proves nothing while looking green -- got {:?}",
        probe.frame,
        probe.raw
    );

    // ---- clause 3: the pyramid file is a pyramid file -------------------------------------------
    assert!(
        dump_bytes.len() > HZB_DUMP_HEADER_BYTES as usize,
        "the pyramid capture is {} bytes, which is header-only or shorter -- the file exists but \
         carries no payload",
        dump_bytes.len()
    );
    let magic = word(&dump_bytes, 0);
    assert_eq!(
        magic, HZB_DUMP_MAGIC,
        "the pyramid capture's leading word is 0x{magic:08x}, not HZB_DUMP_MAGIC \
         (0x{HZB_DUMP_MAGIC:08x}). A stale file from an earlier run, decoded as this run's \
         evidence, is exactly what the removals above exist to prevent."
    );

    // ---- clause 4 (VG R3 piece 3 step P3-7): the two captures describe ONE frame ----------------
    //
    // THE clause this file was built to owe. Clauses 1-3 say both instruments produced a file and
    // each file is well-formed; only this one says they are looking at the same frame. Without it,
    // a cull payload from frame N and a pyramid from frame N+4 would green every clause above while
    // describing two different states of the scene — and every downstream comparison between the
    // cull's verdicts and the pyramid they were tested against would rest on that.
    //
    // Both numbers come from the runner's monotonic per-iteration counter, taken by the work that
    // ran: the probe latches it at its request frame; the recorder stamps it into the header from
    // `GBufferScene::engine_frame_index` with a `vkCmdUpdateBuffer` inside that frame's command
    // buffer. A host-written header would have made this an equality between two host beliefs.
    //
    // The two drivers reach `Request` together because they count the SAME `SETTLE_FRAMES` of
    // presented frames from the same start and are asked on the same loop iteration — that sharing
    // is why `hzb_dump::SETTLE_FRAMES` is a shared constant rather than two literals that agree.
    // This assertion is what MEASURES that, instead of trusting it.
    let dump_frame = word(&dump_bytes, HZB_DUMP_WORD_FRAME_INDEX);
    assert_eq!(
        probe.frame, dump_frame,
        "the two captures came from DIFFERENT engine frames: the cull probe latched frame {}, the \
         pyramid dump's header was stamped with frame {dump_frame}. Both are the runner's own \
         per-iteration counter, taken by the frame that ran, so a difference means the two drivers \
         no longer request on one iteration -- a settle window that drifted, or a driver given its \
         own exit again. Every comparison that treats the cull's verdicts and this pyramid as one \
         frame's evidence is void until they agree. Probe line: {:?}",
        probe.frame,
        probe.raw
    );

    println!(
        "VG R3 P3-7 pairing: ONE process produced BOTH captures FROM ONE FRAME -- cull frame={} \
         gpu_frame={} batches={}, pyramid {} B (levels={}) stamped frame={dump_frame}.",
        probe.frame,
        probe.gpu_frame,
        probe.batches,
        dump_bytes.len(),
        word(&dump_bytes, 3)
    );
}
