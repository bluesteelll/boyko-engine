//! **VG R3 piece 3 step P3-8 — gates G-P3-A / G-P3-B / G-P3-C on the `vb_occ_mixed` fixture.**
//!
//! The first fixture in this repository on which the occlusion cull can reject anything, and
//! therefore the first place the campaign's central conjunction is decidable:
//!
//! > the image produced with the cull ARMED is **byte-identical** to the image produced with it
//! > DISARMED, while the GPU reports a **nonzero deferral count** and, under FORCE-LATE, a
//! > **nonzero late-survivor count**, in the same run.
//!
//! Neither half alone is a gate. Byte-identity alone is satisfied by a cull that rejects nothing —
//! this campaign has shipped that failure six times. A count alone is satisfied by a cull that
//! deletes visible geometry. **The conjunction is the gate**, and the two halves live in two places:
//!
//! | half | where | why there |
//! |---|---|---|
//! | **G-P3-A**, byte-identity | `goldens/PINS.toml`'s four `vb_occ_mixed*` pins, measured by `scripts/golden.ps1`, their cross-agreement machine-checked by `vg_density_census.rs`'s `the_pins_declared_byte_identical_actually_agree` | a pin is a GPU render against a blessed hash and no `cargo test` drives that |
//! | **G-P3-B / G-P3-C**, the counts and the partition | this file | it needs the readback payload and the dumped pyramid from ONE frame of ONE process |
//!
//! # ⚠️ THE FRAME-INDEX TRAP, executed rather than described
//!
//! The pyramid's boot clear makes it all-zeros at birth, so **on frame 1 the cull provably defers
//! NOTHING**. A fixture capturing the first rendered frame would compare a cull that did nothing
//! against a cull that was off, get byte-identity, and prove nothing. Every gate below therefore
//! asserts the triple `probe.frame == dump.frame_index`, `probe.frame >= 3` and
//! `probe.gpu_frame == probe.frame`. Running the two captures in separate processes does NOT fix
//! this and must not be attempted: the readback payload would still be frame 1 and the only way to
//! green the clause would be to relax it.
//!
//! # ⚠️ THE RING ORDER IS DERIVED, NOT PREDICTED
//!
//! Marking a strict subset splits each mesh family into two archetypes, so the absolute ring index
//! of any one instance depends on which archetype the ECS query yields first. This file does not
//! guess: `vb_occ_mixed_scene::RING_SLOT_TO_SPAWN` enumerates the TWO layouts the fixture admits,
//! their candidate-offset sets are DISJOINT in both regimes, and [`derive_ring_layout`] identifies
//! which one the engine produced — or reds naming RING LAYOUT rather than the cull. A predicted
//! layout would have made a kernel iteration-order change read as a cull defect.
//!
//! # What these gates CANNOT claim
//!
//! * **Anything about the shipping barrier chain.** They run with `BOYKO_VB_CULL_READBACK` armed,
//!   which appends a TRANSFER read to three buffers. The PROBE-OFF chain is G-P3-F's job
//!   (`boyko_rhi_vulkan/tests/vb_barrier_stream_baseline.rs`), and that file is a hand-written
//!   REPLICA by its own admission.
//! * **The early phase's verdicts UNDER MOTION.** Clause 7 works only because plan D12 makes
//!   `P_prev == P_cur` bit-for-bit on a converged static frame, so the dumped pyramid IS the one the
//!   early phase read. Under motion, or under FORCE-LATE, the two genuinely differ and only one is
//!   dumped. Closing that would need a second dump pass; it is a known gap, not a discovered one.
//! * **That the late phase is load-bearing on the UNFORCED pin.** By D12 it correctly contributes
//!   ZERO pixels there. No image-level control for it exists or can exist on that pin; it is carried
//!   by clauses 5–7 and by the FORCE-LATE pin, and by nothing else.
//!
//! # THE CORRUPTION TABLE — including the controls that must NOT fire
//!
//! Reporting only the controls that fire is how a vacuous gate ships. Every row below is to be
//! EXECUTED by the orchestrator and its result published, whichever way it lands.
//!
//! | # | corruption | expected |
//! |---|---|---|
//! | **A1** | invert the verdict (`depth_near > occ`) | **RED** on `vb_occ_mixed` and `vb_occ_mixed_late`. **GREEN on `vb_occ_mixed_keep`, and that green is EXPECTED** — `FORCE_KEEP` short-circuits the `&& !(occ_flags & FORCE_KEEP)` guard, so no inverted instruction executes. ⚠️ The mechanism is NOT "the occluder is deferred": the occluder is the UNMARKED slab and the flag is set only on marked entities. It is the other half — the two marked-VISIBLE instances are deferred, then dropped by the inverted late test, and **vanish** |
//! | **A2** | `<=` instead of `<` in the verdict | **RED or GREEN, and the answer is a FINDING either way.** This fixture's occluder is strictly in front of every hidden instance by a factor of ~2.3 in reverse-Z depth, so equality is very unlikely to be reached and the control may simply not fire. **That non-firing is to be reported**, with the boundary case left to G-P3-D's constructed corpus, which plants a pyramid texel equal to a corner's computed `z_ndc` |
//! | **A3** | delete the late cull's `cmd_dispatch` ONLY (leave the pass declared and recorded) | **RED on `vb_occ_mixed_late`** — the record's `instanceCount` stays at the host seed `0`, so the two late instances vanish. Real because under FORCE-LATE the early cull writes no marked global into `vb_visible_instance`, so late-scope residue cannot coincidentally equal the survivor globals. ⚠️ **GREEN on the three unforced pins, by D12, and that green is EXPECTED.** ⚠️ Delete the DISPATCH, not the pass: deleting the pass trips the declare/record parity assert and would be recorded as "RED" for an unrelated reason |
//! | **A4** | force the early phase to defer nothing (`FORCE_KEEP`) | **GREEN** — that is the `vb_occ_mixed_keep` pin, and its green is a claim (the plumbing is inert), not an absence of one |
//! | **B0** | nudge one hidden instance so its rect straddles a 128 boundary | **RED on clause 0**, with the FIXTURE message — and in the ANALYTIC form (`vg_occ_verdict_census.rs`) **before any GPU runs**. That analytic leg is already executed and committed |
//! | **B1** | perturb, in the HOST's copy of the pyramid before running the oracle, exactly the texel `select_texels` reports for a NAMED deferred instance, in the direction that crosses its `depth_near` | **RED on clause 4/5 for that instance.** ⚠️ Not "perturb one texel": on this fixture a random texel need not be one of the four sampled for any candidate, so a blind perturbation could silently not fire |
//! | **B2** | **`keep += 1`** at the end of the late compaction — an OVER-count | **RED on clause 5's elementwise equality AND on its length half**, on `vb_occ_mixed_late` and on `vb_occ_mixed` alike. ⚠️ **`keep -= 1` is FORBIDDEN**: it is an UNDER-count (the wrong class), `keep` is a raw `uint`, the record word is the only bound on the draw and `robustBufferAccess` is OFF — a decrement at `keep == 0`, which D12 GUARANTEES on the converged unforced regime, yields `0xFFFFFFFF` instances: a **TDR, not a red** |
//! | **B2-bound** | — | ⚠️ `keep += 1` is in bounds only because this fixture's eight instances sit far below `INSTANCE_CAPACITY`. It must **not** be generalised to a full ring, where the last batch's `base + n_defer` can be `INSTANCE_CAPACITY` and the same argument applies |
//! | **B3** | capture frame 1 (patch `SETTLE_FRAMES` to 0) | **RED on clauses 1 and 8** — the frame-index trap, executed |
//! | **B4** | run on `vb_mesh` (no occlusion) | **RED on clause 1** — the non-vacuity clause fires, which is why it is an `assert` and not a report |
//! | **B5** | make the early phase's occlusion test read `base_extent` off by one | **RED on clause 7** with clause 2 still green — the early phase IS falsifiable on a converged frame, and this is the demonstration |
//! | **B6** | delete the late phase's write to `vb_indirect_late` | **RED on clause 5** (`instanceCount` stays 0 while `K_b` is non-empty) on `vb_occ_mixed_late` |
//! | **C1** | set `FORCE_LATE` and `FORCE_KEEP` together | must trip the `debug_assert` forbidding it rather than silently resolving |
//! | **F-M4a** | record the uniform's `vkCmdUpdateBuffer` AFTER `cmd_dispatch` — deterministic in SUBMISSION order | **RED on clause 8's third line**: `gpu_frame == frame − FRAMES_IN_FLIGHT`. It proves the INSTRUMENT is live; it does **not** test the barrier |
//! | **F-M4b** | move the fill after `record_vb_pass` but keep it before the dispatch — the REAL record-order defect | **GREEN / undetermined, published either way.** `record_vb_pass` records barriers only, so submission order still orders the write before the read, and a real missing edge is measured invisible here. A green means "this driver did not reorder", never "the barrier is present" |
//!
//! # Run
//!
//! ```text
//! cargo test -p boyko-app --test vb_occ_mixed -- --ignored --nocapture --test-threads=1
//! ```
//! with `BOYKO_DISABLE_VALIDATION=1`. Each driver spawns its own worker; a worker run directly SKIPS
//! (rather than looping forever) without its capture knobs.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::hzb::{HzbLayout, KeepReason, OcclusionVerdict, occlusion_verdict};
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};
use boyko_rhi_vulkan::present::{
    HZB_DUMP_FLAG_DEPTH_EARLY, HZB_DUMP_HEADER_BYTES, HZB_DUMP_HEADER_SCALAR_WORDS, HZB_DUMP_MAGIC,
    HZB_DUMP_SAMPLE_BYTES, HZB_DUMP_WORD_FLAGS, HZB_DUMP_WORD_FRAME_INDEX,
};

mod occ_fixture;
mod vb_inst_cull_scene;
mod vb_occ_mixed_scene;

use vb_inst_cull_scene::{CullProbe, parse_probe_line};
use vb_occ_mixed_scene::{
    BATCH_COUNT, EXTENT, HIDDEN_TOTAL, INSTANCES_PER_MESH, MARKED_TOTAL, MIXED_INSTANCES,
    RING_SLOT_TO_SPAWN, Role, VISIBLE_MARKED_TOTAL, frustum_survivors_of, instance_world_aabb,
    mesh_of_batch, ring_slot_instance, slots_with_role,
};

/// The worker both drivers re-execute. One worker, two regimes, selected by the env the driver
/// sets.
///
/// ⚠️ Since VG R3 piece 4 rung P4-4 `BOYKO_VG_OCC_FORCE` is decoded by `occ_fixture` at APP SETUP
/// and inserted as the `OcclusionForce` Resource, where it used to be read once inside
/// `GpuSceneBundles::boot`. A regime is still a PROCESS here — this worker inserts it once and no
/// system mutates it — but that is now a property of the fixture rather than of the engine, which
/// is why the artifact records the regime (`[probe] occ_regime` from the pushed word, `[host]
/// occ_force` from the live read) instead of the engine asserting it held still.
const WORKER: &str = "vb_occ_mixed_capture_worker";

/// The engine frame index a capture must be at least at (plan D1's convergence clause: the pyramid's
/// boot clear makes frame 1 defer provably nothing, and the fixed point holds from frame 2).
///
/// `>=`, never `==`: the drivers count PRESENTED frames while `frame_index` increments on every loop
/// iteration including recreate-skips, so the exact value is not predictable.
const MIN_CONVERGED_FRAME: u32 = 3;

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
    vb_occ_mixed_scene::spawn_mixed(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut geo_table,
        &dev,
        true,
    );
}

/// **THE WORKER** — one `VisibilityBuffer × Mesh` boot of the mixed scene with the pyramid armed and
/// THREE captures armed by the driver: the cull readback, the pyramid dump and the record probe.
///
/// All three complete and the loop exits through the ONE conjunction step P3-5 folded them into.
/// Arming the first two together is exactly what `vb_cull_hzb_pairing.rs` demonstrated; the record
/// probe is the third, and it is here because `draw_batches == 2` must come from the HOST's own
/// `[host]` table rather than from a number this file re-derives.
#[test]
#[ignore = "needs a real windowed GPU device; the G-P3-B/C drivers spawn it with all three capture knobs set"]
fn vb_occ_mixed_capture_worker() {
    // A worker booted without the knobs arms no capture, so the host loop has nothing to complete
    // and `app.run()` never returns — a hang, the worst failure mode a sweep can have.
    for knob in ["BOYKO_VB_CULL_READBACK", "BOYKO_HZB_DUMP", "BOYKO_VB_PROBE"] {
        if std::env::var(knob).is_err() {
            eprintln!(
                "{WORKER}: {knob} is unset -- SKIPPED. This worker exists to be spawned by its \
                 driver; booted without all three knobs it would render forever, since no armed \
                 capture could ever complete."
            );
            return;
        }
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb_occ_mixed", EXTENT, EXTENT));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    // `HzbMode::Build` is what makes `GBufferScene::hzb` `Some`, which since step P3-6 is a conjunct
    // of `path_vb_occlusion_split()` — without it the split disarms and every clause below would
    // adjudicate an unsplit frame.
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    // VG R3 piece 4 rung P4-4: the OWNER conjunct, armed through THE single insert site so the
    // vacuity control's one edit reds G-P3-B here as well as the pin-binary gate.
    //
    // The MODE is fixed by the fixture — `spawn_mixed(.., true)` above always marks, so this
    // worker is always a split worker — while the REGIME comes from the env the driver sets,
    // because the regime IS this file's one variable (`Regime::Unforced` vs `Regime::ForceLate`).
    // Until this rung that env was read inside `GpuSceneBundles::boot`; the decode moved to
    // `occ_fixture` and the pin file did not change.
    let (_, force) = occ_fixture::occlusion_from_env();
    occ_fixture::arm_occlusion_with(&mut app, boyko_render::OcclusionMode::TwoPhase, force);
    app.run();
}

// ===============================================================================================
// The artifacts, decoded
// ===============================================================================================

/// Little-endian `u32` at word index `i`.
fn word(bytes: &[u8], i: usize) -> u32 {
    let o = i * 4;
    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
}

/// The part of a `BOYKO_HZB_DUMP` file these gates read: the header's own numbers and the pyramid.
///
/// A LOCAL decoder rather than a borrowed one, for the reason `vb_occ_split_gate.rs` gives about its
/// own field reader: a gate that borrows another gate's parser inherits that gate's future edits.
/// The pyramid comparison against the oracle is `hzb_engine_pyramid_gate.rs`'s subject; what this
/// file needs is the pyramid as an ORACLE INPUT, plus the frame stamp.
struct PyramidDump {
    source: [u32; 2],
    levels: u32,
    flags: u32,
    frame_index: u32,
    /// Every mip, finest first, back to back, row-major — as `f32`, because that is what
    /// `occlusion_verdict` folds.
    pyramid: Vec<f32>,
}

fn decode_pyramid(bytes: &[u8], path: &Path) -> PyramidDump {
    assert!(
        bytes.len() >= HZB_DUMP_HEADER_BYTES as usize,
        "{}: {} bytes is shorter than the {}-byte header",
        path.display(),
        bytes.len(),
        HZB_DUMP_HEADER_BYTES
    );
    let magic = word(bytes, 0);
    assert_eq!(
        magic, HZB_DUMP_MAGIC,
        "{}: leading word is 0x{magic:08x}, not HZB_DUMP_MAGIC (0x{HZB_DUMP_MAGIC:08x}). A stale \
         file decoded as this run's evidence is what the driver's `remove_file` exists to prevent.",
        path.display()
    );
    let source = [word(bytes, 1), word(bytes, 2)];
    let levels = word(bytes, 3);
    let flags = word(bytes, HZB_DUMP_WORD_FLAGS);
    let frame_index = word(bytes, HZB_DUMP_WORD_FRAME_INDEX);

    let mut pyramid_texels = 0usize;
    for k in 0..levels as usize {
        let w0 = HZB_DUMP_HEADER_SCALAR_WORDS + 2 * k;
        pyramid_texels += word(bytes, w0) as usize * word(bytes, w0 + 1) as usize;
    }
    let depth_texels = source[0] as usize * source[1] as usize;
    let want = HZB_DUMP_HEADER_BYTES as usize
        + (2 * depth_texels + pyramid_texels) * HZB_DUMP_SAMPLE_BYTES as usize;
    assert_eq!(
        bytes.len(),
        want,
        "{}: {} bytes, but the header describes {want}. The file and its own header disagree, so \
         nothing below it can be trusted.",
        path.display(),
        bytes.len()
    );

    let pyramid_word0 = HZB_DUMP_HEADER_BYTES as usize / 4 + 2 * depth_texels;
    let pyramid =
        (0..pyramid_texels).map(|i| f32::from_bits(word(bytes, pyramid_word0 + i))).collect();
    PyramidDump { source, levels, flags, frame_index, pyramid }
}

/// The raw right-hand side of `table.key` in the record probe's flat TOML subset. Local, for the
/// same reason [`decode_pyramid`] is.
fn probe_field(src: &str, path: &str, file: &Path) -> String {
    let (table, key) = path.split_once('.').expect("a probe path is `table.key`");
    let mut inside = false;
    for line in src.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') && l.ends_with(']') {
            inside = l.trim_start_matches('[').trim_end_matches(']') == table;
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return v.trim().to_string();
        }
    }
    panic!("the record probe {} has no `{path}`", file.display())
}

fn probe_u32(src: &str, path: &str, file: &Path) -> u32 {
    probe_field(src, path, file).parse().unwrap_or_else(|_| panic!("`{path}` is not an integer"))
}

fn probe_bool(src: &str, path: &str, file: &Path) -> bool {
    match probe_field(src, path, file).as_str() {
        "true" => true,
        "false" => false,
        other => panic!("`{path}` is `{other}`, which is not a boolean"),
    }
}

/// Everything ONE worker process produced.
struct Capture {
    probe: CullProbe,
    dump: PyramidDump,
    draw_batches: u32,
    occlusion_instances: u32,
    scopes: u32,
    late_draws: u32,
    late_cull_dispatches: u32,
    late_seed_instances: u32,
}

/// Which regime the worker was booted in — the value of `BOYKO_VG_OCC_FORCE`, decoded once at app
/// setup by `occ_fixture` and inserted as the `OcclusionForce` Resource (VG R3 piece 4 rung P4-4;
/// it was a `GpuSceneBundles::boot` env read before that).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Regime {
    /// No force bit: the early phase decides from the pyramid. Plan D12's converged fixed point
    /// applies, so the correct late survivor count is ZERO.
    Unforced,
    /// `VB_CULL_OCC_FORCE_LATE`: the early phase defers EVERY marked instance regardless of the
    /// pyramid, so the late scope is the one that rasterises the survivors.
    ForceLate,
}

impl Regime {
    /// The env the driver sets for this regime.
    fn env(self) -> Option<(&'static str, &'static str)> {
        match self {
            Regime::Unforced => None,
            Regime::ForceLate => Some(("BOYKO_VG_OCC_FORCE", "late")),
        }
    }

    /// The roles the EARLY phase must defer in this regime — the roles the candidate list carries.
    fn deferred_roles(self) -> &'static [Role] {
        match self {
            Regime::Unforced => &[Role::Hidden],
            Regime::ForceLate => &[Role::Hidden, Role::Visible],
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Regime::Unforced => "vb_occ_mixed",
            Regime::ForceLate => "vb_occ_mixed_late",
        }
    }
}

/// Runs one worker with all three captures armed and returns everything it produced.
fn run_capture(regime: Regime) -> Capture {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let tag = regime.tag();
    let cull_out: PathBuf = std::env::temp_dir().join(format!("boyko_{tag}_cull.txt"));
    let dump_out: PathBuf = std::env::temp_dir().join(format!("boyko_{tag}_hzb.bin"));
    let probe_out: PathBuf = std::env::temp_dir().join(format!("boyko_{tag}_probe.toml"));
    // A stale file from a previous run that this run failed to overwrite would be read as this run's
    // evidence — and "the capture never ran" and "the capture left last run's file" are the same
    // bytes.
    for p in [&cull_out, &dump_out, &probe_out] {
        let _ = std::fs::remove_file(p);
    }

    let mut cmd = Command::new(&exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env("BOYKO_VB_CULL_READBACK", &cull_out)
        .env("BOYKO_HZB_DUMP", &dump_out)
        .env("BOYKO_VB_PROBE", &probe_out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // ⚠️ NONE of the three capture knobs above is removed, and that is deliberate: they must all
        // be armed in ONE process, because the cull's verdicts and the pyramid they were tested
        // against are only one frame's evidence if one frame produced both. The two UNRELATED
        // capture drivers are removed so a red here has one possible cause.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        // The regime is the ONE variable between the two drivers. Removed rather than left
        // inherited, so a stray shell value cannot silently make the unforced run a forced one.
        .env_remove("BOYKO_VG_OCC_FORCE");
    if let Some((k, v)) = regime.env() {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("invariant: the worker process spawns");
    assert!(status.success(), "the `{tag}` capture worker exited {status}");

    let cull_text = std::fs::read_to_string(&cull_out).unwrap_or_else(|e| {
        panic!(
            "`{tag}`: the CULL capture wrote no line at {} ({e}). A worker that renders and produces \
             nothing is an instrument failure, not an empty scene.",
            cull_out.display()
        )
    });
    let dump_bytes = std::fs::read(&dump_out).unwrap_or_else(|e| {
        panic!(
            "`{tag}`: the PYRAMID capture wrote no file at {} ({e}) while the cull capture wrote \
             one. Both drivers exit through ONE conjunction since step P3-5; a regression here \
             means one of them was given its own exit again.",
            dump_out.display()
        )
    });
    let probe_text = std::fs::read_to_string(&probe_out).unwrap_or_else(|e| {
        panic!("`{tag}`: the RECORD probe wrote no file at {} ({e})", probe_out.display())
    });

    Capture {
        probe: parse_probe_line(cull_text.trim()),
        dump: decode_pyramid(&dump_bytes, &dump_out),
        draw_batches: probe_u32(&probe_text, "host.draw_batches", &probe_out),
        occlusion_instances: probe_u32(&probe_text, "host.occlusion_instances", &probe_out),
        scopes: probe_u32(&probe_text, "probe.scopes", &probe_out),
        late_draws: probe_u32(&probe_text, "probe.late_draws", &probe_out),
        late_cull_dispatches: probe_u32(&probe_text, "probe.late_cull_dispatches", &probe_out),
        late_seed_instances: probe_u32(&probe_text, "probe.late_seed_instances", &probe_out),
    }
    .tap_instrument(&probe_text, &probe_out, tag)
}

impl Capture {
    /// The clause every other clause depends on: this boot really did resolve
    /// `VisibilityBuffer × Mesh` and really did split. A device that fails the VB capability probe
    /// degrades to `Deferred`, and then every count would be zero for an INSTRUMENT reason that
    /// reads exactly like "the cull decided nothing".
    fn tap_instrument(self, probe_text: &str, probe_out: &Path, tag: &str) -> Self {
        assert!(
            probe_bool(probe_text, "host.vb_path", probe_out)
                && probe_bool(probe_text, "host.mesh_leg", probe_out),
            "`{tag}`: the probed frame is not a `VisibilityBuffer x Mesh` frame, so its counts say \
             nothing about the cull. This is an instrument failure, not a gate result."
        );
        self
    }
}

// ===============================================================================================
// The ring layout, DERIVED
// ===============================================================================================

/// Identifies which of `vb_occ_mixed_scene::RING_SLOT_TO_SPAWN`'s two layouts the engine produced,
/// from the batch-local offsets of the observed candidate lists.
///
/// The two layouts' expected offset sets are DISJOINT in both regimes (`{1,2}` vs `{0,1}` unforced,
/// `{1,2,3}` vs `{0,1,2}` forced), so this is an identification and not a guess — and if the
/// observed offsets match NEITHER, or the two batches disagree, it panics naming RING LAYOUT rather
/// than the cull.
fn derive_ring_layout(cap: &Capture, regime: Regime) -> usize {
    let mut chosen: Option<usize> = None;
    for b in 0..BATCH_COUNT {
        let (base, members) = &cap.probe.late_cand[b];
        let observed: Vec<usize> =
            members.iter().map(|g| (g - base) as usize).collect();
        let mut matched: Option<usize> = None;
        for l in 0..RING_SLOT_TO_SPAWN.len() {
            if slots_with_role(mesh_of_batch(b), l, regime.deferred_roles()) == observed {
                assert!(
                    matched.is_none(),
                    "batch {b}: candidate offsets {observed:?} match BOTH ring layouts. The two \
                     layouts must be distinguishable or this identification is a coin toss."
                );
                matched = Some(l);
            }
        }
        let l = matched.unwrap_or_else(|| panic!(
            "RING LAYOUT: batch {b}'s candidate offsets are {observed:?} (base {base}, members \
             {members:?}), which match NEITHER admissible layout. The fixture admits exactly two — \
             `[U,H,H,V]` and `[H,H,V,U]`, the two archetype iteration orders — and their expected \
             offsets in the {regime:?} regime are {:?} and {:?}. A third shape means either the \
             gather no longer scatters in archetype-then-spawn order (a KERNEL change, and this \
             message is how it should be read) or the cull deferred the wrong instances. Those two \
             are separated by the count clauses, which run after this.",
            slots_with_role(mesh_of_batch(b), 0, regime.deferred_roles()),
            slots_with_role(mesh_of_batch(b), 1, regime.deferred_roles()),
        ));
        match chosen {
            None => chosen = Some(l),
            Some(prev) => assert_eq!(
                prev, l,
                "RING LAYOUT: batch 0 identified layout {prev} and batch {b} layout {l}. One ECS \
                 archetype iteration order serves the whole world, so the two batches cannot differ."
            ),
        }
    }
    chosen.expect("invariant: BATCH_COUNT >= 1")
}

// ===============================================================================================
// The gate body, shared by both regimes
// ===============================================================================================

/// The oracle's verdict for [`MIXED_INSTANCES`] index `i` against the DUMPED pyramid.
fn verdict(dump: &PyramidDump, layout: &HzbLayout, i: usize) -> OcclusionVerdict {
    let (mn, mx) = instance_world_aabb(i);
    occlusion_verdict(layout, &dump.pyramid, &vb_occ_mixed_scene::view_proj_rows(), mn, mx)
}

/// Runs every clause of G-P3-B over one capture. `regime` decides clause 7's form and nothing else.
#[allow(clippy::too_many_lines)]
fn assert_partition(cap: &Capture, regime: Regime) {
    let tag = regime.tag();

    // ---- the instrument: the fixture reached the ring, and the recorder split ------------------
    assert_eq!(
        cap.occlusion_instances as usize, MARKED_TOTAL,
        "`{tag}`: {} of the {MARKED_TOTAL} instances carried `OcclusionCulling` into the ring. The \
         marker is queued into the SPAWN flush, so a shortfall is the gather's marker lane, not a \
         timing question.",
        cap.occlusion_instances
    );
    assert_eq!(
        cap.draw_batches as usize, BATCH_COUNT,
        "`{tag}`: {} draw batches. Batches bucket per `MeshHandle` and this fixture registers TWO \
         meshes; at ONE batch the per-batch record offset, the `batch_count` bound and the late \
         loop are all unfalsifiable, which is the debt `vb_occ_split_gate.rs` records as piece 3's \
         first gate.",
        cap.draw_batches
    );
    assert_eq!(cap.scopes, 2, "`{tag}`: the recorder reported {} raster scopes", cap.scopes);
    assert_eq!(
        cap.late_draws, cap.draw_batches,
        "`{tag}`: {} late draws against {} batches",
        cap.late_draws, cap.draw_batches
    );
    assert_eq!(
        cap.late_cull_dispatches, 1,
        "`{tag}`: {} late cull dispatches on a split frame",
        cap.late_cull_dispatches
    );
    assert_eq!(
        cap.late_seed_instances, 0,
        "`{tag}`: the HOST seeded {} late instances. The late cull must be the ONLY producer of a \
         nonzero `instanceCount`, which is what makes a missing late-cull dispatch a BLANK scope \
         instead of a draw of untested geometry.",
        cap.late_seed_instances
    );
    assert_ne!(
        cap.dump.flags & HZB_DUMP_FLAG_DEPTH_EARLY,
        0,
        "`{tag}`: the dump header's HZB_DUMP_FLAG_DEPTH_EARLY is clear. The bit is latched AT the \
         early-depth copy inside the recorder, so this frame did not split -- and every clause \
         below would then be adjudicating the wrong pyramid."
    );
    assert_eq!(
        cap.dump.source,
        [EXTENT, EXTENT],
        "`{tag}`: the dump was taken at {:?}, not the {EXTENT}x{EXTENT} this fixture's whole pixel \
         arithmetic is stated against",
        cap.dump.source
    );

    let layout = HzbLayout::new(cap.dump.source[0], cap.dump.source[1])
        .expect("invariant: the engine built a pyramid over this extent");
    assert_eq!(
        cap.dump.levels,
        layout.levels(),
        "`{tag}`: the dump reports {} levels, the oracle's layout {}",
        cap.dump.levels,
        layout.levels()
    );
    assert_eq!(cap.probe.batches, BATCH_COUNT, "`{tag}`: {} drawn batches", cap.probe.batches);
    // Every per-batch lane must carry one entry per drawn batch BEFORE anything indexes them: a
    // short lane would panic with an index message that names the reader instead of the emitter.
    for (name, len) in [
        ("inst", cap.probe.inst.len()),
        ("vis", cap.probe.vis.len()),
        ("late_cnt_pre", cap.probe.late_cnt_pre.len()),
        ("late_cnt_post", cap.probe.late_cnt_post.len()),
        ("late_ic", cap.probe.late_ic.len()),
        ("late_cand", cap.probe.late_cand.len()),
        ("late_surv", cap.probe.late_surv.len()),
    ] {
        assert_eq!(
            len, BATCH_COUNT,
            "`{tag}`: the probe's `{name}=` lane carries {len} entries for {BATCH_COUNT} drawn \
             batches. Every lane is emitted per DRAWN batch, so a short one means the emitter and \
             this reader disagree about the frame's batch count."
        );
    }

    // ---- clause 0: FIXTURE PRECONDITION VG-P3-MIXED-OCCLUDES, MEASURED form --------------------
    //
    // FIRST, because every clause below is meaningless on a scene that cannot occlude. Its message
    // says FIXTURE, and it is textually distinct from clause 1's so a fixture error can never be
    // mistaken for a cull defect. The ANALYTIC form of the same precondition lives in
    // `vg_occ_verdict_census.rs` and runs without a GPU.
    for (i, inst) in MIXED_INSTANCES.iter().enumerate() {
        let want = match inst.role {
            Role::Hidden => OcclusionVerdict::Reject,
            Role::Visible => OcclusionVerdict::Keep(KeepReason::NotOccluded),
            // The unmarked pair is never occlusion-tested by the engine (the capability is
            // structural), so the oracle's verdict on them is not a claim this fixture makes.
            Role::Occluder | Role::Filler => continue,
        };
        let got = verdict(&cap.dump, &layout, i);
        assert_eq!(
            got, want,
            "FIXTURE PRECONDITION -- the mixed scene's geometry does not produce the intended \
             occlusion at this framebuffer size; this is a FIXTURE error, not an engine defect. \
             `{}` ({:?}) is {got:?} over the DUMPED pyramid, expected {want:?}. (`{tag}`)",
            inst.name, inst.role
        );
    }

    // ---- the ring layout, identified before any per-candidate oracle runs ----------------------
    let ring = derive_ring_layout(cap, regime);

    // ---- the per-batch clauses ------------------------------------------------------------------
    let mut total_defer = 0usize;
    let mut total_keep = 0usize;
    for b in 0..BATCH_COUNT {
        let mesh = mesh_of_batch(b);
        let (vis_base, early) = &cap.probe.vis[b];
        let (cand_base, cands) = &cap.probe.late_cand[b];
        let (surv_base, survivors) = &cap.probe.late_surv[b];
        assert!(
            vis_base == cand_base && cand_base == surv_base,
            "`{tag}` batch {b}: the three per-batch regions report bases {vis_base}/{cand_base}/\
             {surv_base}. They are all `VbBatchDesc::base_instance`, so a disagreement is a \
             formatter or a descriptor defect, not a cull one."
        );
        let base = *vis_base;
        let k = early.len();
        let n_defer = cands.len();
        total_defer += n_defer;

        assert_eq!(
            cap.probe.inst[b] as usize, k,
            "`{tag}` batch {b}: the record's instanceCount is {} but the survivor region holds {k}",
            cap.probe.inst[b]
        );
        assert_eq!(
            cap.probe.late_cnt_pre[b] as usize, n_defer,
            "`{tag}` batch {b}: `late_cnt_pre` is {} but the candidate region holds {n_defer}",
            cap.probe.late_cnt_pre[b]
        );

        // ---- clause 2: nothing was DROPPED --------------------------------------------------
        let frustum_survivors = frustum_survivors_of(mesh);
        assert_eq!(
            k + n_defer,
            frustum_survivors,
            "`{tag}` batch {b}: {k} drawn early + {n_defer} deferred = {} against {frustum_survivors} \
             frustum survivors. A cull that 'defers' by dropping instances outright fails HERE, and \
             this is the only clause that can see it -- byte-identity cannot, because a dropped \
             instance that was occluded anyway changes no pixel.",
            k + n_defer
        );

        // ---- clause 2b: INVARIANT VG-P3-RECOVERY ---------------------------------------------
        let mut union: Vec<u32> = early.iter().chain(cands.iter()).copied().collect();
        union.sort_unstable();
        let expected: Vec<u32> = (0..frustum_survivors as u32).map(|s| base + s).collect();
        assert_eq!(
            union, expected,
            "`{tag}` batch {b}: the early survivors {early:?} and the candidates {cands:?} do not \
             partition this batch's frustum survivors {expected:?}. Their union must be exactly the \
             survivor set and they must be DISJOINT -- that pair IS the gate for INVARIANT \
             VG-P3-RECOVERY, which round 1 stated and left unasserted."
        );

        // ---- clause 3: the candidate list is well formed ---------------------------------------
        for w in cands.windows(2) {
            assert!(
                w[0] < w[1],
                "`{tag}` batch {b}: the candidate list {cands:?} is not strictly ascending. A \
                 repeat means the compaction cursor did not advance, so one instance is written \
                 twice while another is dropped."
            );
        }
        for c in cands {
            assert!(
                (base..base + frustum_survivors as u32).contains(c),
                "`{tag}` batch {b}: candidate {c} is outside this batch's region \
                 [{base}, {}). A global index from another batch's region would be drawn by the \
                 wrong draw record.",
                base + frustum_survivors as u32
            );
        }

        // ---- clause 4: the oracle's kept subsequence, derived from the CANDIDATES ---------------
        //
        // `K_b` comes from the candidate list and the DUMPED pyramid, never from the count the GPU
        // wrote — round 1's clause 5 compared the GPU's number against itself.
        let mut k_b: Vec<u32> = Vec::new();
        for c in cands {
            let slot = (c - base) as usize;
            assert!(
                slot < INSTANCES_PER_MESH,
                "`{tag}` batch {b}: candidate {c} maps to ring slot {slot}"
            );
            let i = ring_slot_instance(mesh, ring, slot);
            if verdict(&cap.dump, &layout, i).is_keep() {
                k_b.push(*c);
            }
        }
        total_keep += k_b.len();

        // ---- clause 5: the GPU's survivors ARE `K_b`, elementwise -------------------------------
        assert_eq!(
            cap.probe.late_ic[b] as usize,
            k_b.len(),
            "`{tag}` batch {b}: the GPU wrote instanceCount={} while the oracle keeps {} of the {} \
             candidates ({k_b:?}). An OVER-count draws a valid-looking global the late test \
             rejected; an UNDER-count deletes visible geometry.",
            cap.probe.late_ic[b],
            k_b.len(),
            cands.len()
        );
        assert_eq!(
            survivors.as_slice(),
            k_b.as_slice(),
            "`{tag}` batch {b}: the GPU's late survivor prefix is {survivors:?}, the oracle's \
             {k_b:?}. This is the ORACLE EQUIVALENCE -- an independent implementation of the same \
             predicate over the same numbers. A disagreement here is a FINDING; it must be \
             reported, never 'fixed' by editing the expectation."
        );

        // ---- clause 6: the late phase did not clobber the early count ---------------------------
        assert_eq!(
            cap.probe.late_cnt_post[b], cap.probe.late_cnt_pre[b],
            "`{tag}` batch {b}: `vb_late_count` moved from {} to {} across the late phase. The late \
             cull reads that word as its loop bound and must not write it.",
            cap.probe.late_cnt_pre[b], cap.probe.late_cnt_post[b]
        );
    }

    // ---- clause 1: the NON-VACUITY clause, an assert and not a report ---------------------------
    assert!(
        total_defer > 0,
        "`{tag}`: the GPU deferred NOTHING. Every image gate in this campaign is satisfied by a \
         cull that decides nothing -- that is what six shipped green gates had in common -- so this \
         clause is what separates 'rejected' from 'never ran'. On this fixture four marked \
         instances sit wholly inside the occluder's silhouette, and clause 0 above has already \
         confirmed the ORACLE rejects them over this very pyramid."
    );

    // ---- clause 7: phase agreement (plan D12) ---------------------------------------------------
    match regime {
        Regime::Unforced => {
            assert_eq!(
                total_defer, HIDDEN_TOTAL,
                "`{tag}`: the early phase deferred {total_defer} instances, expected the \
                 {HIDDEN_TOTAL} that are wholly inside the occluder's silhouette"
            );
            assert_eq!(
                total_keep, 0,
                "`{tag}`: the oracle keeps {total_keep} of the {total_defer} candidates over the \
                 DUMPED pyramid. On a converged static frame it must keep NONE: an instance the \
                 early phase rejected writes no depth, so the depth and therefore the pyramid are a \
                 fixed point, and the late phase evaluates ONE predicate over the SAME bytes with \
                 the SAME view-projection. A nonzero here is drift between what the early phase \
                 decided and what the same predicate says over the same pyramid -- in either \
                 direction, and it is the one place a wrong early matrix, a wrong extent or a \
                 divergent phase branch becomes visible on a static scene."
            );
        }
        Regime::ForceLate => {
            assert_eq!(
                total_defer, MARKED_TOTAL,
                "`{tag}`: FORCE_LATE must defer EVERY marked instance regardless of the pyramid; \
                 {total_defer} of {MARKED_TOTAL} were deferred"
            );
            assert!(
                total_keep > 0 && total_keep < total_defer,
                "`{tag}`: the late phase kept {total_keep} of {total_defer} candidates. The bound \
                 is TWO-SIDED on purpose and is derived, never a hard-coded {VISIBLE_MARKED_TOTAL}: \
                 the lower half says the late raster path produces geometry at all, and the UPPER \
                 half says the late test REJECTED something as well -- without it, a late phase \
                 that keeps everything satisfies the clause."
            );
        }
    }

    // ---- clause 8: the frame-index triple --------------------------------------------------------
    assert_eq!(
        cap.probe.frame, cap.dump.frame_index,
        "`{tag}`: the cull probe latched engine frame {} and the pyramid dump's header was stamped \
         with {}. Both are the runner's own per-iteration counter, taken by the frame that ran, so \
         a difference means the two captures describe two frames -- and every comparison above that \
         treats the cull's verdicts and this pyramid as one frame's evidence is void.",
        cap.probe.frame, cap.dump.frame_index
    );
    assert!(
        cap.probe.frame >= MIN_CONVERGED_FRAME,
        "`{tag}`: the capture came from engine frame {}, below the converged floor \
         {MIN_CONVERGED_FRAME}. The pyramid's boot clear makes frame 1 defer PROVABLY nothing, so a \
         payload from there is green while proving nothing.",
        cap.probe.frame
    );
    assert_eq!(
        cap.probe.gpu_frame, cap.probe.frame,
        "`{tag}`: the CULL read frame index {} out of `VbCullUniform` while the host latched {}. \
         This is control F-M4a's channel and it is the FIRST fixture that can assert it -- every \
         earlier readback fixture marked nothing, so the shader never wrote the word and the probe \
         reported the boot prefill. A difference of exactly FRAMES_IN_FLIGHT means the uniform's \
         `vkCmdUpdateBuffer` is recorded AFTER the dispatch. ⚠️ It proves the INSTRUMENT is live; \
         it does NOT test the intra-pass TRANSFER->COMPUTE barrier, which has no executable red on \
         this machine.",
        cap.probe.gpu_frame, cap.probe.frame
    );

    println!(
        "VG R3 P3-8 {tag}: frame={} gpu_frame={} batches={} ring_layout={} S n_defer={} S |K_b|={} \
         late_ic={:?} pyramid={} texels. The counts are the GPU's; `K_b` is the host oracle's over \
         the DUMPED pyramid, derived from the CANDIDATE list and never from the count the GPU wrote.",
        cap.probe.frame,
        cap.probe.gpu_frame,
        cap.probe.batches,
        ring,
        total_defer,
        total_keep,
        cap.probe.late_ic,
        cap.dump.pyramid.len(),
    );
}

// ===============================================================================================
// The gates
// ===============================================================================================

/// The fixture's own arithmetic, checked without a device — so a constant-table edit reds in a plain
/// `cargo test` rather than at the next GPU run.
#[test]
fn the_mixed_fixture_is_internally_consistent() {
    vb_occ_mixed_scene::assert_fixture_invariants();
    assert_eq!(
        frustum_survivors_of(vb_occ_mixed_scene::MixedMesh::Sphere),
        INSTANCES_PER_MESH,
        "every sphere instance must survive the FRUSTUM cull -- clause 2 compares `k + n_defer` \
         against this number, and a fixture that drifted off screen would make that clause a \
         statement about the frustum arm instead of the occlusion arm"
    );
    assert_eq!(
        frustum_survivors_of(vb_occ_mixed_scene::MixedMesh::Cube),
        INSTANCES_PER_MESH,
        "every cube instance must survive the FRUSTUM cull"
    );
}

/// **GATE G-P3-B** — on the UNFORCED, converged pin the GPU deferred something, and it partitioned
/// EXACTLY what the oracle says.
///
/// The load-bearing gate of piece 3. See the module header for the eight clauses, for what it cannot
/// claim, and for the corruption table.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); the orchestrator runs it with --test-threads=1"]
fn vb_occ_mixed_partition_matches_the_oracle() {
    let cap = run_capture(Regime::Unforced);
    assert_partition(&cap, Regime::Unforced);
}

/// **GATE G-P3-C** — under FORCE-LATE the late scope actually rasterises, and the ordering is real.
///
/// `VB_CULL_OCC_FORCE_LATE` makes the early phase defer EVERY marked instance regardless of the
/// pyramid, which is the ONLY regime in which a static scene can reach three properties:
///
/// 1. **The late raster path produces correct pixels.** The six marked instances go through
///    `vb_set0_late`, through the survivor-indirection bit, with a GPU-written `instanceCount`; the
///    two marked-VISIBLE ones survive, so `0 < Σ n_keep < Σ n_defer` — asserted in that two-sided
///    form, never as a hard-coded count, because the number is a property of the geometry and the
///    geometry is what clause 0 pins.
/// 2. **The ordering.** The early depth contains only the unmarked pair, so
///    `depth_early ≠ depth_final` BY CONSTRUCTION and G-P3-E's two-sided clause is non-vacuous.
///    That clause is asserted in `hzb_engine_pyramid_gate.rs`, over the same regime.
/// 3. **`late_draws` and `late_cull_dispatches` at `draw_batches == 2`** — the per-batch record
///    offset evaluated at `i > 0`, which is the debt `vb_occ_split_gate.rs:43-44` records as piece
///    3's first gate and which no golden covers.
///
/// ⚠️ **Why the fixture must be MIXED.** With every instance marked, FORCE-LATE empties the early
/// depth entirely — every texel the reverse-Z far plane — which trips the SHIPPED non-vacuity
/// clauses in `hzb_engine_pyramid_gate.rs`. The unmarked filler exists precisely to populate it.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); the orchestrator runs it with --test-threads=1"]
fn vb_occ_mixed_force_late_rasterises() {
    let cap = run_capture(Regime::ForceLate);
    assert_partition(&cap, Regime::ForceLate);
    assert_eq!(
        cap.probe.drawn_instances() as usize,
        MIXED_INSTANCES.len() - MARKED_TOTAL,
        "vb_occ_mixed_late: the EARLY scope drew {} instances. Under FORCE_LATE it must draw only \
         the UNMARKED pair -- the occluding slab and the far filler -- and that is exactly what \
         keeps the early depth from being a constant field.",
        cap.probe.drawn_instances()
    );
}
