//! Asset-streaming plan F6 — REAL-DEVICE churn integration test: drives the FULL
//! fence-gated deferred-free pipeline (carrier hooks -> `apply_refcount_deltas`'s
//! enqueue -> `retire_deferred_frees`'s fence-gated drain -> `MeshAssetsExt`
//! device-buffer/BLAS teardown) against a LIVE `VulkanContext` over many presented
//! frames — the one class of F6 bug (a use-after-free / double-free of a mesh's
//! `VkBuffer`/BLAS, or a resource freed before its last submit is fence-complete)
//! that NO CPU-only unit/integration test in this rung can exercise: every other
//! F6 suite either fakes device-free `MeshGpu` values with `VkBuffer::NULL` and
//! never calls a real Vulkan function on them (`asset_streaming_f5_validation.rs`'s
//! established idiom), or drives the CPU-side enqueue/fence-gate/store-retire
//! logic with NO device at all (`asset_refcount.rs`'s churn stress, this rung's
//! sibling test) — `retire_deferred_frees` itself takes `&VulkanContext`, which
//! has no public/testable constructor outside a real device boot.
//!
//! # Scenario
//!
//! Startup registers [`INITIAL_MESHES`] distinct cube meshes, each carried by its
//! own entity (single-owner — despawning it drives that mesh's refcount to
//! exactly zero), plus a minimal sun + sky + camera (mirrors `interp_smoke.rs`'s
//! minimal setup — this test asserts asset-lifetime bookkeeping, not the
//! rendered image). A per-frame `churn_step` system then runs for
//! [`CHURN_FRAMES`] frames: each churn frame despawns the OLDEST still-alive
//! churned entity (a real `-1` reaching zero -> `Retiring` -> enqueued into
//! `DeferredFree`, fence-stamped `retire_frame = epoch + RETIRE_DELAY`) and spawns
//! a BRAND-NEW entity with a FRESHLY-REGISTERED cube mesh (a real
//! `ctx.create_buffer` pair, and — under `hwrt` on an RT device — a real BLAS
//! build). The runner's own `retire_deferred_frees` call (host plan step 4.5,
//! after `wait_frame_in_flight`) then drains + destroys whatever crossed its
//! fence horizon EVERY frame, exactly as it does in production. A [`DRAIN_FRAMES`]
//! tail (well past `RETIRE_DELAY == FRAMES_IN_FLIGHT == 2`) follows with no more
//! churn, so every straggler enqueued by the LAST churn frame has certainly
//! drained by the time this test inspects the final state.
//!
//! # Assertions (all via the PUBLIC `Assets<MeshGpu>` / `DeferredFree` /
//! `OrphanedMeshGpu` accessors — no private-field access)
//!
//! - `DeferredFree::is_empty()` — every enqueued retire fully drained by the
//!   drain tail's end (a stuck entry would mean a leaked mesh: its device
//!   buffers/BLAS never freed).
//! - `OrphanedMeshGpu::is_empty()` — no `fill()`-rejected mesh value was ever
//!   orphaned in this scenario (no `fill` caller exists in this test — a
//!   regression net, not the primary property).
//! - `Assets::install_epoch()` is EXACTLY `INITIAL_MESHES + 2 * CHURN_FRAMES` —
//!   a deterministic, timing-independent counter bumped once per `add`/`fill`
//!   (reuse included) AND once per `retire` (the terminal Retiring->Vacant
//!   transition — see `Assets::install_epoch`'s doc): `INITIAL_MESHES +
//!   CHURN_FRAMES` adds plus `CHURN_FRAMES` retires (the churn despawns; the
//!   still-alive `INITIAL_MESHES` retire only at shutdown, after the snapshot),
//!   so this holds regardless of exact fence-drain timing.
//! - `Assets::high_water()` stays FAR below `INITIAL_MESHES + CHURN_FRAMES` —
//!   proof that the free-list actually RECYCLED retired slots (a broken reuse
//!   path would grow `high_water` 1:1 with every churn spawn, exactly like the
//!   CPU-only `material_churn_over_many_epochs_...` sibling test proves for
//!   `Assets<Material>`).
//! - `Assets::free_epoch()` is EXACTLY `2 * CHURN_FRAMES` — each despawned
//!   mesh's single owner reaching zero bumps it once at the `Retiring`
//!   transition (`dec_ref`) and once more at the terminal free (`retire`); this
//!   is only exactly determinable BECAUSE `DeferredFree::is_empty()` is checked
//!   first (every enqueued retire has, by then, certainly completed both
//!   transitions).
//! - `Assets::len()` is back to EXACTLY `INITIAL_MESHES` — the steady-state
//!   population (every despawn was matched by a spawn; by the drain tail's end
//!   every retire completed), proving no phantom row survives or vanishes.
//!
//! # What this actually catches (and what it does NOT)
//!
//! The numeric assertions above are a CPU-side bookkeeping cross-check — they
//! would pass even if `retire_deferred_frees` freed a `VkBuffer`/BLAS ONE FRAME
//! TOO EARLY (a fence-gate violation) or never freed it in the first Vulkan
//! sense while still reporting correct store bookkeeping. The added value over
//! the CPU-only sibling test is that this one drives REAL `ctx.create_buffer`
//! pairs (and, under `hwrt`, REAL `vkBuildAccelerationStructuresKHR` /
//! `vkDestroyAccelerationStructureKHR`) against a LIVE device over many churn
//! frames: a genuine double-free or a free-before-last-submit that the driver
//! itself trips surfaces as a `VK_ERROR_DEVICE_LOST` / process crash / hang
//! EVEN WITHOUT validation layers, which no device-less test can reach.
//!
//! IMPORTANT — validation layers do NOT engage on this boot path. The windowed
//! runner hardcodes `InstanceConfig::enable_validation = false` (runner.rs), so
//! `EnginePlugins::window(...)` never requests `VK_LAYER_KHRONOS_validation`
//! regardless of `BOYKO_DISABLE_VALIDATION`; and on the windows-gnu (MinGW)
//! toolchain the VulkanSDK validation DLL (an MSVC build) crashes the process
//! on load anyway (see `VulkanContext::boot`'s escape-hatch doc). So the
//! VUID-level "traces a freed resource" oracle is NOT available here. The
//! load-bearing guarantee that a retired slot is never traced/drawn is instead
//! STRUCTURAL and proven at the source, not at runtime: `retire_deferred_frees`
//! is fence-gated on `submission_epoch + FRAMES_IN_FLIGHT`, and the per-frame
//! gather EXCLUDES every retired slot from the draw ring / `mesh_ids` / TLAS
//! instance set BY CONSTRUCTION (F6 C1 fix), so no freed `VkBuffer`/BLAS can be
//! referenced by a later submit in the first place (see `retire_deferred_frees`
//! and the mesh-draw scatter's docs). This test is the real-device SMOKE that
//! the whole churn pipeline runs crash-free end-to-end against those proofs.
//!
//! # Running (both feature configurations — the orchestrator, NOT a subagent)
//!
//! ```text
//! # Default (software-only mesh-shadow path):
//! cargo test -p boyko-app --test asset_streaming_f6_churn_headless -- --ignored --test-threads=1
//!
//! # hwrt (real BLAS build/destroy churn on an RT device):
//! cargo test -p boyko-app --features hwrt --test asset_streaming_f6_churn_headless -- --ignored --test-threads=1
//! ```
//!
//! `BOYKO_DISABLE_VALIDATION` may be set or unset — it makes no difference here
//! (the windowed runner requests no validation regardless; see above), and on
//! windows-gnu setting it avoids the MSVC-DLL load crash on any code path that
//! WOULD request the layer. `--test-threads=1` is required (windowed-test
//! convention: a single process-global GPU device). On a windowless / GPU-less
//! box the runner exits before the frame loop and this test SKIPs gracefully
//! (the same discrimination `interp_smoke.rs` / `room_smoke.rs` use), never
//! asserting on the "did anything even run" question — only the orchestrator,
//! with a real GPU, gets a load-bearing pass/fail here.

#![cfg(windows)]

use std::collections::VecDeque;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::OrphanedMeshGpu;
use boyko_scene::DeferredFree;

/// Frames left before the test requests exit. Decremented once per Main run —
/// mirrors every other windowed smoke test's budget idiom.
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

/// Number of distinct entities/meshes alive at steady state — small enough that
/// a single-frame churn (despawn one, spawn one) keeps the FIFO queue at a
/// constant depth throughout the run.
const INITIAL_MESHES: u32 = 5;
/// Number of frames the churn system actively despawns + spawns. Chosen well
/// above `RETIRE_DELAY` (`FRAMES_IN_FLIGHT == 2`) so the free-list reuse path is
/// exercised many times over, not just once.
const CHURN_FRAMES: u32 = 20;
/// Trailing frames with NO further churn, letting every straggling
/// `FreeEntry`/orphan drain past its fence horizon before this test inspects the
/// final state. `>> RETIRE_DELAY` (2) for margin against a pre-acquire
/// recreate-skip frame (which advances neither `frame_index` nor
/// `submission_epoch` — see `Renderer::submission_epoch`'s doc).
const DRAIN_FRAMES: u32 = 8;
const BUDGET: u32 = CHURN_FRAMES + DRAIN_FRAMES;

/// The FIFO ring of currently-alive churned entities (single-owner: each holds
/// exactly one freshly-registered mesh) + the countdown of remaining ACTIVE
/// churn frames.
#[derive(Resource)]
struct ChurnState {
    queue: VecDeque<Entity>,
    remaining: u32,
}

/// A DURING-run snapshot of the four `Assets<MeshGpu>` lifetime counters,
/// overwritten by [`snapshot_mesh_stats`] EVERY frame — the LAST frame's values
/// win (by the `DRAIN_FRAMES` tail's end every churn retire has settled into a
/// steady state, so the final capture is the terminal state).
///
/// It is a plain `Send` `Resource`, so it SURVIVES the runner's shutdown. The
/// store itself does NOT: shutdown `remove_non_send_resource::<Assets<MeshGpu>>()`s
/// it to force-drain every residual `DeferredFree` entry and destroy its device
/// buffers/BLAS BEFORE the `GpuDevice` is torn down (runner.rs shutdown, plan
/// F6) — the store is therefore GONE by the time this test inspects state after
/// `app.run()` returns (exactly as `RhiContext`/`MaterialTable`/`GpuDevice`
/// are), so its final counters MUST be captured into a surviving resource
/// while the run is still live. (`DeferredFree`/`OrphanedMeshGpu`, inspected
/// directly post-run below, are NOT removed at shutdown and so survive as-is.)
#[derive(Resource, Default, Clone, Copy)]
struct FinalMeshStats {
    install_epoch: u64,
    high_water: usize,
    free_epoch: u64,
    len: usize,
    /// Set the first time [`snapshot_mesh_stats`] runs — proves the frame loop
    /// actually executed (a windowless boot never runs `Main`, leaving this
    /// `false`), a defence-in-depth companion to the `FrameBudget` skip check.
    captured: bool,
}

/// Overwrite [`FinalMeshStats`] from the live store every `Main` frame. Runs
/// AFTER the runner's host-step `retire_deferred_frees` (frame top, before the
/// ECS schedule), so each capture reflects that frame's post-drain state; the
/// final budget frame — past the `DRAIN_FRAMES` tail, every churn retire
/// settled — leaves the terminal steady-state counters behind for the post-run
/// assertions.
fn snapshot_mesh_stats(meshes: NonSendRes<Assets<MeshGpu>>, mut stats: ResMut<FinalMeshStats>) {
    stats.install_epoch = meshes.install_epoch();
    stats.high_water = meshes.high_water();
    stats.free_epoch = meshes.free_epoch();
    stats.len = meshes.len();
    stats.captured = true;
}

/// The churn scene: `INITIAL_MESHES` cubes, each on its own entity (single
/// mesh-owner — its despawn later drives that mesh's refcount to exactly
/// zero), plus a minimal sun + sky + camera (asset-lifetime bookkeeping is
/// this test's concern, not the rendered image — mirrors `interp_smoke.rs`'s
/// minimal non-floor setup). `ChurnState` is pre-inserted (empty queue, full
/// countdown) BEFORE `app.run()` — a startup system has no `insert_resource`
/// command, so this system only POPULATES the already-present resource.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
    mut churn: ResMut<ChurnState>,
) {
    for i in 0..INITIAL_MESHES {
        let handle = meshes.cube(dev.get(), 1.0);
        let e = commands
            .spawn(MeshBundle::new(
                handle,
                Transform::from_translation(Vec3::new(i as f32, 0.5, 0.0)),
            ))
            .id();
        churn.queue.push_back(e);
    }

    const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
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

/// Per-frame churn: while `remaining > 0`, despawn the oldest still-alive
/// churned entity (a real `-1` reaching zero — its mesh's ONLY owner) and spawn
/// a fresh entity carrying a BRAND-NEW cube mesh (a real device buffer pair,
/// and — under `hwrt` on an RT device — a real BLAS). A no-op once
/// `remaining == 0` (the `DRAIN_FRAMES` tail): no more churn, only the
/// runner's own `retire_deferred_frees` call continues draining whatever is
/// still fence-pending.
fn churn_step(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
    mut churn: ResMut<ChurnState>,
) {
    if churn.remaining == 0 {
        return;
    }
    churn.remaining -= 1;

    if let Some(old) = churn.queue.pop_front() {
        commands.entity(old).despawn();
    }

    let handle = meshes.cube(dev.get(), 1.0);
    let e = commands
        .spawn(MeshBundle::new(handle, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0))))
        .id();
    churn.queue.push_back(e);
}

#[test]
#[ignore = "needs a real windowed GPU device with validation layers ON (do NOT set \
            BOYKO_DISABLE_VALIDATION — see this file's module doc); run with \
            --test-threads=1, once default-features and once --features hwrt"]
fn mesh_churn_over_many_frames_never_leaks_or_double_frees_against_a_live_device() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(ChurnState { queue: VecDeque::new(), remaining: CHURN_FRAMES });
    app.insert_resource(FinalMeshStats::default());
    app.add_systems(exit_after_budget);
    app.add_systems(churn_step);
    app.add_systems(snapshot_mesh_stats);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app F6 churn headless", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner exits
    // BEFORE the frame loop, so the budget is untouched — SKIP (see this file's
    // module doc: only the orchestrator, with a real GPU, gets a load-bearing
    // pass/fail here).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!(
            "SKIP mesh_churn_over_many_frames_never_leaks_or_double_frees_against_a_live_device: \
             windowed boot unavailable"
        );
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    // Every enqueued retire (mesh) and every fill-rejected orphan (none in this
    // scenario) must have fully drained by the DRAIN_FRAMES tail's end — a
    // stuck entry here means a leaked mesh (its device buffers/BLAS never freed).
    assert!(
        app.world().resource::<DeferredFree>().is_empty(),
        "every enqueued FreeEntry must have drained past its fence horizon by the end of the \
         DRAIN_FRAMES tail"
    );
    assert!(
        app.world().non_send_resource::<OrphanedMeshGpu>().is_empty(),
        "no fill()-rejected mesh value should ever be orphaned in this scenario"
    );

    // The store itself is GONE post-run — shutdown force-drains + removes it (see
    // `FinalMeshStats`' doc). Assert on the DURING-run snapshot: its last capture
    // (final budget frame, past the DRAIN_FRAMES tail with every churn retire
    // settled) holds the terminal steady-state counters.
    let stats = *app.world().resource::<FinalMeshStats>();
    assert!(
        stats.captured,
        "snapshot_mesh_stats must have run (the frame loop executed) — else the counters below \
         are meaningless Default zeros"
    );

    // install_epoch bumps once per `add`/`fill` (reuse included) AND once per
    // `retire` (the terminal Retiring->Vacant transition — FIX-C1: so the hwrt
    // `blas_addr` table resyncs a freed slot's stale device address to the Vacant
    // sentinel even before any reuse; see `Assets::install_epoch`'s doc). So:
    // (INITIAL_MESHES + CHURN_FRAMES) adds — 5 startup + 20 churn `cube()` calls,
    // each a FRESH procedural mesh (no path dedup) — plus CHURN_FRAMES retires:
    // the 20 despawned meshes, all fully retired within the DRAIN_FRAMES tail
    // BEFORE the final snapshot (the still-alive INITIAL_MESHES retire only at
    // shutdown, after it). Deterministic, fence-timing-independent.
    assert_eq!(
        stats.install_epoch,
        u64::from(INITIAL_MESHES) + 2 * u64::from(CHURN_FRAMES),
        "install_epoch must count one bump per add (startup+churn) AND one per retire (churn despawns)"
    );

    // high_water stays far below the naive (no-reuse) total — proof the
    // free-list actually recycled retired slots against a LIVE device.
    let naive_total = (INITIAL_MESHES + CHURN_FRAMES) as usize;
    assert!(
        stats.high_water < naive_total,
        "high_water ({}) must stay below the naive total ({naive_total}) — free-list reuse must \
         have recycled retired slots, not appended a fresh row for every churn spawn",
        stats.high_water
    );

    // free_epoch is exactly 2 per fully-completed retire (one at the Retiring
    // transition, one at the terminal free) — only exactly determinable because
    // DeferredFree::is_empty() (above) proves every enqueued retire completed.
    // Captured DURING the run (last frame), so the still-alive INITIAL_MESHES —
    // retired only at shutdown, after the final snapshot — do NOT inflate it.
    assert_eq!(
        stats.free_epoch,
        2 * u64::from(CHURN_FRAMES),
        "free_epoch must count exactly two bumps per fully-retired mesh (Retiring + terminal free)"
    );

    // The steady-state population: every despawn was matched by a spawn, and by
    // the final snapshot every churn retire has completed — exactly the starting
    // count still alive (they are torn down later, at shutdown).
    assert_eq!(
        stats.len,
        INITIAL_MESHES as usize,
        "the live mesh count must return to exactly its starting population"
    );
}
