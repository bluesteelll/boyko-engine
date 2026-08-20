//! VB-SV0 DP6 rung **DP6-0b** (`docs/VB-SV0-DP6-DESIGN.md` §R4) — the repaired instrument's own
//! gate: the per-leg expectation table, the `[e6 → b11]` gap, and the source pins that keep the
//! derived id un-stamped.
//!
//! # What this file is for
//!
//! DP6-0b restamps ids 10/11 to `BOTTOM_OF_PIPE` and mints `ZONE_VB_PRODUCE_RUN` (12),
//! `ZONE_VB_PRESHADE` (13) and the derived `ZONE_VB_PRODUCE_NET` (14). Every DP6 gate then reads
//! ONE quantity, `NET = PRODUCE_RUN − PRESHADE`, formed per frame. That instrument is only worth
//! reading if three things are checked, and none of them was checkable before:
//!
//! 1. **Which zones stamp on which leg.** The doc comments at `gpu_zone.rs` used to cite a
//!    `280/560` pair-count pin in `vb_bench_query_validation` — a pin that **does not exist** (that
//!    test asserts `bench_ok == control_ok`, `measured > 0` and validation-message-set equality, and
//!    counts no pairs). A pair count is refused as a replacement with a reason: it is leg-dependent,
//!    it would red on every legitimate zone addition, and it has already failed to red on two. The
//!    **expectation table** below asserts the invariant those comments were reaching for, per leg,
//!    and it reds in BOTH directions — a `Required` zone missing and a `Forbidden` zone present.
//! 2. **The unbracketed `vb_viewt` dispatch.** It is checked without minting a zone: the gap
//!    between id 6's END and id 11's BEGIN IS that dispatch, measured at 5 248 ns on this box.
//! 3. **That id 14 is never stamped.** No recorder may open a pair for the derived id, and an
//!    artifact cannot say so — a derived row and a bracketed row look identical in the file. The
//!    check is therefore a SOURCE pin, and it runs without a GPU.
//!
//! # The GPU half is `#[ignore]`; the logic half is not
//!
//! The expectation table, the gap arithmetic and the source pins are pure functions over rows, so
//! their red-capability is demonstrated by unit tests in this file that run on any machine. The
//! two windowed drivers that produce real rows need a GPU: run with `BOYKO_DISABLE_VALIDATION=1`
//! and `--test-threads=1`.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use boyko_app::prelude::*;
use boyko_app::profiling::artifact::{Artifact, OrderCensus, ZoneLabel, ZoneRow};
// ONE expectation vocabulary for the whole rung: the reducer declares it, this harness consumes it.
// A second local enum of the same shape would let the artifact's producer and its reader disagree
// about what `Forbidden` means while both compiled.
use boyko_app::profiling::reduce::{
    Expect, VB_CHAIN_FUSED, VB_CHAIN_SPLIT, VB_DERIVED_FUSED, VB_DERIVED_SPLIT,
};
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::{
    GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig, SsaoConfig,
    SsaoQuality,
};
use boyko_rhi_vulkan::present::gpu_zone::{
    ZONE_VB_GEO, ZONE_VB_HZB_BUILD, ZONE_VB_PRESHADE, ZONE_VB_PRODUCE_NET, ZONE_VB_PRODUCE_RUN,
    ZONE_VB_RUN, ZONE_VB_SDF_MESH, ZONE_VB_SHADE,
};

mod sv0_scene;

// ===============================================================================================
// Knobs
// ===============================================================================================

/// The worker every leg re-executes.
const WORKER: &str = "vb_sv0_produce_run_worker";

/// Which boot the worker builds: `fused` = `[vb_both_sdf]`, `split` = `[vb_both_ssao]`.
const ENV_FIXTURE: &str = "BOYKO_DP6_FIXTURE";

/// Timed frames per worker, past the runner's own warmup. **ODD**, for the reason
/// `vg_occ_split_timing` states: an even count makes every published median the mean of two
/// samples, which is a value no frame had and half a tick off the timestamp lattice.
const BENCH_FRAMES: u32 = 221;

/// The `vb_viewt` pre-tail dispatch, measured on this box as the `[e6 → b11]` gap
/// (`gpu_zone.rs`'s `zone_begin_stage` doc). The bounds below are DELIBERATELY generous: the claim
/// under test is PRESENCE — a dispatch is either in that gap or it is not — and a tight band would
/// make the check a measurement of a dispatch nobody is trying to price here.
const VIEWT_GAP_NS: f64 = 5_248.0;

/// A gap at or below this is "no dispatch ran here". One device timer step is 512 ns on this box;
/// four of them is comfortably below the smallest full-screen dispatch and comfortably above the
/// stamp-to-stamp floor.
const NO_DISPATCH_NS: f64 = 2_048.0;

// ===============================================================================================
// The expectation table
// ===============================================================================================

// The leg's expectations are spelled ONCE, in the reducer, and imported here — see the header
// import block. `zone_declarations_agree_with_the_reducer` then checks that this file's per-leg
// TABLE and the reducer's chain/derived DECLARATIONS name the same zones with compatible
// expectations, which is the link the two spellings previously lacked.

/// One row of a leg's expectation table.
#[derive(Clone, Copy, Debug)]
struct Cell {
    zone: u16,
    name: &'static str,
    expect: Expect,
}

/// The FUSED leg's table — `[vb_both_sdf]`, `mesh_geo_shade_split == false`.
///
/// `sv0_armed` is the `BOYKO_SDF_MESH` arm: the dedicated prepass is recorded only under
/// `plan.sv0_pass`, so id 10 is the one cell that moves with the arm rather than with the leg.
fn table_fused(sv0_armed: bool) -> Vec<Cell> {
    vec![
        Cell { zone: ZONE_VB_RUN, name: "vb_run", expect: Expect::Required },
        Cell { zone: ZONE_VB_HZB_BUILD, name: "vb_hzb_build", expect: Expect::Required },
        Cell {
            zone: ZONE_VB_SDF_MESH,
            name: "vb_sdf_mesh",
            expect: if sv0_armed { Expect::Required } else { Expect::Forbidden },
        },
        // No split ⇒ no `vb_geo` and no pre-shade stretch. Both are FORBIDDEN and not merely
        // absent-by-accident: this is the cell that reds if a future edit opens either bracket
        // outside `if scene.path_vb_split()`.
        Cell { zone: ZONE_VB_GEO, name: "vb_geo", expect: Expect::Forbidden },
        Cell { zone: ZONE_VB_PRESHADE, name: "vb_preshade", expect: Expect::Forbidden },
        Cell { zone: ZONE_VB_PRODUCE_RUN, name: "vb_produce_run", expect: Expect::Required },
        Cell { zone: ZONE_VB_SHADE, name: "vb_shade", expect: Expect::Required },
        // The DERIVED row. Required as a ROW (the reducer forms it every frame) and forbidden as a
        // STAMP, which no artifact can express — see `no_recorder_stamps_the_derived_zone`.
        Cell { zone: ZONE_VB_PRODUCE_NET, name: "vb_produce_net", expect: Expect::Required },
    ]
}

/// The SPLIT leg's table — `[vb_both_ssao]`, `mesh_geo_shade_split == true`.
fn table_split(sv0_armed: bool) -> Vec<Cell> {
    vec![
        Cell { zone: ZONE_VB_RUN, name: "vb_run", expect: Expect::Required },
        Cell { zone: ZONE_VB_HZB_BUILD, name: "vb_hzb_build", expect: Expect::Required },
        Cell {
            zone: ZONE_VB_SDF_MESH,
            name: "vb_sdf_mesh",
            expect: if sv0_armed { Expect::Required } else { Expect::Forbidden },
        },
        Cell { zone: ZONE_VB_GEO, name: "vb_geo", expect: Expect::Required },
        Cell { zone: ZONE_VB_PRESHADE, name: "vb_preshade", expect: Expect::Required },
        Cell { zone: ZONE_VB_PRODUCE_RUN, name: "vb_produce_run", expect: Expect::Required },
        Cell { zone: ZONE_VB_SHADE, name: "vb_shade", expect: Expect::Required },
        Cell { zone: ZONE_VB_PRODUCE_NET, name: "vb_produce_net", expect: Expect::Required },
    ]
}

/// Checks one leg's rows against its table. `Ok(())` or the first breach, named.
///
/// A pure function over rows so that both of its directions can be driven without a GPU — the
/// property that separates a gate from a claim about one.
fn check_expectations(rows: &[ZoneRow], table: &[Cell]) -> Result<(), String> {
    for c in table {
        let row = rows.iter().find(|r| r.zone == c.zone);
        match (c.expect, row) {
            (Expect::Required, None) => {
                return Err(format!(
                    "zone {} (`{}`) is Required on this leg and has NO row: the bracket did not \
                     stamp, which on this leg means the recorder skipped it",
                    c.zone, c.name
                ));
            }
            (Expect::Required, Some(r)) if r.label != ZoneLabel::Measured => {
                return Err(format!(
                    "zone {} (`{}`) is Required and its row is `{:?}` rather than Measured",
                    c.zone, c.name, r.label
                ));
            }
            (Expect::Forbidden, Some(r)) => {
                return Err(format!(
                    "zone {} (`{}`) is Forbidden on this leg and yet has a `{:?}` row with n={}: \
                     a bracket opened where this leg records no such pass",
                    c.zone, c.name, r.label, r.n
                ));
            }
            // The two satisfied cases, spelled rather than wildcarded: a `_` arm would silently
            // accept a third `Expect` variant as "nothing to check", which is how a policy grows a
            // state nobody implemented. `flag_code`'s non-`_` match is the tree's precedent.
            (Expect::Required, Some(_)) | (Expect::Forbidden, None) => {}
        }
    }
    Ok(())
}

/// The `[e6 → b11]` gap — the unbracketed `vb_viewt` pre-tail dispatch, checked without a zone.
///
/// Returns `None` when either end is missing, which is not a failure here: on a fused leg there is
/// no id 11 at all and the gap is not a statement about anything.
fn viewt_gap_ns(rows: &[ZoneRow]) -> Option<f64> {
    let e6 = rows.iter().find(|r| r.zone == ZONE_VB_HZB_BUILD)?.end_off_ns;
    let b11 = rows.iter().find(|r| r.zone == ZONE_VB_GEO)?.begin_off_ns;
    Some(b11 - e6)
}

/// Verdict on a gap against whether the leg expects `vb_viewt` to run.
///
/// The predicate the expectation is derived from is the two-arm one at `gpu_scene/mod.rs`:
/// `VB ∧ mesh_leg ∧ ((¬sdf_leg ∧ aa == Taa) ∨ (split ∧ ssao))`. On `[vb_both_ssao]` arm (b) fires
/// on BOTH sides of DP6, which is what makes the dispatch common-mode inside the run bracket.
///
/// # ⚠️ DEVIATION — the `Forbidden` side is not live on any bootable fixture (W9)
///
/// The design's §R4.3.6 gives `vb_viewt` a per-side cell — `Forbidden` on `[vb_both_sdf]` both
/// sides, `Required` on `[vb_both_ssao]` both sides — *"checked mechanically without minting a
/// zone: the `[e6 → b11]` gap must be ≈ 5 248 ns where `Required` and ≈ 0 where `Forbidden`"*.
/// **On `[vb_both_sdf]` there is no `b11` at all**: id 11 is stamped only inside
/// `if scene.path_vb_split()`, and that fixture is fused, so the gap has no second end and the
/// arithmetic has no subject. As specified the `Forbidden` cell is unimplementable at this rung.
///
/// What ships instead: the fused driver asserts [`viewt_gap_ns`] is **`None`** — "the check does
/// not apply here", which is a different statement from "the check passed" and is the one that is
/// true. The `Forbidden` ARM of this function is exercised by arithmetic in the unit tests below,
/// so it is not dead, and it becomes live on a real leg at **DP6a**, where `[vb_both_sdf]` gains
/// the split (id 11 appears) while still carrying no `SsaoConfig` (arm (b) stays dead) — the first
/// boot on which "id 11 exists and `vb_viewt` must not have run" is a statement about a frame.
///
/// # Boundary ownership
///
/// `NO_DISPATCH_NS` belongs to the `Required` side: a gap of exactly that value passes `Required`
/// and fails `Forbidden`, so every value satisfies **exactly one** of the two verdicts and neither
/// a `>` / `>=` slip nor a value landing on the boundary can make both read green.
fn check_viewt_gap(gap_ns: f64, expected_to_run: bool) -> Result<(), String> {
    if expected_to_run {
        // A whole full-screen dispatch, generously banded — the claim is presence.
        if !(NO_DISPATCH_NS..=8.0 * VIEWT_GAP_NS).contains(&gap_ns) {
            return Err(format!(
                "`vb_viewt` is Required on this leg, so the [e6 -> b11] gap should hold one \
                 full-screen dispatch (~{VIEWT_GAP_NS} ns on this box); it measured {gap_ns:.1} ns"
            ));
        }
    } else if gap_ns >= NO_DISPATCH_NS {
        return Err(format!(
            "`vb_viewt` is Forbidden on this leg, so nothing should sit between id 6's END and \
             id 11's BEGIN; the gap measured {gap_ns:.1} ns, which is a dispatch"
        ));
    }
    Ok(())
}

/// Clause 5(3) and 5(4)'s structural half, read off the artifact's `[order]` block.
fn check_order(order: &OrderCensus) -> Result<(), String> {
    if order.frames_checked == 0 {
        return Err(
            "OrderCensus.frames_checked == 0: nothing was checked, which is not a pass. Either \
             the leg declared no chain or its run bracket never measured."
                .to_owned(),
        );
    }
    if order.violations != 0 {
        return Err(format!(
            "OrderCensus.violations == {} over {} frames checked (worst {:.1} ns): the brackets \
             do not partition the run they divide",
            order.violations, order.frames_checked, order.worst_ns
        ));
    }
    if !order.derived_inconclusive.is_empty() {
        return Err(format!(
            "derived rows {:?} fell below the 0.9 * frames_checked floor over {} frames: a \
             derived row folded from a different subset of frames than its terms is INCONCLUSIVE, \
             not merely noisy",
            order.derived_inconclusive, order.frames_checked
        ));
    }
    Ok(())
}

// ===============================================================================================
// Source pins — no GPU, and they are the only form these two claims have
// ===============================================================================================

/// The repository root, from this test binary's own manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// **`ZONE_VB_PRODUCE_NET` is never stamped.**
///
/// It takes a zone id so the artifact can carry it as a `[[zone]]` row keyed like every other, and
/// it takes nothing else: no `TsWitness` site opens a pair for it, so `pair_of[14]` stays
/// `NO_PAIR` and mask bit 14 is never set. **An artifact cannot state this** — a derived row and a
/// bracketed row are the same six numbers — so the check is over the recorder's source, which is
/// where the property lives.
#[test]
fn no_recorder_stamps_the_derived_zone() {
    let vb = repo_root().join("crates/boyko_rhi_vulkan/src/present/passes/vb.rs");
    let src = std::fs::read_to_string(&vb).expect("invariant: the VB recorder source is readable");
    assert!(
        !src.contains("ZONE_VB_PRODUCE_NET"),
        "`{}` names ZONE_VB_PRODUCE_NET. That id is DERIVED — `PRODUCE_RUN.dur - PRESHADE.dur`, \
         formed per frame in the reducer — and a recorder site for it would publish a bracket \
         under a row every consumer reads as the derived quantity.",
        vb.display()
    );
}

/// **THE WORKER'S OWN `atrous_levels: 3` — the line this file's measurement actually boots.**
///
/// This closes the DP6-0b → DP6c window. Until DP6c takes the byte pin, `PRESHADE Required`
/// asserts that the pre-shade bracket STAMPED and says nothing about what it CONTAINS: a level
/// count that quietly dropped to 1 would still stamp, still measure, and still pass every other
/// clause here while its `PRESHADE` magnitude — 78 % of the run bracket — moved underneath the
/// baseline this rung publishes.
///
/// # It reads THIS file, and the first version of it read the wrong one
///
/// [`vb_sv0_produce_run_worker`] builds its own `App` and inserts its own [`SsaoConfig`]; it does
/// not run `vb_both_ssao.rs`. A pin over that file therefore could not see an edit to the literal
/// this measurement boots — the two would have to be changed together by somebody remembering to,
/// which is precisely the coupling a pin exists to replace. **A source pin has to read the source
/// its own subject is written in**, so this one is self-referential.
#[test]
fn the_worker_boots_three_atrous_levels() {
    let f = repo_root().join("crates/boyko_app/tests/vb_sv0_produce_run_timing.rs");
    let src = std::fs::read_to_string(&f).expect("invariant: this test's own source is readable");
    // TWO occurrences: this literal, and the worker's insert. Counted rather than `contains`, so
    // deleting the worker's line while this doc keeps the words does not read as a pass.
    let n = src.matches("atrous_levels: 3").count();
    assert!(
        n >= 2,
        "`{}` carries {n} occurrence(s) of `atrous_levels: 3`; the worker's own SsaoConfig insert \
         is one of them and this pin is the other. The pre-shade stretch is ~78 % of \
         ZONE_VB_PRODUCE_RUN on this boot, so the level count is part of what the DP6-0b baseline \
         measured; changing it re-takes the baseline.",
        f.display()
    );
}

/// **`[vb_both_ssao]` keeps `atrous_levels: 3` — a DIFFERENT claim, kept for a different reason.**
///
/// This one does **not** guard this file's measurement (see
/// [`the_worker_boots_three_atrous_levels`] for the pin that does). It guards the FIXTURE that
/// DP6c turns into a byte pin: `[vb_both_ssao]` is the split boot class DP6 changes most, its
/// golden is taken at DP6c, and the level count is part of the frame that pin will bless. A drift
/// between the two files is itself worth reding on — the worker exists to measure the fixture's
/// boot class, and if the two stop agreeing about the à-trous chain they are measuring and pinning
/// two different workloads.
#[test]
fn the_pinned_split_fixture_keeps_three_atrous_levels() {
    let f = repo_root().join("crates/boyko_app/tests/vb_both_ssao.rs");
    let src = std::fs::read_to_string(&f).expect("invariant: the split fixture source is readable");
    assert!(
        src.contains("atrous_levels: 3"),
        "`{}` no longer inserts `atrous_levels: 3`. That fixture is DP6c's byte pin for the split \
         boot class, and this rung's worker is built to boot the same chain; a divergence means \
         the measurement and the pin describe two different workloads.",
        f.display()
    );
}

// ===============================================================================================
// THE WORKER
// ===============================================================================================

/// `vb_both_ssao.rs::setup`, verbatim — and copied for that file's own stated reason: the delta
/// between the two boot classes must be exactly the `SsaoConfig` insert, so a shared setup that
/// later grew a knob would move both classes at once.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let materials_row: [Option<u16>; sv0_scene::MESH_ROW_COUNT] = [
        None,
        Some(red.index() as u16),
        Some(green.index() as u16),
        Some(gold.index() as u16),
        Some(blue.index() as u16),
    ];

    sv0_scene::spawn_scene(&mut commands, sphere, &materials_row);
}

/// **THE WORKER** — one boot of the fixture named by [`ENV_FIXTURE`], with the zone recorder armed.
///
/// Skips loudly rather than hanging when the driver's env is absent: this worker arms no capture of
/// its own, so a boot with no exit condition renders forever — the worst failure a sweep can have,
/// and the one `vg_occ_split_timing`'s worker documents at length.
#[test]
#[ignore = "spawned by the drivers below; needs a real windowed GPU device"]
fn vb_sv0_produce_run_worker() {
    let Ok(fixture) = std::env::var(ENV_FIXTURE) else {
        eprintln!(
            "{WORKER}: {ENV_FIXTURE} is unset -- SKIPPED. This worker exists to be spawned by the \
             drivers in this file; booted without it there is no fixture to build."
        );
        return;
    };
    if std::env::var("BOYKO_VB_ZONE").is_err() {
        eprintln!(
            "{WORKER}: BOYKO_VB_ZONE is unset, so no exit condition is armed -- SKIPPED rather \
             than rendering forever."
        );
        return;
    }
    let split = fixture == "split";

    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(
        "boyko_engine vb sv0 produce run",
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
    ));
    app.add_startup_system(setup);
    // After `add_plugins`, so these owner overrides win — `vb_both_sdf.rs`'s post-plugins pattern.
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
    });
    if split {
        // `[vb_both_ssao]`'s ONE delta against `[vb_both_sdf]`, copied verbatim including the level
        // count. **THIS LINE is what the measurement boots** — the fixture file is not run by this
        // worker — so `the_worker_boots_three_atrous_levels` pins THIS literal, in THIS file.
        // `the_pinned_split_fixture_keeps_three_atrous_levels` separately holds the fixture that
        // DP6c will byte-pin, so a drift between the two reds on both sides.
        app.insert_resource(SsaoConfig { quality: SsaoQuality::High, atrous_levels: 3 });
    }
    {
        // DP6a: `host` is a FOURTH accepted value and is deliberately in neither pattern below —
        // it arms the boot-side `sdf_mesh_term_wanted` (read from the env at `runner`'s boot seam)
        // and leaves both REQUEST bits false. That IS measurement arm B, and this worker's own
        // arms are `on` / unset, so neither pattern needs to change for it.
        let sdf_mesh = std::env::var("BOYKO_SDF_MESH").unwrap_or_default();
        app.insert_resource(boyko_render::LightingConfig {
            vb_sdf_mesh_shadow: matches!(sdf_mesh.as_str(), "on" | "shadow"),
            vb_sdf_mesh_ao: matches!(sdf_mesh.as_str(), "on" | "ao"),
            ..boyko_render::LightingConfig::default()
        });
    }
    app.run();
}

// ===============================================================================================
// Driving the worker
// ===============================================================================================

/// Runs ONE worker and returns its artifact, or `None` when the device serves no timestamps.
fn run_worker(fixture: &str, sv0_armed: bool) -> Option<Artifact> {
    let mut path = std::env::temp_dir();
    let arm = if sv0_armed { "on" } else { "off" };
    path.push(format!("boyko_dp6_produce_run_{fixture}_{arm}.toml"));
    let _ = std::fs::remove_file(&path);
    let token = format!("dp6-0b-{fixture}-{arm}");

    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(ENV_FIXTURE, fixture)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env("BOYKO_VB_ZONE", "1")
        .env("BOYKO_PROFILE_ARTIFACT", &path)
        .env("BOYKO_PROFILE_RUN_TOKEN", &token)
        .env("BOYKO_PROFILE_WORKLOAD", format!("dp6_{fixture}_{arm}"))
        .env("BOYKO_VB_BENCH_FRAMES", BENCH_FRAMES.to_string())
        // Every knob an operator's shell might carry that would change the scene, the leg or the
        // command stream. Removed rather than assumed unset, for `base_worker_cmd`'s reason: a
        // worker whose boot depends on the ambient environment is not reproducible.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_WINDOW_FRAMES")
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_AA")
        .env_remove("BOYKO_SSAO");
    if sv0_armed {
        cmd.env("BOYKO_SDF_MESH", "on");
    } else {
        cmd.env_remove("BOYKO_SDF_MESH");
    }

    let out = cmd.output().expect("invariant: the DP6-0b worker spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    if merged.contains("no usable timestamps") {
        eprintln!("{WORKER}: this device serves no timestamps -- SKIPPED");
        return None;
    }
    assert!(
        out.status.success(),
        "the DP6-0b worker (`{fixture}`, sv0={arm}) exited {}.\n---- worker output ----\n{merged}",
        out.status
    );
    let art = Artifact::read(&path, &token).unwrap_or_else(|e| {
        panic!(
            "`{fixture}`/{arm}: the worker completed but its artifact is unusable: {e}\n\
             ---- worker output ----\n{merged}"
        )
    });
    let _ = std::fs::remove_file(&path);
    Some(art)
}

/// **The fused leg** — `[vb_both_sdf]`'s boot, SV0 armed.
#[test]
#[ignore = "live GPU measurement; the orchestrator runs it with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn dp6_0b_fused_leg_matches_its_expectation_table() {
    let Some(art) = run_worker("fused", true) else { return };
    if let Err(why) = check_expectations(&art.zones, &table_fused(true)) {
        panic!("[vb_both_sdf] (SV0 armed): {why}");
    }
    if let Err(why) = check_order(&art.order) {
        panic!("[vb_both_sdf] (SV0 armed): {why}");
    }
    // The fused leg has no id 11, so the gap is not a statement about anything here — asserted as
    // ABSENT rather than skipped silently, because "the check did not apply" and "the check passed"
    // are the two states this campaign keeps confusing.
    assert!(
        viewt_gap_ns(&art.zones).is_none(),
        "the fused leg stamped ZONE_VB_GEO, so its expectation table is wrong about the leg"
    );
    // NET ≡ PRODUCE_RUN on this row: `PRESHADE` is absent-Forbidden, so the derived value is the
    // run bracket itself. Asserted because it is what makes G-NEUTRAL and G-REDUCE read ONE
    // comparator rather than two.
    let run = row(&art.zones, ZONE_VB_PRODUCE_RUN);
    let net = row(&art.zones, ZONE_VB_PRODUCE_NET);
    assert!(
        (run.median_ns - net.median_ns).abs() < 1.0,
        "on a fused leg NET must be identical to PRODUCE_RUN (PRESHADE is structurally absent and \
         contributes 0.0); they read {} and {}",
        run.median_ns,
        net.median_ns
    );
}

/// **The split leg** — `[vb_both_ssao]`'s boot, SV0 armed.
#[test]
#[ignore = "live GPU measurement; the orchestrator runs it with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn dp6_0b_split_leg_matches_its_expectation_table() {
    let Some(art) = run_worker("split", true) else { return };
    if let Err(why) = check_expectations(&art.zones, &table_split(true)) {
        panic!("[vb_both_ssao] (SV0 armed): {why}");
    }
    if let Err(why) = check_order(&art.order) {
        panic!("[vb_both_ssao] (SV0 armed): {why}");
    }
    let gap = viewt_gap_ns(&art.zones)
        .expect("invariant: the split leg stamps both id 6 and id 11, so the gap exists");
    if let Err(why) = check_viewt_gap(gap, true) {
        panic!("[vb_both_ssao] (SV0 armed): {why}");
    }
    // The partition identity, on the published medians: `NET + PRESHADE` is `PRODUCE_RUN` only
    // approximately here, because each of the three is its own window median — the per-frame
    // identity is exact and lives in the reducer. A loose bound, stated as such: it catches a
    // derived row formed from the wrong pair of zones, not a reduction artefact.
    let run = row(&art.zones, ZONE_VB_PRODUCE_RUN);
    let pre = row(&art.zones, ZONE_VB_PRESHADE);
    let net = row(&art.zones, ZONE_VB_PRODUCE_NET);
    let residual = (net.median_ns + pre.median_ns - run.median_ns).abs();
    assert!(
        residual < 0.05 * run.median_ns,
        "NET + PRESHADE should reconstruct PRODUCE_RUN to within reduction noise; residual was \
         {residual:.1} ns on a {:.1} ns run bracket",
        run.median_ns
    );
    assert!(
        pre.median_ns > net.median_ns,
        "on `[vb_both_ssao]` the pre-shade stretch dominates the run bracket (~78 %); it read \
         {:.1} ns against a NET of {:.1} ns, which means the fixture is not the one this rung \
         baselined",
        pre.median_ns,
        net.median_ns
    );
}

/// The zone's row, or a panic naming it.
fn row(rows: &[ZoneRow], zone: u16) -> &ZoneRow {
    rows.iter()
        .find(|r| r.zone == zone)
        .unwrap_or_else(|| panic!("invariant: the artifact carries a row for zone {zone}"))
}

// ===============================================================================================
// The logic half — both directions, no GPU
// ===============================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Measured` row for `zone`, with the two offsets a gap check reads.
    fn measured(zone: u16, begin: f64, end: f64) -> ZoneRow {
        ZoneRow {
            zone,
            label: ZoneLabel::Measured,
            n: 200,
            median_ns: end - begin,
            mean_ns: end - begin,
            p95_ns: end - begin,
            stddev_ns: 0.0,
            begin_off_ns: begin,
            end_off_ns: end,
        }
    }

    /// The rows a healthy split leg produces.
    fn split_rows() -> Vec<ZoneRow> {
        vec![
            measured(ZONE_VB_RUN, 0.0, 40_000.0),
            measured(ZONE_VB_HZB_BUILD, 41_000.0, 42_000.0),
            measured(ZONE_VB_SDF_MESH, 40_500.0, 40_800.0),
            measured(ZONE_VB_GEO, 47_248.0, 60_000.0),
            measured(ZONE_VB_PRESHADE, 60_000.0, 316_000.0),
            measured(ZONE_VB_PRODUCE_RUN, 40_100.0, 430_000.0),
            measured(ZONE_VB_SHADE, 316_000.0, 420_000.0),
            measured(ZONE_VB_PRODUCE_NET, 40_100.0, 430_000.0),
        ]
    }

    /// The table passes on the rows it describes — the control, without which the two reds below
    /// would be reds about the fixture rather than about the rule.
    #[test]
    fn the_split_table_passes_on_split_rows() {
        assert!(check_expectations(&split_rows(), &table_split(true)).is_ok());
    }

    /// **RED direction 1** — a `Required` zone with no row.
    #[test]
    fn a_missing_required_zone_reds() {
        let mut rows = split_rows();
        rows.retain(|r| r.zone != ZONE_VB_PRESHADE);
        let err = check_expectations(&rows, &table_split(true))
            .expect_err("a missing Required zone must red");
        assert!(err.contains("Required"), "the message must name the direction: {err}");
    }

    /// **RED direction 2** — a `Forbidden` zone that has a row.
    ///
    /// This is red mutation (d) in miniature: declaring `ZONE_VB_GEO` `Forbidden` on a leg that
    /// stamps it must red, or the table is a description of whatever happened rather than a claim.
    #[test]
    fn a_present_forbidden_zone_reds() {
        let err = check_expectations(&split_rows(), &table_fused(true))
            .expect_err("a Forbidden zone with a row must red");
        assert!(err.contains("Forbidden"), "the message must name the direction: {err}");
    }

    /// A `Required` zone whose row is not `Measured` reds too — a bracket that opened and lost its
    /// numbers is not the same fact as a bracket that ran.
    #[test]
    fn a_required_zone_with_a_lost_row_reds() {
        let mut rows = split_rows();
        for r in &mut rows {
            if r.zone == ZONE_VB_GEO {
                r.label = ZoneLabel::Lost;
            }
        }
        let err = check_expectations(&rows, &table_split(true))
            .expect_err("a Required zone that is not Measured must red");
        assert!(err.contains("Measured"), "{err}");
    }

    /// **The two spellings are LINKED** — W5's fix, and the only thing that keeps this harness's
    /// per-leg table and the reducer's chain/derived declarations describing one instrument.
    ///
    /// The reducer decides which zones are ordered and which are differenced; this file decides
    /// which zones must be present. They are different questions over the SAME set, so the check is
    /// containment plus compatibility: every chain member and every derived-spec zone appears in its
    /// leg's table, and none of them is declared `Forbidden` there — a zone the reducer orders or
    /// differences on a leg cannot be one the harness says must not exist on it.
    #[test]
    fn zone_declarations_agree_with_the_reducer() {
        for (leg, chain, derived, table) in [
            ("fused", VB_CHAIN_FUSED, VB_DERIVED_FUSED, table_fused(true)),
            ("split", VB_CHAIN_SPLIT, VB_DERIVED_SPLIT, table_split(true)),
        ] {
            for zone in chain {
                let cell = table
                    .iter()
                    .find(|c| c.zone == *zone)
                    .unwrap_or_else(|| panic!("{leg}: chain member {zone} has no expectation cell"));
                assert_eq!(
                    cell.expect,
                    Expect::Required,
                    "{leg}: zone {zone} is a chain member, so the table cannot say it may be absent"
                );
            }
            for spec in derived {
                for (role, z) in
                    [("derived row", spec.zone), ("minuend", spec.minuend), ("subtrahend", spec.subtrahend)]
                {
                    let cell = table.iter().find(|c| c.zone == z).unwrap_or_else(|| {
                        panic!("{leg}: the {role} zone {z} has no expectation cell")
                    });
                    // The SUBTRAHEND is the one that may legitimately be `Forbidden` — that is the
                    // fused leg's whole shape — so it is checked for AGREEMENT with the spec rather
                    // than for presence.
                    if role == "subtrahend" {
                        assert_eq!(
                            cell.expect, spec.subtrahend_expect,
                            "{leg}: the table and the DerivedSpec disagree about zone {z}"
                        );
                    } else {
                        assert_eq!(
                            cell.expect,
                            Expect::Required,
                            "{leg}: the {role} zone {z} must be Required in the table"
                        );
                    }
                }
            }
        }
    }

    /// The gap check reds in BOTH directions, on arithmetic rather than on hardware.
    #[test]
    fn the_viewt_gap_reds_in_both_directions() {
        let rows = split_rows();
        let gap = viewt_gap_ns(&rows).expect("both ends are present in the fixture rows");
        // 47 248 − 42 000 = 5 248: the dispatch this box measured.
        assert!((gap - VIEWT_GAP_NS).abs() < 1.0, "the fixture's own gap is {gap}");
        assert!(check_viewt_gap(gap, true).is_ok());
        // Expected to run, and nothing is there.
        assert!(check_viewt_gap(0.0, true).is_err(), "an empty gap under Required must red");
        // Expected NOT to run, and a dispatch is there.
        assert!(check_viewt_gap(gap, false).is_err(), "a dispatch under Forbidden must red");
        assert!(check_viewt_gap(0.0, false).is_ok());
        // The boundary belongs to exactly one verdict — O4. A value that satisfied both would let
        // a leg pass whichever expectation it was given, which is a check that cannot fail.
        assert!(check_viewt_gap(NO_DISPATCH_NS, true).is_ok(), "the boundary passes Required");
        assert!(check_viewt_gap(NO_DISPATCH_NS, false).is_err(), "and fails Forbidden");
    }

    /// A fused leg's rows have no id 11, so the gap is `None` — "not applicable" rather than zero.
    #[test]
    fn the_gap_is_absent_without_the_geo_bracket() {
        let mut rows = split_rows();
        rows.retain(|r| r.zone != ZONE_VB_GEO);
        assert!(viewt_gap_ns(&rows).is_none());
    }

    /// `violations == 0` over `frames_checked == 0` is refused, which is clause 5(3)'s second half.
    #[test]
    fn a_zero_over_zero_order_census_is_not_a_pass() {
        let empty = OrderCensus::default();
        let err = check_order(&empty).expect_err("zero frames checked must red");
        assert!(err.contains("frames_checked"), "{err}");

        let good = OrderCensus { frames_checked: 200, ..OrderCensus::default() };
        assert!(check_order(&good).is_ok());

        let violated = OrderCensus {
            frames_checked: 200,
            violations: 3,
            worst_ns: 1_024.0,
            ..OrderCensus::default()
        };
        assert!(check_order(&violated).is_err());

        let inconclusive = OrderCensus {
            frames_checked: 200,
            derived_inconclusive: vec![ZONE_VB_PRODUCE_NET],
            ..OrderCensus::default()
        };
        let err = check_order(&inconclusive).expect_err("a floored derived row must red");
        assert!(err.contains("INCONCLUSIVE"), "{err}");
    }
}
