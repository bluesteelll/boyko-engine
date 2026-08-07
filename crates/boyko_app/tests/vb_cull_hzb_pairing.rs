//! **VG R3 piece 3 step P3-5 — the B2 PAIRING gate: both captures, ONE process.**
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
//! # ⚠️ What this gate CANNOT claim yet, and the tripwire that makes the gap loud
//!
//! **It cannot compare the two frame indices.** The probe line carries `frame=` since this step; the
//! dump header does **not** carry one until step **P3-7** widens it (`HZB_DUMP_HEADER_SCALAR_WORDS`,
//! the magic bump, and the recorder stamping the index inside the copy frame's command buffer). So
//! the equality clause `probe.frame_index == dump_header.frame_index` — the third and load-bearing
//! half of the plan's frame-index trap — has no second side to read.
//!
//! Rather than leave that as a note nobody re-reads,
//! [`the_dump_header_still_carries_no_frame_index`] asserts the header's CURRENT shape. When P3-7
//! widens it, that test goes red and names what must be written here. A missing clause that
//! announces itself is the only kind worth having.
//!
//! It also claims nothing about the cull's decisions: the payload is still the inert partition
//! (`occ_flags == 0`, so `n_defer == 0` everywhere). What it claims is that the two instruments can
//! be read from one frame of one run.
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
use boyko_rhi_vulkan::present::{HZB_DUMP_HEADER_BYTES, HZB_DUMP_HEADER_WORDS, HZB_DUMP_MAGIC, MAX_HZB_LEVELS};

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

/// **THE B2 GATE** — one process, both knobs, both files.
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

    println!(
        "VG R3 P3-5 pairing: ONE process produced BOTH captures -- cull frame={} gpu_frame={} \
         batches={}, pyramid {} B (levels={}). ⚠️ The frame-index EQUALITY clause is not asserted \
         here: the dump header carries no frame index until step P3-7.",
        probe.frame,
        probe.gpu_frame,
        probe.batches,
        dump_bytes.len(),
        word(&dump_bytes, 3)
    );
}

/// **THE TRIPWIRE FOR THE CLAUSE THIS STEP CANNOT WRITE.**
///
/// The pairing gate above owes one more clause — `probe.frame == dump_header.frame_index` — and it
/// cannot be written yet, because the dump header has no such word. Step P3-7 adds it by raising
/// `HZB_DUMP_HEADER_SCALAR_WORDS` from 4 to 6 and bumping the magic.
///
/// This test pins the header's CURRENT shape, so that widening reds HERE with the instruction
/// attached, instead of leaving the missing clause to be noticed by nobody. It is NOT `#[ignore]`d:
/// it needs no device, and the whole point is that it runs in every sweep.
///
/// **When it goes red, the fix is to WRITE the equality clause in
/// [`both_captures_are_produced_by_one_process`] and delete this test** — not to update the number.
#[test]
fn the_dump_header_still_carries_no_frame_index() {
    assert_eq!(
        HZB_DUMP_HEADER_WORDS,
        4 + 2 * MAX_HZB_LEVELS,
        "the `BOYKO_HZB_DUMP` header has grown past its four scalar words \
         `[magic, source_w, source_h, levels]`. If step P3-7 added `frame_index`, the pairing gate \
         in this file must now assert `probe.frame == dump_header.frame_index` -- the third clause \
         of the plan's frame-index trap, which is the ONLY one that says the two captures describe \
         ONE frame. Write that clause and delete this test; do not update this number."
    );
}
