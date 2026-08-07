//! **VG R3 piece 4 rung P4-2 — the query-pool commands, adjudicated by the validation layer.**
//!
//! Rung P4-2 put seven new `vkCmdWriteTimestamp` brackets into the shipping VB recorder and grew
//! the query pool 3 → 10 pairs. Every one of those commands is recorded ONLY under
//! `BOYKO_VB_BENCH`, so **the bench-armed run is the only configuration in this repository that
//! executes `vkCmdResetQueryPool` / `vkCmdWriteTimestamp` on the VB path at all**. Nothing else can
//! see them: no golden pin arms the bench (the collector is `None` on every pinned frame, which
//! records zero commands), and the barrier-stream baseline is a REPLICA of the declarator, blind to
//! timestamps by construction.
//!
//! This file is the one gate that runs those commands past a live `VK_LAYER_KHRONOS_validation`.
//!
//! # The three-outcome discipline
//!
//! Two workers boot the SAME `vb_occ_mixed` scene, the SAME `VisibilityBuffer × Mesh` path and the
//! SAME `HzbConfig::Build`, with validation **ON** (this gate removes `BOYKO_DISABLE_VALIDATION`
//! from the child environment). They differ in exactly one variable: one has `BOYKO_VB_BENCH=1`.
//!
//! | outcome | condition | verdict |
//! |---|---|---|
//! | **GREEN** | both completed, the bench worker printed ≥1 `VB-P4 pass=` line, the normalized message sets are EQUAL | pass |
//! | **RED** | both completed and the message sets DIFFER | fail — the only failure this gate claims |
//! | **INSTRUMENT-DEAD** | *neither* completed | printed loudly, **not asserted** |
//! | **INCONCLUSIVE** | exactly ONE completed | printed and **failed** — escalation, not classification |
//!
//! **Why INSTRUMENT-DEAD is not a red.** The standing environment note for this machine is that the
//! validation layer is crash-prone (`BOYKO_DISABLE_VALIDATION=1` is the norm for every GPU leg in
//! this tree). A layer that takes BOTH workers down is a fact about the layer, not a finding about
//! piece 4 — and a boot without the layer present at all fails with `ValidationUnavailable` rather
//! than silently running unvalidated, so "the oracle was absent" also lands here instead of
//! greening vacuously.
//!
//! **Why INCONCLUSIVE fails rather than skipping.** A real bench-only defect — a VUID that aborts
//! the armed worker and only the armed worker — takes exactly this shape. Classifying it as
//! "environment" would be the gate deciding the question it exists to ask.
//!
//! **The non-vacuity clause is not optional.** Without it, two workers that both recorded nothing
//! (a disarmed collector, a worker that never reached a frame) agree trivially and the gate is
//! green for a configuration in which not one timestamp command executed.
//!
//! # What this gate CANNOT claim
//!
//! * **Nothing about BARRIERS.** Synchronization validation is MEASURED DEAD on this machine — a
//!   deliberately removed barrier produced zero `SYNC-HAZARD` messages and a byte-identical golden.
//!   This leg therefore sees STATIC legality only: object lifetimes, VUIDs on command parameters,
//!   render-scope legality of `vkCmdResetQueryPool`, query index ranges. A missing dependency
//!   between a timestamp and the work it brackets would be invisible here.
//! * **Nothing about the NUMBERS.** It never reads a duration or an offset. Whether a bracket spans
//!   the right commands is `vg_occ_split_timing.rs`'s question, from rung P4-6.
//! * **Nothing on a golden frame.** No pin sets `BOYKO_VB_BENCH`, so on every pinned run the
//!   witness is `None` and every site added by P4-1/P4-2 records zero commands. The pins cannot
//!   observe this instrument at all — which is the reason this gate exists.
//! * **It cannot prove the layer would have spoken.** Two EMPTY message sets compare equal. The
//!   observed message counts are PRINTED by the passing gate precisely so a reader can tell "the
//!   layer said the same things" from "the layer said nothing at all"; the gate does not assert a
//!   nonzero count, because a clean run legitimately has one.
//!
//! # The controls this gate is the red for (plan P4-2)
//!
//! * **(i)** move `reset_frame` inside a rendering scope → `VUID-vkCmdResetQueryPool-renderpass` in
//!   the armed worker, absent from the control, and the set comparison reds.
//! * **(iii)** size the pool at `2 * 3` while `VB_PASS_COUNT == 10` → with `debug_assert!`s off, an
//!   out-of-range reset / write, again armed-only.
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko-app --test vb_bench_query_validation -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! ⚠️ Unlike every other GPU gate here, the DRIVER may run with `BOYKO_DISABLE_VALIDATION=1` in the
//! shell — it removes the variable from both children on purpose. Both workers SKIP unless their
//! driver spawned them (see [`DRIVER_MARKER`]): booted bare, the control worker has nothing to end
//! its frame loop.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};

mod occ_fixture;
mod vb_occ_mixed_scene;

use vb_occ_mixed_scene::EXTENT;

/// The `BOYKO_VB_BENCH=1` worker — the only configuration in the tree that records
/// `vkCmdResetQueryPool` / `vkCmdWriteTimestamp` on the VB path.
const WORKER_BENCH: &str = "vb_bench_query_validation_bench_worker";

/// The twin without the knob. One variable apart, same scene, same path, same pyramid config.
const WORKER_CONTROL: &str = "vb_bench_query_validation_control_worker";

/// TIMED frames the armed worker collects past `VB_BENCH_WARMUP` (20), i.e. 28 presented frames
/// before it prints and returns. The plan's own number; small because this gate reads LABELS and
/// message sets, never a measurement.
const BENCH_FRAMES: &str = "8";

/// The control worker's `BOYKO_WINDOW_FRAMES` budget.
///
/// Deliberately ABOVE the armed worker's 28 presented frames: the sets are compared as sets, so a
/// repeated message costs nothing, but a control that stopped EARLIER could miss a message the
/// armed run emitted late and red for a reason that has nothing to do with the bench.
const CONTROL_FRAMES: &str = "40";

/// The armed worker's own cap — belt-and-braces, far above the 28 frames its bench budget needs, so
/// a worker that renders but never completes its bench EXITS and reds on the missing `VB-P4` line
/// instead of spinning.
const BENCH_FRAME_CAP: &str = "400";

/// The prefix `boyko_rhi_vulkan::debug`'s messenger callback puts on every WARNING/ERROR message it
/// receives. The gate's entire input.
const VALIDATION_PREFIX: &str = "[vk-validation] ";

/// The boot notice the `O2` decline path prints when the DEVICE cannot serve timestamps at all —
/// then `BOYKO_VB_BENCH` arms no collector and the armed worker is not armed. INSTRUMENT-DEAD.
const NO_TIMESTAMPS: &str = "device timestamps are unusable";

/// The driver's private marker: how a worker tells "my driver spawned me" from "an `--ignored`
/// sweep reached me". Keying on `BOYKO_VB_BENCH` alone would not do — the operator running these
/// gates has that variable in their shell, and the CONTROL worker must skip even then.
const DRIVER_MARKER: &str = "BOYKO_VB_QUERY_VALIDATION_DRIVEN";

// ===============================================================================================
// The workers
// ===============================================================================================

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    vb_occ_mixed_scene::spawn_mixed(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut geo_table,
        &dev,
        true,
    );
}

/// The configuration both workers share, spelled ONCE.
///
/// `HzbConfig::Build` is load-bearing here in a way it is not in the totality gate: it is what makes
/// `GBufferScene::hzb` `Some`, hence one conjunct of `path_vb_occlusion_split()` on this marked
/// scene — and the split is what gives slots 3, 6, 7 and 8 real recorded work to sit around. A
/// worker without it would still write all ten pairs (the brackets sit outside their units' gates)
/// but four of them would enclose nothing, and the layer would be adjudicating a command stream
/// this rung does not care about.
///
/// ⚠️ VG R3 piece 4 rung P4-4 made the OWNER's `OcclusionConfig` the split's FIRST conjunct, so
/// `HzbConfig::Build` and the marker are no longer sufficient. The arming goes through
/// `occ_fixture` — THE single insert site — for two reasons: the paragraph above stops being true
/// the moment this worker silently unsplits, and the vacuity control's one edit must red every
/// gate whose premise is an armed split, this one included.
fn boot(title: &'static str) -> App {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(title, EXTENT, EXTENT));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    occ_fixture::arm_occlusion_with(
        &mut app,
        boyko_render::OcclusionMode::TwoPhase,
        boyko_app::OcclusionForce::None,
    );
    app
}

/// Both workers refuse to run unless their driver spawned them AND the knob that ends their frame
/// loop is present — booted bare they would render until killed, which is the worst failure mode a
/// `-- --ignored` sweep can have.
fn skip_unless_driven(worker: &str, terminating_knob: &str) -> bool {
    if std::env::var(DRIVER_MARKER).is_ok() && std::env::var(terminating_knob).is_ok() {
        return false;
    }
    eprintln!(
        "{worker}: {DRIVER_MARKER} and/or {terminating_knob} unset -- SKIPPED. This worker exists \
         to be spawned by its driver; booted without them it would render forever. To run it by \
         hand, set BOTH variables."
    );
    true
}

/// **THE ARMED WORKER** — `BOYKO_VB_BENCH=1`, so `record_vb` resets the ten-pair pool at the frame
/// top and writes all twenty queries, and the runner reads them back after every presented frame.
#[test]
#[ignore = "needs a real windowed GPU device with the validation layer; the driver spawns it"]
fn vb_bench_query_validation_bench_worker() {
    if skip_unless_driven(WORKER_BENCH, "BOYKO_VB_BENCH") {
        return;
    }
    let mut app = boot("boyko_engine vb query validation (bench armed)");
    app.run();
}

/// **THE CONTROL WORKER** — the same frame, one variable away. Records not one query command.
#[test]
#[ignore = "needs a real windowed GPU device with the validation layer; the driver spawns it"]
fn vb_bench_query_validation_control_worker() {
    if skip_unless_driven(WORKER_CONTROL, "BOYKO_WINDOW_FRAMES") {
        return;
    }
    let mut app = boot("boyko_engine vb query validation (control)");
    app.run();
}

// ===============================================================================================
// The driver
// ===============================================================================================

/// Spawns `worker` in this same test binary, returning `(stdout ++ stderr, exited_successfully)`.
///
/// The streams are MERGED because the evidence is split across them by construction: the `VB-P4`
/// lines are `println!` while every validation message is `eprintln!` from the debug-utils
/// callback. A gate reading one stream would silently stop seeing half of what it asserts on.
///
/// **`BOYKO_DISABLE_VALIDATION` is REMOVED**, and that removal is this gate's whole subject. Every
/// other GPU gate in this tree sets it (the layer is crash-prone here); this one must not, because
/// the layer IS the oracle. The other removals keep the two workers one variable apart: a capture
/// knob or the sibling bench would change the recorded stream, or refuse the boot outright — since
/// rung P4-2 `BOYKO_VB_CULL_READBACK` and `BOYKO_VB_BENCH` are mutually exclusive and the armed
/// worker would panic at boot with an inherited one.
fn spawn_worker(worker: &str, extra: &[(&str, &str)]) -> (String, bool) {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVER_MARKER, "1")
        // THE POINT OF THIS FILE.
        .env_remove("BOYKO_DISABLE_VALIDATION")
        // Every capture driver has its own exit rule and its own recorded commands.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VG_CENSUS")
        // The OTHER bench: the runner refuses the two together with a panic of its own.
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_SV0_BENCH_NULL")
        // Scene / regime selectors: both workers must render ONE scene in ONE regime.
        .env_remove("BOYKO_VG_SCENE")
        .env_remove("BOYKO_VG_OCC")
        .env_remove("BOYKO_VG_OCC_FORCE")
        .env_remove("BOYKO_VG_HZB")
        // Bench-shape knobs from an operator's shell would change the light rig and the scene.
        .env_remove("BOYKO_VB_BENCH_LIGHTS")
        .env_remove("BOYKO_VB_BENCH_GRID")
        .env_remove("BOYKO_VB_BENCH_RIG")
        .env_remove("BOYKO_VB_FROXEL_FORCE_OFF")
        // The knob under test, set per worker by the caller.
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_VB_BENCH_FRAMES");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("invariant: the query-validation worker process spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    (merged, out.status.success())
}

/// The `VUID-...` token in `msg`, if it carries one.
///
/// A VUID is the message's IDENTITY — the thing that appears when a new static-legality violation
/// appears and disappears when it is fixed — so keying on it makes the comparison insensitive to
/// the handle values, object names and spec-URL versions the rest of the text carries.
fn vuid_of(msg: &str) -> Option<&str> {
    let at = msg.find("VUID-")?;
    let tail = &msg[at..];
    let end = tail
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(tail.len());
    Some(&tail[..end])
}

/// Every numeric literal replaced by `#` — handles, indices, sizes, spec-URL versions.
///
/// Hex literals are folded whole (`0x` + hex digits → `0x#`) rather than digit-by-digit, because
/// their letters are not digits and a per-character rule would leave `0xa1b2` and `0xc3d4` looking
/// different while `0x1234` and `0x5678` collapsed.
fn scrub_numerals(msg: &str) -> String {
    let chars: Vec<char> = msg.chars().collect();
    let mut out = String::with_capacity(msg.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '0' && i + 1 < chars.len() && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
            let mut j = i + 2;
            while j < chars.len() && chars[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j > i + 2 {
                out.push_str("0x#");
                i = j;
                continue;
            }
        }
        if chars[i].is_ascii_digit() {
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push('#');
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many characters of a scrubbed, VUID-less message survive into its key.
///
/// Validation messages carry a multi-paragraph "The Vulkan spec states:" tail whose wording tracks
/// the SDK version. Truncating keeps the key stable across an SDK bump while leaving the
/// distinguishing head — the layer's own sentence — intact.
const KEY_CHARS: usize = 160;

/// One message's comparison key.
fn message_key(msg: &str) -> String {
    match vuid_of(msg) {
        Some(vuid) => format!("VUID {vuid}"),
        None => scrub_numerals(msg).chars().take(KEY_CHARS).collect(),
    }
}

/// `(the key set, the raw message count)` for one worker's merged output.
///
/// The COUNT travels beside the set because they answer different questions: the set is what the
/// gate asserts on, the count is what tells a reader whether the layer produced anything at all —
/// the difference between "the two streams agree" and "the oracle was silent in both".
fn validation_messages(output: &str) -> (BTreeSet<String>, usize) {
    let mut keys = BTreeSet::new();
    let mut count = 0usize;
    for line in output.lines() {
        let Some(at) = line.find(VALIDATION_PREFIX) else {
            continue;
        };
        let msg = &line[at + VALIDATION_PREFIX.len()..];
        count += 1;
        keys.insert(message_key(msg));
    }
    (keys, count)
}

/// **THE GATE.**
#[test]
#[ignore = "live GPU gate with the validation layer ON (spawns two windowed workers); run with --test-threads=1"]
fn the_bench_armed_query_commands_add_no_validation_message() {
    let (bench_out, bench_ok) = spawn_worker(
        WORKER_BENCH,
        &[
            ("BOYKO_VB_BENCH", "1"),
            ("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES),
            ("BOYKO_WINDOW_FRAMES", BENCH_FRAME_CAP),
        ],
    );
    let (control_out, control_ok) =
        spawn_worker(WORKER_CONTROL, &[("BOYKO_WINDOW_FRAMES", CONTROL_FRAMES)]);

    let (bench_keys, bench_count) = validation_messages(&bench_out);
    let (control_keys, control_count) = validation_messages(&control_out);

    // ---- INSTRUMENT-DEAD: neither worker completed -------------------------------------------
    //
    // Printed loudly and NOT asserted. Two dead workers is a statement about the validation layer
    // (crash-prone on this machine) or about the device (no usable timestamps, so the armed worker
    // is not actually armed), never about this rung.
    if !bench_ok && !control_ok {
        let why = if bench_out.contains(NO_TIMESTAMPS) || control_out.contains(NO_TIMESTAMPS) {
            "the device reports unusable timestamps, so BOYKO_VB_BENCH arms no collector"
        } else {
            "both workers died with validation ON -- the standing note for this machine is that \
             the layer is crash-prone, and a boot on a host WITHOUT the layer fails with \
             ValidationUnavailable rather than running unvalidated"
        };
        eprintln!(
            "vb_bench_query_validation: INSTRUMENT-DEAD -- {why}. This is not a finding about VG \
             R3 piece 4; re-run on a host whose validation layer survives a windowed VB boot.\n\
             ---- bench worker ----\n{bench_out}\n---- control worker ----\n{control_out}"
        );
        return;
    }

    // ---- INCONCLUSIVE: exactly one completed --------------------------------------------------
    //
    // FAILS rather than skipping. A genuine bench-only defect -- a VUID that aborts the armed
    // worker and only the armed worker -- has exactly this shape, so classifying this as
    // environment would be the gate answering the question it exists to ask.
    assert_eq!(
        bench_ok, control_ok,
        "INCONCLUSIVE: exactly one worker completed (bench_ok={bench_ok}, \
         control_ok={control_ok}). This is NOT classified, because a real bench-only defect looks \
         identical to a flaky layer here: an abort reachable only from the ten-pair reset and the \
         twenty timestamp writes would take the armed worker down alone. Read both outputs below \
         and decide; do not re-run until it passes.\n\
         ---- bench worker (BOYKO_VB_BENCH=1) ----\n{bench_out}\n\
         ---- control worker ----\n{control_out}"
    );

    // ---- NON-VACUITY: the armed worker actually ran the instrument ----------------------------
    //
    // Without this clause, two workers that both recorded ZERO query commands agree trivially --
    // and the gate would be green for a run in which the thing under test never executed.
    let pass_lines: Vec<&str> =
        bench_out.lines().filter(|l| l.contains("VB-P4 pass=")).collect();
    assert!(
        !pass_lines.is_empty(),
        "the bench-armed worker completed but printed NO `VB-P4 pass=` line, so nothing proves it \
         reset a pool or wrote a timestamp -- and two workers that both recorded nothing agree \
         trivially. Either the bench never reached its {BENCH_FRAMES}-frame budget (the \
         BOYKO_WINDOW_FRAMES={BENCH_FRAME_CAP} cap fired first), or the collector was disarmed, or \
         the per-pass summary is gone.\n---- bench worker ----\n{bench_out}"
    );

    // ---- RED: the message sets differ ---------------------------------------------------------
    let only_bench: Vec<&String> = bench_keys.difference(&control_keys).collect();
    let only_control: Vec<&String> = control_keys.difference(&bench_keys).collect();
    assert!(
        only_bench.is_empty() && only_control.is_empty(),
        "RED: the bench-armed run's validation message set differs from the control's.\n\
         ONLY in the BENCH-ARMED run ({} key(s)) -- these are the messages the query-pool reset \
         and the twenty timestamp writes introduced:\n  {}\n\
         ONLY in the CONTROL run ({} key(s)) -- these are messages the armed run stopped emitting, \
         which is just as much a change to the command stream:\n  {}\n\
         ---- bench worker ----\n{bench_out}\n---- control worker ----\n{control_out}",
        only_bench.len(),
        only_bench.iter().map(|k| k.as_str()).collect::<Vec<_>>().join("\n  "),
        only_control.len(),
        only_control.iter().map(|k| k.as_str()).collect::<Vec<_>>().join("\n  "),
    );

    // ---- GREEN ---------------------------------------------------------------------------------
    //
    // The counts are printed, never asserted: a clean run legitimately emits zero messages, so a
    // nonzero requirement would be a false red -- but a reader must be able to tell "the layer said
    // the same things" from "the layer said nothing at all", and only the numbers can say which.
    println!(
        "VG R3 P4-2 query-validation gate: GREEN. Both workers completed; the bench-armed one \
         printed {} `VB-P4 pass=` line(s) (so the reset and all {} timestamp writes executed), and \
         the two normalized validation message sets are equal at {} key(s). Raw message counts: \
         bench={bench_count}, control={control_count}{}.",
        pass_lines.len(),
        pass_lines.len() * 2,
        bench_keys.len(),
        if bench_count == 0 && control_count == 0 {
            " -- ZERO on both sides, so this run shows the armed stream added nothing, NOT that \
             the layer was capable of speaking"
        } else {
            ""
        }
    );
}
