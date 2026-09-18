//! **VG R3 piece 4 rung P4-2 — the query-pool commands, adjudicated by the validation layer.**
//!
//! Rung P4-2 put seven new `vkCmdWriteTimestamp` brackets into the shipping VB recorder and grew
//! the query pool 3 → 10 pairs. Every one of those commands is recorded ONLY under the profiler's
//! own knob — `BOYKO_VB_BENCH` when this file was written, `BOYKO_VB_ZONE` since profiling rung 7
//! deleted that collector — so **the armed run is the only configuration in this repository that
//! executes `vkCmdResetQueryPool` / `vkCmdWriteTimestamp` on the VB path at all**. Nothing else can
//! see them: no golden pin arms the profiler (the recorder is `None` on every pinned frame, which
//! records zero commands), and the barrier-stream baseline is a REPLICA of the declarator, blind to
//! timestamps by construction.
//!
//! This file is the one gate that runs those commands past a live `VK_LAYER_KHRONOS_validation`.
//!
//! # The three-outcome discipline
//!
//! Two workers boot the SAME `vb_occ_mixed` scene, the SAME `VisibilityBuffer × Mesh` path and the
//! SAME `HzbConfig::Build`, with validation **ON** (this gate sets `BOYKO_ENABLE_VALIDATION` AND
//! removes `BOYKO_DISABLE_VALIDATION` in the child environment — the backend needs both, and for
//! the whole of this file's life only the second was done). They differ in exactly one variable:
//! one has `BOYKO_VB_ZONE=1`.
//!
//! | outcome | condition | verdict |
//! |---|---|---|
//! | **GREEN** | both completed, the armed worker's artifact counts ≥1 MEASURED pair, BOTH workers reported a live validation messenger, the normalized message sets are EQUAL | pass |
//! | **RED** | both completed and the message sets DIFFER | fail — the only failure this gate claims |
//! | **INSTRUMENT-DEAD** | *neither* completed | printed loudly, **not asserted** |
//! | **INCONCLUSIVE** | exactly ONE completed | printed and **failed** — escalation, not classification |
//!
//! **Why INSTRUMENT-DEAD is not a red.** The standing environment note for this machine is that the
//! validation layer is crash-prone (`BOYKO_DISABLE_VALIDATION=1` is the norm for every GPU leg in
//! this tree). A layer that takes BOTH workers down is a fact about the layer, not a finding about
//! piece 4. A boot without the layer at all is NOT this row: the worker logs `boyko-E3002`
//! (`ValidationUnavailable`) and exits 0 with no arming line, so the missing-witness red refuses
//! it (measured 2026-09-18 under `VK_LOADER_LAYERS_DISABLE`), never a vacuous green.
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
//! * **Nothing on a golden frame.** No pin arms the profiler, so on every pinned run the recorder
//!   is `None`, zero query commands are recorded, and this gate's subject does not exist there.
//!   ⚠️ This bullet was TRUNCATED mid-sentence by rung 7's own migration edit and read as an
//!   unfinished thought for two commits — a doc defect no gate can see, because a comment compiles.
//!
//! # Profiling rung 7 — the ARMED LEG IS THE ZONE RECORDER, and the witness got stronger
//!
//! This gate used to arm `BOYKO_VB_BENCH` and take *"a `VB-P4 pass=` line exists"* as proof that the
//! instrument ran. Rung 7 deletes both. The armed leg is now `BOYKO_VB_ZONE`, because the gate's
//! subject is *the profiler's query commands are VUID-clean* and after this rung the profiler IS
//! `GpuZoneRecorder` — leaving it pointed at the retired collector would have kept it green about
//! code nobody ships.
//!
//! The liveness witness is the artifact's **label census**, and it is strictly stronger than the
//! line it replaces: a printed line proved a SUMMARY was produced, while `measured > 0` proves pairs
//! were bracketed **and their results came back** — which is exactly the property this gate needs,
//! a pool that was reset and timestamps that executed. The parent stamps its own run token, so a
//! leftover artifact from an earlier run is refused on the header instead of being read as this
//! run's evidence.
//!
//! * **The golden pins cannot stand in for it.** No pin arms the profiler, so on a pinned run the
//!   witness is `None` and every site added by P4-1/P4-2 records zero commands. The pins cannot
//!   observe this instrument at all — which is the reason this gate exists.
//! * **It proves the layer was ARMED; it does not prove the layer would have spoken about THIS
//!   defect class.** Two EMPTY message sets compare equal, so the gate refuses a run in which
//!   nothing adjudicated the streams — but it refuses it on the messenger's EXISTENCE, not on the
//!   messenger having talked. See [`ARM_WITNESS_PREFIX`] for why the difference is the whole point:
//!   the mask is WARNING | ERROR, so a *correct* run is a SILENT one, and an arm keyed on speech
//!   reds precisely when the code is clean. What no arm here can establish is COVERAGE: a layer
//!   audible on some other VUID does not prove it would have reported a query-pool one. The
//!   per-side counts stay printed for that reason.
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
//! shell — it removes that variable from both children and sets `BOYKO_ENABLE_VALIDATION` there on
//! purpose, so the child environment is armed whatever the shell holds. Both workers SKIP unless their
//! driver spawned them (see [`DRIVER_MARKER`]): booted bare, the control worker has nothing to end
//! its frame loop.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};

mod occ_fixture;
mod vb_occ_mixed_scene;

use vb_occ_mixed_scene::EXTENT;

/// The `BOYKO_VB_ZONE=1` worker — the only configuration in the tree that records
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

/// The line each worker prints from a startup system, stating whether the instance it is about to
/// record its frames on carries a live `VK_LAYER_KHRONOS_validation` debug-utils messenger.
///
/// **This witnesses the ARMING, and the distinction from witnessing the SPEAKING is the whole
/// point.** The arm it replaces asserted `bench_count + control_count > 0` — "the layer must have
/// COMPLAINED at least once". The messenger's mask is `WARNING | ERROR` only
/// (`boyko_rhi_vulkan::debug::MESSENGER_SEVERITY`), so a clean run emits NOTHING, and that arm
/// therefore reds exactly when the layer was loaded and the code was correct. Its own remediation
/// text carried the fact that refutes it: a boot WITHOUT the layer fails with
/// `ValidationUnavailable` rather than running unvalidated, so silence means "it loaded and said
/// nothing". It survived the `VkQueryPool` leak fix only because every validated boot still
/// carried start-up messages — 18 reported / 50 uncapped (2026-09-18); 0 after
/// fix/boot-validation-errors — so it would red the day the code became clean.
///
/// `VulkanContext::validation_enabled()` answers the property the set equality below actually
/// needs. It is `debug_state.is_some()`, and that state exists only if BOTH halves happened:
/// `create_instance` enabled the layer — its absence returns `ValidationUnavailable`, never a
/// silent unvalidated boot — and `vkCreateDebugUtilsMessengerEXT` succeeded, which requires
/// `VK_EXT_debug_utils`. So `ARMED` separates "layer loaded, silent" from "layer never loaded",
/// which is the only thing silence is ambiguous about.
const ARM_WITNESS_PREFIX: &str = "VB-QUERY-VALIDATION messenger=";

/// [`ARM_WITNESS_PREFIX`]'s value when a messenger is attached to this process's instance.
const ARM_ARMED: &str = "ARMED";

/// [`ARM_WITNESS_PREFIX`]'s value when the context booted without one — the oracle is dead and the
/// two message sets below would be empty for a reason that has nothing to do with the code.
const ARM_ABSENT: &str = "ABSENT";

/// The line a worker prints whenever the messenger's OWN counter grows.
///
/// The gate parses its message sets out of TEXT, so a drift in the callback's
/// [`VALIDATION_PREFIX`] would empty both sets and leave the equality vacuously true while the
/// layer was in fact talking. This number comes from the other end of that channel —
/// `DebugMessengerState::total()`, incremented inside the callback itself — so the parent can
/// refuse a run in which the counter moved and no text arrived.
///
/// A message the layer emits AFTER the last frame's system ran is never reported, so this number
/// is a LOWER bound on the callback's final count. That direction is the safe one: it can only ask
/// the parent for less evidence than the run actually has, never manufacture a red.
const TRAFFIC_WITNESS_PREFIX: &str = "VB-QUERY-VALIDATION callback-total=";

/// The highest `DebugMessengerState::total()` this WORKER process has already printed.
///
/// Its only job is to make the per-frame witness emit one line per NEW message instead of one line
/// per frame — at the armed worker's 400-frame cap the difference is 400 lines of noise inside
/// every failure message this file prints.
static REPORTED_TOTAL: AtomicU32 = AtomicU32::new(0);

/// The run token the parent stamps into the armed worker's artifact, so a leftover from an
/// earlier run is refused on the header rather than read as this run's witness.
const RUN_TOKEN: &str = "vb-query-validation-1";

/// The boot notice the `O2` decline path prints when the DEVICE cannot serve timestamps at all —
/// then `BOYKO_VB_ZONE` arms no recorder and the armed worker is not armed. INSTRUMENT-DEAD.
const NO_TIMESTAMPS: &str = "device timestamps are unusable";

/// The driver's private marker: how a worker tells "my driver spawned me" from "an `--ignored`
/// sweep reached me". Keying on `BOYKO_VB_ZONE` alone would not do — the operator running these
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

/// Prints the ARMING witness — whether this process's device carries a validation messenger.
///
/// Registered on BOTH workers, not just the armed one: the gate compares two streams for EQUALITY,
/// so a control that booted unvalidated contributes an empty set that compares equal to anything,
/// and the gate would be green about a command stream only one side ever adjudicated.
fn report_messenger_arming(dev: NonSendRes<GpuDevice>) {
    let armed = if dev.get().validation_enabled() { ARM_ARMED } else { ARM_ABSENT };
    println!("{ARM_WITNESS_PREFIX}{armed}");
}

/// Prints the messenger's own message counter whenever it grows — the parent's cross-check against
/// the `[vk-validation]` lines it parses out of the merged streams.
fn report_messenger_traffic(dev: NonSendRes<GpuDevice>) {
    let Some(state) = dev.get().debug_state() else {
        return;
    };
    let total = state.total();
    // Relaxed on both halves: this system takes a NonSend parameter, so the runner calls it on the
    // one thread that owns the device and this cell has a single reader-writer. It suppresses
    // duplicate lines and nothing else — the value the parent reads is the printed text, and
    // `total()` does its own Acquire load against the callback's Release increments.
    if total > REPORTED_TOTAL.load(Ordering::Relaxed) {
        REPORTED_TOTAL.store(total, Ordering::Relaxed);
        println!("{TRAFFIC_WITNESS_PREFIX}{total}");
    }
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
    // The oracle's own liveness, reported from inside the process that boots it — the only place
    // that can tell "the layer loaded and stayed silent" from "the layer never loaded".
    app.add_startup_system(report_messenger_arming);
    app.add_systems(report_messenger_traffic);
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

/// **THE ARMED WORKER** — `BOYKO_VB_ZONE=1`, so `record_vb` resets the ring slot's pool at the frame
/// top and writes all twenty queries, and the runner retires them without blocking.
///
/// ⚠️ The terminating knob is the ZONE one. It read `BOYKO_VB_BENCH` for one commit after the
/// driver had moved, and the worker then SKIPPED silently while the driver waited for an artifact
/// nobody wrote — a green worker and a red gate, which is the shape a skip always takes.
#[test]
#[ignore = "needs a real windowed GPU device with the validation layer; the driver spawns it"]
fn vb_bench_query_validation_bench_worker() {
    if skip_unless_driven(WORKER_BENCH, "BOYKO_VB_ZONE") {
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
/// **The layer is armed by TWO variables, and the removal alone armed NOTHING.** The backend reads
/// a conjunction: `runner.rs`'s `InstanceConfig.enable_validation` is
/// `BOYKO_ENABLE_VALIDATION.is_some()`, and `device.rs`'s `validation_requested` is that AND
/// `BOYKO_DISABLE_VALIDATION.is_none()`. This file removed only the second for the whole of its
/// life, so with the first unset every run booted UNVALIDATED — measured 0 armed validation lines,
/// against 19 with `BOYKO_ENABLE_VALIDATION=1` — and the message-set equality it asserts compared
/// two empty sets. Both conjuncts are therefore set here, and
/// [`the_bench_armed_query_commands_add_no_validation_message`]'s arming arm refuses a run in which
/// no messenger was attached — see [`ARM_WITNESS_PREFIX`] for why that arm cannot be keyed on the
/// layer having spoken. The other removals keep the two workers one variable apart: a capture
/// knob or a sibling bench would change the recorded stream, or refuse the boot outright — since
/// rung P4-2 `BOYKO_VB_CULL_READBACK` and the profiler's knob are mutually exclusive and the armed
/// worker would panic at boot with an inherited one.
fn spawn_worker(worker: &str, extra: &[(&str, &str)]) -> (String, bool) {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(DRIVER_MARKER, "1")
        // THE POINT OF THIS FILE — and it takes BOTH halves of the backend's conjunction. Setting
        // the first without removing the second arms nothing (`device.rs::validation_requested`);
        // removing the second without setting the first arms nothing either
        // (`runner.rs::run_windowed` builds `InstanceConfig` from the presence of the first).
        .env("BOYKO_ENABLE_VALIDATION", "1")
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

/// What [`ARM_WITNESS_PREFIX`] reported in `output`, or `None` if the line is absent ENTIRELY.
///
/// The three-way answer is deliberate: `Some(false)` is "a device booted without the layer" and
/// `None` is "no device-stage system ever ran, or `boot` stopped registering the witness". They get
/// different remediations, and collapsing them would send a reader to the wrong one.
fn messenger_armed(output: &str) -> Option<bool> {
    output.lines().find_map(|line| {
        let at = line.find(ARM_WITNESS_PREFIX)?;
        Some(line[at + ARM_WITNESS_PREFIX.len()..].starts_with(ARM_ARMED))
    })
}

/// The highest counter value [`TRAFFIC_WITNESS_PREFIX`] reported — how many WARNING/ERROR messages
/// the callback itself counted, read from the non-text end of the channel.
fn callback_total(output: &str) -> u32 {
    output
        .lines()
        .filter_map(|line| {
            let at = line.find(TRAFFIC_WITNESS_PREFIX)?;
            line[at + TRAFFIC_WITNESS_PREFIX.len()..].trim().parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// Fails unless `output` carries the ARMING witness saying a messenger was attached.
#[track_caller]
fn assert_messenger_armed(worker: &str, output: &str) {
    match messenger_armed(output) {
        Some(true) => {}
        Some(false) => panic!(
            "DEAD ORACLE in {worker}: the worker completed but its device reports NO validation \
             messenger, so `VK_LAYER_KHRONOS_validation` adjudicated nothing it recorded and the \
             message-set equality below would compare an EMPTY set.\n\
             What to do: the backend reads a CONJUNCTION -- `runner.rs` builds \
             `InstanceConfig.enable_validation` from `BOYKO_ENABLE_VALIDATION` being set, and \
             `device.rs::validation_requested` ANDs that with `BOYKO_DISABLE_VALIDATION` being \
             unset. `spawn_worker` does both halves; check that nothing re-adds the second after \
             the removal there.\n\
             ⚠️ Do NOT weaken this arm to make the gate pass: a green with no oracle is what it \
             exists to refuse. Equally, do NOT restore the arm that asserted the layer SPOKE -- \
             the mask is WARNING | ERROR, so a clean run is silent and that arm reds on correct \
             code.\n---- {worker} ----\n{output}"
        ),
        None => panic!(
            "{worker} printed no `{ARM_WITNESS_PREFIX}` line at all, so this gate cannot tell \
             whether an oracle was attached. Either the worker never reached the stage where \
             `GpuDevice` exists (read its output below -- it completed, so this would be a boot \
             that skipped rendering), or `boot()` no longer registers `report_messenger_arming`. \
             The witness is the gate's only evidence that the validation layer was loaded; \
             without it every assertion below is about an unadjudicated command stream.\n\
             ---- {worker} ----\n{output}"
        ),
    }
}

/// **THE GATE.**
#[test]
#[ignore = "live GPU gate with the validation layer ON (spawns two windowed workers); run with --test-threads=1"]
fn the_bench_armed_query_commands_add_no_validation_message() {
    // Profiling rung 7: the armed leg is the ZONE RECORDER, not the collector rung 7 deletes. The
    // gate's subject is "the profiler's query commands are VUID-clean", and after this rung the
    // profiler IS `GpuZoneRecorder` — pointing the gate at the retired collector would have left it
    // green about code nobody ships.
    let mut artifact = std::env::temp_dir();
    artifact.push("boyko_vb_query_validation.toml");
    let _ = std::fs::remove_file(&artifact);
    let artifact_s = artifact.to_string_lossy().into_owned();
    let (bench_out, bench_ok) = spawn_worker(
        WORKER_BENCH,
        &[
            ("BOYKO_VB_ZONE", "1"),
            ("BOYKO_PROFILE_ARTIFACT", &artifact_s),
            ("BOYKO_PROFILE_RUN_TOKEN", RUN_TOKEN),
            ("BOYKO_PROFILE_WORKLOAD", "query_validation"),
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
            "the device reports unusable timestamps, so BOYKO_VB_ZONE arms no recorder"
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
         ---- armed worker (BOYKO_VB_ZONE=1) ----\n{bench_out}\n\
         ---- control worker ----\n{control_out}"
    );

    // ---- THE ORACLE WAS ARMED: a live messenger sat on BOTH instances -------------------------
    //
    // The set comparison below is an EQUALITY, and two empty sets are equal — so without this arm
    // the gate is green for a run in which the validation layer was never armed. That is not a
    // hypothetical: this file removed `BOYKO_DISABLE_VALIDATION` and left `BOYKO_ENABLE_VALIDATION`
    // unset for its whole life, which arms nothing (the backend reads the conjunction), and the
    // gate passed every time on two empty sets.
    //
    // The arm is keyed on the messenger EXISTING, not on it having spoken. Keying on speech — which
    // is what stood here — makes this gate red exactly when the layer is loaded and the code is
    // clean, because the mask is WARNING | ERROR and a correct run is a SILENT one. See
    // `ARM_WITNESS_PREFIX`.
    assert_messenger_armed(WORKER_BENCH, &bench_out);
    assert_messenger_armed(WORKER_CONTROL, &control_out);

    // ---- THE TEXT CHANNEL IS LIVE: the callback's counter and the parsed lines agree ----------
    //
    // The sets below are parsed out of TEXT. A drift in the callback's `[vk-validation]` prefix
    // would empty both of them while the layer talked, and the equality would hold vacuously —
    // the same defect shape the arming arm above closes, one channel further down. The counter is
    // incremented inside the callback, so the two ends disagreeing is conclusive.
    for (worker, counted, parsed, out) in [
        (WORKER_BENCH, callback_total(&bench_out), bench_count, &bench_out),
        (WORKER_CONTROL, callback_total(&control_out), control_count, &control_out),
    ] {
        assert!(
            counted == 0 || parsed > 0,
            "BROKEN ORACLE CHANNEL in {worker}: the debug-messenger callback counted {counted} \
             WARNING/ERROR message(s), but this driver parsed ZERO `{VALIDATION_PREFIX}` lines out \
             of the worker's merged streams. The layer spoke and the gate did not hear it, so the \
             message-set comparison below is about nothing. Most likely the prefix in \
             `boyko_rhi_vulkan::debug::debug_callback` changed and `VALIDATION_PREFIX` here did \
             not.\n---- {worker} ----\n{out}"
        );
    }

    // ---- NON-VACUITY: the armed worker actually ran the instrument ----------------------------
    //
    // Without this clause, two workers that both recorded ZERO query commands agree trivially --
    // and the gate would be green for a run in which the thing under test never executed.
    // THE WITNESS IS THE ARTIFACT'S LABEL CENSUS, and it is strictly stronger than the printed
    // line it replaces. "A `VB-P4 pass=` line exists" proved a SUMMARY was printed; `measured > 0`
    // proves pairs were bracketed AND their results came back, which is the property this gate
    // needs — a pool that was reset and timestamps that executed.
    let art = boyko_app::profiling::artifact::Artifact::read(&artifact, RUN_TOKEN)
        .unwrap_or_else(|e| {
            panic!(
                "the armed worker completed but its artifact is unusable: {e}. Nothing then proves \
                 it reset a pool or wrote a timestamp, and two workers that both recorded nothing \
                 agree trivially.\n---- bench worker ----\n{bench_out}"
            )
        });
    let measured = art.census.measured;
    assert!(
        measured > 0,
        "the armed worker wrote an artifact whose census counts ZERO measured pairs, so nothing \
         proves it reset a pool or wrote a timestamp -- and two workers that both recorded nothing \
         agree trivially. Either the window never reached its {BENCH_FRAMES}-frame budget (the \
         BOYKO_WINDOW_FRAMES={BENCH_FRAME_CAP} cap fired first), or the recorder was disarmed. \
         Census: measured={} not_bracketed={} lost={} torn={}.\n---- bench worker ----\n{bench_out}",
        art.census.measured,
        art.census.not_bracketed,
        art.census.lost,
        art.census.torn
    );
    let _ = std::fs::remove_file(&artifact);

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
    // The counts are printed but NOT gated: a clean run is a silent one, so zero here is the
    // expected reading. They let a reader see on WHICH side the layer talked and how loudly -- an
    // armed run whose every message comes from the control is a different picture from one where
    // both streams talk.
    let messages_total = bench_count + control_count;
    println!(
        "VG R3 P4-2 query-validation gate: GREEN. Both workers completed with a live validation \
         messenger attached; the bench-armed one wrote an artifact counting {} MEASURED pair(s) \
         (so the reset and all {} timestamp writes executed and their results came back), and the \
         two normalized validation message sets are equal at {} key(s). The layer emitted \
         {messages_total} message(s) in total -- zero is the CORRECT reading for clean code, which \
         is why the oracle is gated on being armed rather than on having spoken. Raw message \
         counts: bench={bench_count}, control={control_count}.",
        measured,
        measured * 2,
        bench_keys.len(),
    );
}
