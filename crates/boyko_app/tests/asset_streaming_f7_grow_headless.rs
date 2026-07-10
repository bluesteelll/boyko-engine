//! Asset-streaming plan F7 — REAL-DEVICE grow-and-defer-old integration test: drives
//! `MaterialTable::grow_if_needed` + `GBufferFrame::repoint_material_table` +
//! `RetiredGpuBuffers`'s fence-gated drain (and, non-RT, `GpuSceneBundles::
//! grow_instance_family_if_needed`) against a LIVE `VulkanContext` over many
//! presented frames — the one class of F7 bug (a UAF on a descriptor set still
//! bound to a freed material SSBO, a leaked superseded buffer, or a device-lost from
//! a `vkUpdateDescriptorSets` on a still-pending set) that NO CPU-only unit test in
//! this rung can exercise.
//!
//! # ONE `#[test]` per file (structural constraint, read before adding a second one)
//!
//! `App::new()` + `add_plugins(EnginePlugins)` registers PROCESS-GLOBAL component
//! hooks (`register_component_hooks::<T>` — asset-refcount carriers, light hooks,
//! …). Those hooks may be registered exactly once per type per process; a SECOND
//! `App`/`add_plugins` call in the SAME process (which is what running multiple
//! `#[test]` fns under `--test-threads=1` in one binary does) panics the instant a
//! type that already appears in a live archetype is re-registered. This is exactly
//! why every OTHER windowed smoke in this workspace (`interp_smoke.rs`,
//! `room_smoke.rs`, `asset_streaming_f6_churn_headless.rs`) has EXACTLY ONE
//! `#[test]` per file — and why this file, after an earlier revision briefly grew to
//! 4-6 `#[test]` fns and only the FIRST ever passed on a real device, was
//! restructured back down to one. The F7 scenarios (§12: grow-past-boot material,
//! grow-past-boot instance, FIX-F multi-grow, FIX-E rebind-under-FIF) are therefore
//! sequential PHASES within a SINGLE `App`/`app.run()`, not separate tests. The one
//! exception — `w3_rt_cap`, which must PANIC — cannot share this App (its panic
//! would abort the phased run mid-way) and lives in its own file,
//! `asset_streaming_f7_rt_cap_headless.rs`, with its own single `#[test]`.
//!
//! # What the boot material-table capacity actually is (corrects an earlier draft)
//!
//! `MaterialTable::boot_seed` sizes the device table to `Assets::<MaterialGpu>::
//! high_water()` AT `boot_seed` TIME (`runner.rs`: `app.finish()` — which drains
//! every `add_startup_system` — runs BEFORE `boot_seed`). `Assets::<MaterialGpu>::
//! with_reserved(256)` (`runner.rs`'s `MATERIAL_CAPACITY`) is only the Vec's
//! preallocated STORAGE reservation (avoids reallocation up to 256 rows) — it is
//! NOT the device table's row capacity. This test's `setup_minimal_scene` startup
//! system mints no material of its own (only a mesh + minimal lights/camera), so by
//! `boot_seed` time `Assets::<MaterialGpu>::high_water() == 1` (only the runner's own
//! pinned default material, minted directly in `run_windowed` before `finish()`).
//! The device table's boot ROW capacity is therefore exactly **1**, not 256 — Phase
//! A below (a steady 1-material/frame post-boot mint) crosses the
//! `next_power_of_two()` ladder (1->2->4->8->16->…) almost immediately, including
//! TWO grows on the very first two mints landing on IMMEDIATELY ADJACENT frames
//! (`need=2` grows to capacity 2, `need=3` — the VERY NEXT mint — already exceeds
//! that and grows to 4): FIX-F's "two grows inside one FIF window" is therefore
//! exercised by Phase A itself, with no separate artificial batch-jump phase needed.
//!
//! # Public-API-only constraint
//!
//! `GpuSceneBundles`, `WindowHost`, and `INSTANCE_CAPACITY` are ALL `pub(crate)`
//! inside `boyko_app` — this integration test compiles as a SEPARATE crate and
//! cannot name them, read `instance_capacity[s]`, or call
//! `grow_instance_family_if_needed`/`needs_instance_grow` directly. Phase B's
//! instance-family assertion is therefore an EXECUTION SMOKE (does a >
//! `INSTANCE_CAPACITY`-drawable scene render without a panic/device-lost?), not a
//! numeric capacity check. `MaterialTable`/`RetiredGpuBuffers` ARE public
//! (`boyko_render::{MaterialTable, RetiredGpuBuffers}`), so the material-side
//! assertions DO cross-check real state (`table().buffer` identity,
//! `rebind_pending`, `is_empty()`).
//!
//! # Phase B is runtime-gated to non-RT devices (W3)
//!
//! `GpuSceneBundles::grow_instance_family_if_needed`'s own gate is
//! `self.tlas.is_some() || self.mv.is_some()` (design §7.3 step 1) — RT instance
//! growth is OUT OF SCOPE (E1 Option B). On an RT-capable device built with
//! `--features hwrt`, `tlas.is_some()` — so Phase B's > `INSTANCE_CAPACITY` spawn
//! would trip the LIVE hard `assert!` in `upload_instance_models` instead of
//! growing, aborting this phased run before Phases A/D's assertions are ever
//! checked. `instance_grow_out_of_scope` (below) reproduces the SAME condition
//! `ctx.ray_query_enabled()` computes (`RayCaps.tier`, the world-visible mirror of
//! `DeviceCaps::rt_tier()` `runner.rs` inserts UNCONDITIONALLY at boot, ADDITIONALLY
//! gated on the `hwrt` Cargo feature — see that fn's doc for why `RayCaps.tier`
//! alone is not sufficient), so Phase B runs ONLY when instance growth is actually
//! in scope. `FinalGrowStats`'s material-side assertions (`grow_transitions >= 2`,
//! `rebind_pending` both `false`, `retired_is_empty`) hold on BOTH legs regardless;
//! Phase B's own assertion is conditioned on whether it ran. The RT hard cap itself
//! is verified separately, by `asset_streaming_f7_rt_cap_headless.rs`.
//!
//! # What "a grow happened" / "how many" means here (no private grow counter exists)
//!
//! Neither `MaterialTable` nor `RetiredGpuBuffers` expose a "how many times did you
//! grow / destroy" counter (by design). A grow is observed STRUCTURALLY:
//! `MaterialTable::table().buffer` (a `VkBuffer`, `PartialEq`) changes identity
//! exactly when `grow_if_needed` swaps in a new device buffer — `FinalGrowStats`
//! samples it EVERY frame and counts TRANSITIONS (`grow_transitions`), giving an
//! exact count of distinct grow events over the run (at most one grow runs per
//! frame, so no transition is ever missed by per-frame sampling). "No leak" is
//! `RetiredGpuBuffers::is_empty()` after the Phase D idle tail (well past
//! `RETIRE_DELAY == FRAMES_IN_FLIGHT`) — every superseded buffer this run pushed
//! must have drained by then.
//!
//! # C1 hwrt-completeness fold-in
//!
//! The completeness guarantee itself (every material-bearing descriptor-set ring is
//! walked by `repoint_material_table`) is proven STATICALLY and exhaustively by the
//! CPU-only `material_set_rings_count_matches_expected_across_every_hwrt_arming_
//! combination` unit test in `boyko_rhi_vulkan::present::targets`, backstopped by the
//! `debug_assert` in `sync_gbuffer`. Under `--features hwrt` this SAME phased run
//! additionally arms `ShadowDenoiseConfig::Both` (every `Option`-guarded HWRT resolve
//! ring `material_set_rings` enumerates) BEFORE `app.run()` — one `cfg`-gated line,
//! no second `App`/`#[test]` — so the file stays at EXACTLY ONE `#[test]` in BOTH
//! configs. The SAME final assertions (both FIF slots converged, no leak) then also
//! certify the hwrt path survived the grow+repoint cycle with every resolve variant
//! armed.
//!
//! # Asset-streaming plan F8 fold-in — the PER_INSTANCE_MATERIAL ring's own lockstep grow
//!
//! Phase B's batch (below) now carries a SHARED non-default material (previously every
//! Phase-B drawable used the implicit default `MaterialHandle(0)`), so its
//! `> INSTANCE_CAPACITY` spawn also exercises F8's `pm_instance_material_rings[s]` /
//! `pm_bind_groups[s]` growth — `GpuSceneBundles`'s F8 fields are `pub(crate)`/private
//! (same public-API-only constraint as `INSTANCE_CAPACITY` above), so this is, like Phase
//! B's own instance-ring claim, an EXECUTION SMOKE: `upload_instance_materials` (`upload.rs`)
//! hard-asserts `bytes.len() <= ring_slot.size` — if `pm_instance_material_rings[s]` had NOT
//! grown in lockstep with `instance_rings[s]` (F8 §7b/i), this phased run would abort with
//! that assert instead of reaching the final checks below. `FinalPmStats` (a plain `Send`
//! snapshot, mirroring `FinalGrowStats`'s idiom) additionally cross-checks, via the PUBLIC
//! `MeshRenderScratch` surface, that the material lane actually scaled past
//! `INSTANCE_CAPACITY` and that the PM pipeline-selection flag stayed armed — so a silently
//! SKIPPED material upload (e.g. a gating bug that never calls `upload_instance_materials`
//! at all) would also be caught, not just a hard-cap overflow.
//!
//! # Running (the orchestrator, NOT a subagent)
//!
//! ```text
//! cargo test -p boyko-app --test asset_streaming_f7_grow_headless -- --ignored --test-threads=1
//! cargo test -p boyko-app --features hwrt --test asset_streaming_f7_grow_headless -- --ignored --test-threads=1
//! ```
//!
//! `BOYKO_DISABLE_VALIDATION` may be set or unset (the windowed runner requests no
//! validation regardless — see `asset_streaming_f6_churn_headless.rs`'s doc);
//! `--test-threads=1` is required (single process-global GPU device).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{MaterialGpu, MaterialTable, MeshRenderScratch, RayCaps, RetiredGpuBuffers, RtTier};
#[cfg(feature = "hwrt")]
use boyko_render::{ShadowDenoiseConfig, ShadowDenoiseMode};

/// `true` iff RT instance growth is OUT OF SCOPE on this run (W3 — the coordinator's
/// finding): `GpuSceneBundles::grow_instance_family_if_needed`'s own gate is
/// `self.tlas.is_some() || self.mv.is_some()`, and `tlas`/`mv` are built iff
/// `ctx.ray_query_enabled()` — which is `#[cfg(feature = "hwrt")] {
/// self.device_caps().ray_query } #[cfg(not(feature = "hwrt"))] { false }`
/// (`boyko_rhi_vulkan::device`). `RayCaps.tier` mirrors the RAW device probe
/// (`DeviceCaps::rt_tier()`, inserted UNCONDITIONALLY at boot regardless of the
/// `hwrt` Cargo feature — `runner.rs`), so on a NON-hwrt build the raw probe can
/// still read `Weak`/`Strong` on RT-capable hardware even though `tlas` never
/// exists there — checking `RayCaps.tier` ALONE would be wrong. This fn reproduces
/// `ray_query_enabled()`'s EXACT compile-time-AND-runtime combination instead of
/// approximating it.
fn instance_grow_out_of_scope(caps: &RayCaps) -> bool {
    cfg!(feature = "hwrt") && caps.tier != RtTier::Absent
}

/// Frames left before the test requests exit. Decremented once per `Main` run —
/// mirrors `asset_streaming_f6_churn_headless.rs`'s budget idiom.
#[derive(Resource)]
struct FrameBudget(u32);

fn exit_after_budget(mut budget: ResMut<FrameBudget>, mut exit: ResMut<AppExit>) {
    if budget.0 > 0 {
        budget.0 -= 1;
        if budget.0 == 0 {
            exit.0 = true;
        }
    }
}

/// Ascending per-frame counter (separate from `FrameBudget`'s countdown) driving
/// which phase this frame belongs to.
#[derive(Resource, Default)]
struct FrameCounter(u32);

/// Phase A: a steady 1-material/frame post-boot mint, run for this many frames.
/// See this file's module doc for why — starting from a boot capacity of 1 — this
/// alone crosses many `next_power_of_two()` boundaries (including two on
/// immediately adjacent frames, FIX-F) and comfortably exercises "grow-past-boot
/// (material)" (§12) with wide margin.
const PHASE_A_MINT_FRAMES: u32 = 800;

/// Phase B fires exactly once, on the frame right after Phase A ends: a one-shot
/// batch of drawables well past the boot instance-family capacity (`INSTANCE_CAPACITY
/// == 1024`, `boyko_app::gpu_scene`'s `pub(crate)` constant — not nameable here, see
/// this file's module doc).
const PHASE_B_INSTANCE_SPAWN_AT_FRAME: u32 = PHASE_A_MINT_FRAMES;
const PHASE_B_INSTANCE_DRAWABLES: u32 = 1024 + 200;

/// Phase D: a quiet tail with ZERO further material/instance mutation, run for this
/// many frames after Phase B — `>> FRAMES_IN_FLIGHT` (2), proving FIX-E
/// (rebind-under-FIF: the fenced slot converges even on a non-dirty stream) and
/// giving `RetiredGpuBuffers` ample margin to drain every superseded buffer past its
/// `RETIRE_DELAY` horizon.
const PHASE_D_IDLE_FRAMES: u32 = 16;

const BUDGET: u32 = PHASE_A_MINT_FRAMES + 1 + PHASE_D_IDLE_FRAMES;

/// The single shared mesh handle every drawable in this test's scenes reuses (only
/// the MATERIAL/instance-COUNT side is meant to grow — the mesh side is untouched).
/// `Option`-wrapped + `Default`-derived so it can be `app.insert_resource`d BEFORE
/// `app.run()` (a startup system has no `insert_resource` command — only
/// `ResMut<T>` on an ALREADY-present resource, the same constraint
/// `asset_streaming_f6_churn_headless.rs`'s `ChurnState` doc documents); the startup
/// system below only POPULATES it once the device + mesh asset table exist.
#[derive(Resource, Default, Clone, Copy)]
struct SharedCubeMesh(Option<MeshHandle>);

impl SharedCubeMesh {
    fn get(self) -> MeshHandle {
        self.0.expect("invariant: setup_minimal_scene populates this before any reader runs")
    }
}

/// `true` iff Phase B (the > `INSTANCE_CAPACITY` drawable spawn) actually ran this
/// run. Plain `Send` `Resource` (like `FinalGrowStats`), so it survives the
/// runner's shutdown for the post-run assertions. `false` on an RT device (Phase B
/// is skipped there — see `instance_grow_out_of_scope`'s doc; the RT hard cap is
/// covered separately by `asset_streaming_f7_rt_cap_headless.rs`).
#[derive(Resource, Default, Clone, Copy)]
struct InstanceGrowRan(bool);

/// A DURING-run snapshot of the observable F7 state, overwritten every frame — the
/// LAST frame's values win. Plain `Send`, so it SURVIVES the runner's shutdown
/// (`MaterialTable`/`RetiredGpuBuffers` do NOT — both are removed + force-destroyed
/// at shutdown, exactly like `MeshAssetsExt`'s NonSend store — see
/// `asset_streaming_f6_churn_headless.rs`'s `FinalMeshStats` doc for the identical
/// reasoning).
#[derive(Resource, Default, Clone, Copy)]
struct FinalGrowStats {
    last_table_buffer_bits: u64,
    first_table_buffer_bits: u64,
    /// Incremented in `snapshot_grow_stats` every time THIS frame's
    /// `table().buffer` bits differ from the PRIOR frame's — an exact count of
    /// distinct grow events (at most one grow runs per frame, so per-frame
    /// sampling never misses one).
    grow_transitions: u32,
    /// `RetiredGpuBuffers::is_empty()` as of the LAST frame — the "no leak by the
    /// Phase D tail's end" signal.
    retired_is_empty: bool,
    rebind_pending_slot0: bool,
    rebind_pending_slot1: bool,
    captured: bool,
}

fn snapshot_grow_stats(
    material_table: NonSendRes<MaterialTable>,
    retired: NonSendRes<RetiredGpuBuffers>,
    mut stats: ResMut<FinalGrowStats>,
) {
    let bits = material_table.table().buffer.0;
    if !stats.captured {
        stats.first_table_buffer_bits = bits;
    } else if bits != stats.last_table_buffer_bits {
        stats.grow_transitions += 1;
    }
    stats.last_table_buffer_bits = bits;
    stats.retired_is_empty = retired.is_empty();
    stats.rebind_pending_slot0 = material_table.rebind_pending(0);
    stats.rebind_pending_slot1 = material_table.rebind_pending(1);
    stats.captured = true;
}

/// Asset-streaming plan F8 fold-in — a DURING-run snapshot of the material-gather PM
/// state, overwritten every frame (the LAST frame's values win) — mirrors
/// `FinalGrowStats`'s snapshot idiom. Plain `Send`, so it SURVIVES the runner's shutdown
/// (`MeshRenderScratch` itself is a plain `Send` resource the render plugin owns for the
/// whole run — unlike `MaterialTable`/`RetiredGpuBuffers`, it is never force-destroyed at
/// shutdown, so `app.world().resource::<MeshRenderScratch>()` would also work post-run;
/// this snapshot exists so the LAST in-run frame's values are captured even if the runner
/// clears/rebuilds render-only state during shutdown).
#[derive(Resource, Default, Clone, Copy)]
struct FinalPmStats {
    any_non_default_material_last: bool,
    /// The highest `MeshRenderScratch::material_ids` length observed across the whole
    /// run. Phase B's >`INSTANCE_CAPACITY` batch is a ONE-SHOT spawn (nothing despawns
    /// it), so later frames' length stays at the post-Phase-B total — sampling the MAX
    /// is robust regardless of exactly which frame this system runs relative to the
    /// gather within a frame.
    max_material_ids_len: usize,
}

fn snapshot_pm_stats(scratch: Res<MeshRenderScratch>, mut stats: ResMut<FinalPmStats>) {
    stats.any_non_default_material_last = scratch.any_non_default_material();
    stats.max_material_ids_len = stats.max_material_ids_len.max(scratch.material_ids.len());
}

/// The phase driver: ONE system, gated on `FrameCounter`, that runs Phase A (steady
/// material mint), then Phase B (one-shot instance-heavy spawn — SKIPPED on an RT
/// device, see `instance_grow_out_of_scope`), then goes quiet for Phase D. See this
/// file's module doc for why a single steady mint stream suffices for both
/// "grow-past-boot (material)" and FIX-F.
fn phase_driver(
    mut commands: Commands,
    mut materials: ResMut<Assets<MaterialGpu>>,
    mut frame: ResMut<FrameCounter>,
    mut instance_grow_ran: ResMut<InstanceGrowRan>,
    caps: Res<RayCaps>,
    cube: Res<SharedCubeMesh>,
) {
    let f = frame.0;
    frame.0 += 1;

    if f < PHASE_A_MINT_FRAMES {
        // Phase A: one fresh material per frame, each carried by its own freshly
        // spawned drawable (so the mesh-draw gather + the material-bearing resolve
        // sets are both live while the grow happens, not just an inert
        // `Assets<MaterialGpu>` row).
        let handle = materials.add(MaterialGpu::default());
        commands.spawn(MeshBundle {
            material: MaterialHandle(handle.index() as u16),
            ..MeshBundle::new(cube.get(), Transform::default())
        });
        return;
    }

    if f == PHASE_B_INSTANCE_SPAWN_AT_FRAME {
        // Phase B: one-shot large batch — the non-RT instance-family grow
        // execution smoke (§12 grow-past-boot, instance). SKIPPED on an RT device
        // (W3 — `grow_instance_family_if_needed` is a no-op there BY DESIGN;
        // spawning past `INSTANCE_CAPACITY` there instead trips the LIVE hard
        // `assert!` in `upload_instance_models`, which `asset_streaming_f7_rt_cap_
        // headless.rs` verifies separately). Running it here anyway on an RT
        // device would abort THIS phased run before Phases A/D's assertions below
        // ever get checked.
        if !instance_grow_out_of_scope(&caps) {
            // Asset-streaming plan F8 fold-in: every Phase-B drawable carries a SHARED
            // non-default material (not just Phase A's fresh-material-per-drawable side
            // effect — Phase A's own drawables already carry non-zero-indexed handles) —
            // see this file's module doc for why this batch is ALSO the F8
            // pm_instance_material_rings lockstep-growth execution smoke.
            let pm_material = materials.add(MaterialGpu::new(
                [0.9, 0.1, 0.1, 1.0],
                1.0,
                0.3,
                0.5,
                [0.0, 0.0, 0.0],
                0,
            ));
            for i in 0..PHASE_B_INSTANCE_DRAWABLES {
                commands.spawn(MeshBundle {
                    material: MaterialHandle(pm_material.index() as u16),
                    ..MeshBundle::new(
                        cube.get(),
                        Transform::from_translation(Vec3::new((i % 64) as f32, (i / 64) as f32, 0.0)),
                    )
                });
            }
            instance_grow_ran.0 = true;
        }
    }

    // Phase D (every remaining frame): quiet — no further material/instance
    // mutation at all, letting the fenced-slot rebind converge (FIX-E) and
    // RetiredGpuBuffers drain past its RETIRE_DELAY horizon.
}

fn setup_minimal_scene(
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
    mut commands: Commands,
    mut shared: ResMut<SharedCubeMesh>,
) {
    let cube = meshes.cube(dev.get(), 1.0);
    shared.0 = Some(cube);

    const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));

    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 320.0 / 240.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// F7 §12, phased: Phase A (grow-past-boot material + FIX-F multi-grow, folded —
/// see module doc), Phase B (grow-past-boot instance, non-RT execution smoke),
/// Phase D (FIX-E rebind-under-FIF + the final no-leak drain tail). Under
/// `--features hwrt`, the SAME run additionally arms `ShadowDenoiseConfig::Both`
/// (C1 hwrt completeness) before `app.run()` — one `cfg`-gated line, no second App.
#[test]
#[ignore = "needs a real windowed GPU device (do NOT set BOYKO_DISABLE_VALIDATION \
            requirements beyond this file's doc); run with --test-threads=1, once \
            default-features and once --features hwrt"]
fn f7_grow_and_defer_old_phased_headless() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(FrameCounter::default());
    app.insert_resource(FinalGrowStats::default());
    app.insert_resource(FinalPmStats::default());
    app.insert_resource(InstanceGrowRan::default());
    app.insert_resource(SharedCubeMesh::default());
    app.add_systems(exit_after_budget);
    app.add_systems(phase_driver);
    app.add_systems(snapshot_grow_stats);
    app.add_systems(snapshot_pm_stats);
    app.add_startup_system(setup_minimal_scene);
    app.add_plugins(EnginePlugins::window("boyko_app F7 grow-and-defer-old headless", 320, 240));
    // C1 hwrt completeness fold-in (see module doc): arms EVERY Option-guarded HWRT
    // resolve ring `material_set_rings` enumerates, so the SAME phased run below
    // also proves the C1 repoint survives with every variant armed. Inserted AFTER
    // `add_plugins` so it overrides `ShadowDenoisePlugin`'s default (`None`, the
    // 0%-gate) — mirrors that plugin's own doc contract. A no-op line on the
    // default build (the import itself is `cfg`-gated out).
    #[cfg(feature = "hwrt")]
    app.insert_resource(ShadowDenoiseConfig { mode: ShadowDenoiseMode::Both, ..Default::default() });

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits
    // BEFORE the frame loop, so the budget is untouched — SKIP (mirrors
    // `asset_streaming_f6_churn_headless.rs`'s idiom).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP f7_grow_and_defer_old_phased_headless: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    let stats = *app.world().resource::<FinalGrowStats>();
    assert!(stats.captured, "snapshot_grow_stats must have run at least once");

    // Phase A: grow-past-boot (material) — the table's buffer identity must have
    // changed (a grow happened), and MORE THAN ONE grow must have been observed
    // (proving FIX-F's "multiple grows across the run" holds against the real
    // device, not just a single boundary crossing — see module doc for why the
    // boot capacity of 1 guarantees this).
    assert_ne!(
        stats.first_table_buffer_bits, stats.last_table_buffer_bits,
        "Phase A's steady material mint must have triggered at least one \
         MaterialTable::grow_if_needed (the table's device buffer identity must have \
         changed)"
    );
    assert!(
        stats.grow_transitions >= 2,
        "Phase A's steady 1-material/frame mint (starting from a boot capacity of 1) \
         must cross MULTIPLE next_power_of_two() boundaries — observed {} distinct grow \
         transitions, expected >= 2 (FIX-F: at least one pair of these lands on \
         immediately adjacent frames, per this file's module doc)",
        stats.grow_transitions
    );

    // Phase D: FIX-E (rebind-under-FIF) — a fully quiet tail must still converge
    // both FIF slots (the repoint is gated ONLY on rebind_pending, never on
    // dirty_gen/flush_if_dirty).
    assert!(
        !stats.rebind_pending_slot0 && !stats.rebind_pending_slot1,
        "both FIF slots must have converged by the end of the quiet Phase D tail \
         (FIX-E/W1): the fenced slot's descriptor sets are repointed to the CURRENT \
         table before every record, so neither slot should still be flagged pending"
    );

    // No leak: every superseded buffer (from every grow across the whole run,
    // including Phase A's multiple grows) must have drained past its RETIRE_DELAY
    // horizon by the Phase D tail's end.
    assert!(
        stats.retired_is_empty,
        "every superseded material buffer (across ALL {} observed grows) must have \
         drained past its RETIRE_DELAY horizon by the Phase D idle tail's end — a \
         leaked entry here means RetiredGpuBuffers::drain_ready missed one",
        stats.grow_transitions
    );

    // Phase B: grow-past-boot (instance, non-RT) — EXECUTION SMOKE ONLY (see module
    // doc). Conditional on the runtime device tier (`instance_grow_out_of_scope`,
    // W3): on a NON-RT device this is LOAD-BEARING (Phase B must have run — a
    // silently-skipped Phase B here would be a gating bug, not a legitimate RT
    // skip); on an RT device Phase B is correctly skipped (growth is out of scope
    // there BY DESIGN — the RT hard cap is verified separately by
    // `asset_streaming_f7_rt_cap_headless.rs`). Reaching this assertion at all
    // (rather than a panic) already proves whichever path ran did so without a
    // device-lost.
    let instance_grow_ran = app.world().resource::<InstanceGrowRan>().0;
    let device_is_rt = instance_grow_out_of_scope(app.world().resource::<RayCaps>());
    assert_eq!(
        instance_grow_ran, !device_is_rt,
        "Phase B (the > INSTANCE_CAPACITY drawable spawn) must run iff instance growth \
         is in scope (non-RT device): instance_grow_ran={instance_grow_ran}, \
         device_is_rt={device_is_rt}"
    );

    // Asset-streaming plan F8 fold-in: on a non-RT device, Phase B's shared non-default
    // material must have kept the PM pipeline-selection flag armed AND the material lane
    // must have scaled past INSTANCE_CAPACITY in lockstep with the instance ring — reaching
    // this assertion at all (rather than a panic inside `upload_instance_materials`) already
    // proves the lockstep grow held (see this file's module doc); these two checks
    // additionally rule out a SILENTLY SKIPPED material upload (a gating bug that never
    // calls `upload_instance_materials`, which would leave the flag/lane looking inert
    // instead of aborting).
    if !device_is_rt {
        let pm_stats = *app.world().resource::<FinalPmStats>();
        assert!(
            pm_stats.any_non_default_material_last,
            "F8: any_non_default_material() must still read true at the end of the run — \
             Phase B's shared non-default material (plus Phase A's own \
             fresh-material-per-drawable drawables, all still live) must keep the PM gate armed"
        );
        assert!(
            pm_stats.max_material_ids_len >= PHASE_B_INSTANCE_DRAWABLES as usize,
            "F8: MeshRenderScratch::material_ids must have scaled to at least the \
             post-Phase-B drawable count (observed max {}, expected >= {}) — a material \
             lane that failed to grow past INSTANCE_CAPACITY in lockstep with the instance \
             ring would have aborted this run via upload_instance_materials's hard assert \
             before ever reaching here",
            pm_stats.max_material_ids_len, PHASE_B_INSTANCE_DRAWABLES
        );
    }
}
