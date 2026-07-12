//! Textured-PBR rung T6b — REAL-DEVICE integration test: registers a texture
//! into `Assets<TextureGpu>` via the boot upload drain
//! ([`upload_texture_assets`](boyko_render::upload_texture_assets)), confirms it
//! receives a real bindless slot, then drives the O1 fence-gated bindless-slot
//! recycle (`BindlessTextureTable::unregister` -> the runner's own per-frame
//! `retire_deferred_frees` -> `BindlessTextureTable::retire_ready_slots`) across
//! real presented frames against a LIVE `VulkanContext`, and finally exercises
//! the openQ2 shutdown teardown order (`Assets<TextureGpu>::destroy` BEFORE
//! `BindlessTextureTable::destroy`) via the runner's own shutdown path.
//!
//! # Why the bindless-slot recycle, not an `Assets<TextureGpu>` refcount retire
//!
//! T6b wires the O1 bindless-slot lifetime (`BindlessTextureTable::unregister` /
//! `retire_ready_slots`) and the `OrphanedTextureGpu` fill-reject drain into
//! `retire_deferred_frees`, but NOT an `Assets<TextureGpu>` refcount-driven
//! retire branch: that needs a `TextureHandle` carrier component and a
//! `boyko_scene::AssetRefKind::Texture` producer feeding `DeferredFree`, neither
//! of which exists yet (no material references a texture until T7 — see
//! `asset_refcount.rs`'s `retire_deferred_frees` doc). This test therefore
//! drives the slot's retirement directly through `BindlessTextureTable::unregister`
//! (the SAME low-level call `Assets<TextureGpu>::destroy` /
//! `OrphanedTextureGpu::drain_ready` make internally) rather than through a
//! refcount-driven despawn — the genuinely wired-and-testable T6b surface.
//!
//! # Scenario
//!
//! Startup reserves one `Handle<TextureGpu>` and pushes a tiny (2x2) decoded
//! `TextureData` into `AssetStaging<TextureGpu>` (the SAME reserve+stage path
//! `AssetServer::load` would drive, minus the PNG decode step), plus a minimal
//! sun + sky + camera (mirrors `asset_streaming_f6_churn_headless.rs`'s minimal
//! setup — this test asserts asset-lifetime bookkeeping, not the rendered
//! image). The runner's own boot-time `upload_texture_assets` drain (host plan
//! step, `run_windowed`) then fills the reserved row with a real, bindless-registered
//! `TextureGpu` BEFORE the frame loop starts.
//!
//! A per-frame `drive_lifecycle` system: on its first invocation, resolves the
//! handle, asserts a real (non-error, `>= 1`) bindless slot was issued, and
//! stages that slot for retirement via `BindlessTextureTable::unregister(slot,
//! epoch + RETIRE_DELAY)` — modeling the decision a future T7 carrier-driven
//! retire path would make. On every subsequent frame it asserts the PER-FRAME
//! invariant `BindlessTextureTable::is_empty() implies epoch >= retire_frame`
//! (the slot must never appear recycled before its fence horizon) and records
//! the first epoch at which it DOES recycle.
//!
//! # Assertions
//!
//! - The frame loop actually ran (`TestState::ran`) — a windowless / GPU-less
//!   box is SKIPped, mirroring every other windowed lifecycle test's discrimination.
//! - The resolved `bindless_slot` is `>= 1` (never the reserved error slot 0).
//! - `BindlessTextureTable::is_empty()` was NEVER observed `true` before
//!   `epoch >= retire_frame` (asserted PER-FRAME inside `drive_lifecycle` — a
//!   violation panics the test run at the exact frame it happens, not just at
//!   the end).
//! - The slot DID eventually recycle (`recycled_at_epoch.is_some()`) within the
//!   run's budget — proving `retire_deferred_frees` actually threads `epoch`
//!   into `BindlessTextureTable::retire_ready_slots` against a real device, not
//!   just that recycling never happens too early.
//! - `app.run()` returns `AppExit(true)` with no panic/hang — the runner's
//!   shutdown ran the openQ2 order (`Assets<TextureGpu>::destroy` before
//!   `BindlessTextureTable::destroy`, both under the SAME step-1 device-idle
//!   wait every other F6/F7/T6b teardown call relies on) without a crash.
//!
//! IMPORTANT — validation layers do NOT engage on this boot path (see
//! `asset_streaming_f6_churn_headless.rs`'s module doc for the full argument:
//! `enable_validation: false` hardcoded in `run_windowed`, and the windows-gnu
//! MSVC validation DLL crashes on load regardless). A genuine device-UAF here
//! (e.g. a slot descriptor-write racing an in-flight read, or a double-free of
//! a `VulkanTexture`) would surface as a driver-level crash / hang /
//! `VK_ERROR_DEVICE_LOST`, not a VUID message — this test is the real-device
//! smoke that the O1 lifetime pipeline runs crash-free end-to-end, on top of
//! the CPU-only `BindlessSlotAllocator` fence-gate unit tests (`bindless.rs`)
//! that already exhaustively prove the recycle math without a device.
//!
//! # Running (both feature configurations — the orchestrator, NOT a subagent)
//!
//! ```text
//! cargo test -p boyko-app --test texture_retire_lifecycle -- --ignored --test-threads=1
//! cargo test -p boyko-app --features hwrt --test texture_retire_lifecycle -- --ignored --test-threads=1
//! ```
//!
//! `--test-threads=1` is required (windowed-test convention: a single
//! process-global GPU device). On a windowless / GPU-less box the runner exits
//! before the frame loop and this test SKIPs gracefully.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::{AssetStaging, Staged};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{
    BindlessTextureTable, ColorSpace, RETIRE_DELAY, RenderEpoch, TextureAssetsExt, TextureData,
    TextureGpu,
};

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

/// Trailing frames past `RETIRE_DELAY` after the unregister — margin against a
/// pre-acquire recreate-skip frame (advances neither `frame_index` nor
/// `submission_epoch`; mirrors `asset_streaming_f6_churn_headless.rs`'s
/// `DRAIN_FRAMES` margin). `RETIRE_DELAY` (`FRAMES_IN_FLIGHT`) is small (2-3);
/// 10 is comfortably `>>` that.
const MARGIN_FRAMES: u32 = 10;

/// The during-run lifecycle state, overwritten by `drive_lifecycle` every
/// `Main` frame — survives the runner's shutdown (a plain `Resource`, unlike
/// the NonSend `Assets<TextureGpu>` / `BindlessTextureTable` tables themselves,
/// which `teardown` removes — see `asset_streaming_f6_churn_headless.rs`'s
/// `FinalMeshStats` for the identical reasoning).
#[derive(Resource, Default)]
struct TestState {
    handle: Option<Handle<TextureGpu>>,
    bindless_slot: Option<u32>,
    retire_frame: Option<u64>,
    unregistered: bool,
    /// The first epoch at which `BindlessTextureTable::is_empty()` was observed
    /// `true` after the unregister — `None` until the slot actually recycles.
    recycled_at_epoch: Option<u64>,
    /// Set the first time `drive_lifecycle` runs — proves the frame loop
    /// actually executed (a windowless boot never runs `Main`, leaving this
    /// `false`), the same defence-in-depth companion `FinalMeshStats::captured`
    /// uses.
    ran: bool,
}

/// Reserves one `Handle<TextureGpu>` and stages a tiny (2x2, opaque magenta)
/// decoded texture for the boot upload drain to pick up, plus a minimal sun +
/// sky + camera (asset-lifetime bookkeeping is this test's concern, not the
/// rendered image — mirrors `asset_streaming_f6_churn_headless.rs`'s minimal
/// setup).
fn setup(
    mut commands: Commands,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut staging: NonSendResMut<AssetStaging<TextureGpu>>,
    mut state: ResMut<TestState>,
) {
    let handle = textures.reserve();
    staging.push(Staged {
        handle,
        cpu: TextureData {
            width: 2,
            height: 2,
            rgba8: vec![
                255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255,
            ],
            color_space: ColorSpace::Linear,
        },
    });
    state.handle = Some(handle);

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

/// Per-frame driver: on the first frame the reserved handle resolves (the boot
/// drain fills it before the frame loop starts, so this is normally frame 0),
/// stages its bindless slot for fence-gated retirement. On every later frame,
/// asserts the slot never appears recycled before its fence horizon and
/// records the first epoch at which it does.
fn drive_lifecycle(
    textures: NonSendRes<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    epoch: Res<RenderEpoch>,
    mut state: ResMut<TestState>,
) {
    state.ran = true;

    if !state.unregistered {
        let handle = state.handle.expect("invariant: setup reserved a handle before Main ever runs");
        // The boot drain runs before the frame loop starts, so this should
        // already resolve on frame 0 — but guard defensively rather than
        // assume, in case a future rung moves texture uploads onto a
        // multi-frame streaming path.
        let Some(tex) = textures.try_get(handle) else {
            return;
        };
        assert!(
            tex.bindless_slot >= 1,
            "a registered texture must never bind the reserved error slot 0, got {}",
            tex.bindless_slot
        );
        state.bindless_slot = Some(tex.bindless_slot);
        let retire_frame = epoch.0 + RETIRE_DELAY;
        state.retire_frame = Some(retire_frame);
        bindless.unregister(tex.bindless_slot, retire_frame);
        state.unregistered = true;
        return;
    }

    let retire_frame = state
        .retire_frame
        .expect("invariant: state.unregistered implies retire_frame was stamped above");
    let is_empty = bindless.is_empty();
    // Panics iff `is_empty && epoch.0 < retire_frame` — the ONLY real violation
    // (a PREMATURE recycle). `!is_empty` at ANY epoch is never a violation
    // (system-ordering lag: this system observes `is_empty()` at the START of
    // `update_with_delta`, BEFORE `retire_deferred_frees`'s own
    // `retire_ready_slots(epoch)` call for THIS SAME frame — step 4.5, AFTER
    // `wait_frame_in_flight` — has run; so `is_empty() == false` at
    // `epoch == retire_frame` is expected, not a bug: the slot recycles LATER
    // this same frame and is first OBSERVED empty on the NEXT frame's check).
    assert!(
        !is_empty || epoch.0 >= retire_frame,
        "BindlessTextureTable slot recycled BEFORE its fence horizon: epoch={} retire_frame={}",
        epoch.0,
        retire_frame
    );
    if is_empty && state.recycled_at_epoch.is_none() {
        state.recycled_at_epoch = Some(epoch.0);
    }
}

#[test]
#[ignore = "needs a real windowed GPU device (do NOT set BOYKO_DISABLE_VALIDATION — see this \
            file's module doc); run with --test-threads=1, once default-features and once \
            --features hwrt"]
fn texture_bindless_slot_retires_fence_gated_not_before_its_horizon() {
    let budget = RETIRE_DELAY as u32 + MARGIN_FRAMES;

    let mut app = App::new();
    app.insert_resource(FrameBudget(budget));
    app.insert_resource(TestState::default());
    app.add_systems(exit_after_budget);
    app.add_systems(drive_lifecycle);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app T6b texture retire lifecycle", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    // Boot-failure discrimination: on a windowless / GPU-less box the runner
    // exits BEFORE the frame loop, so the budget is untouched — SKIP (only the
    // orchestrator, with a real GPU, gets a load-bearing pass/fail here).
    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == budget {
        eprintln!(
            "SKIP texture_bindless_slot_retires_fence_gated_not_before_its_horizon: windowed \
             boot unavailable"
        );
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {budget}-frame budget");

    let state = app.world().resource::<TestState>();
    assert!(
        state.ran,
        "drive_lifecycle must have run (the frame loop executed) — else the fields below are \
         meaningless Default/None"
    );
    let slot = state.bindless_slot.expect("a bindless slot must have been resolved and recorded");
    assert!(slot >= 1, "the resolved bindless slot must never be the reserved error slot 0, got {slot}");
    let retire_frame = state.retire_frame.expect("the unregister call must have stamped a retire_frame");
    // Not-vacuous check: a slot that NEVER recycles would satisfy the per-frame
    // `!is_empty || epoch >= retire_frame` invariant trivially (`!is_empty` holds
    // forever) — this is the load-bearing "it actually happened" proof that
    // `retire_deferred_frees` really threaded `epoch` into
    // `BindlessTextureTable::retire_ready_slots` against the real device, not
    // just that a premature recycle never fired.
    let recycled_at_epoch = state.recycled_at_epoch.unwrap_or_else(|| {
        panic!(
            "the staged slot must have recycled (BindlessTextureTable::is_empty() must have \
             become true) within the {budget}-frame budget — a stuck slot here means \
             retire_deferred_frees never threaded epoch into \
             BindlessTextureTable::retire_ready_slots against the real device"
        )
    });
    assert!(
        recycled_at_epoch >= retire_frame,
        "the slot must not have been observed recycled before its fence horizon: \
         recycled_at_epoch={recycled_at_epoch} retire_frame={retire_frame}"
    );

    // `app.run()` returning normally (no panic/hang past this point, and the
    // per-frame `assert!` inside `drive_lifecycle` never fired) is this test's
    // proof that the openQ2 shutdown teardown order
    // (`Assets<TextureGpu>::destroy` before `BindlessTextureTable::destroy`,
    // `runner.rs`'s `teardown`) ran without a crash — see this file's module
    // doc for why no VUID-level oracle is available on this boot path.
}
