//! Defect B (2026-09-11 latent-defects checkpoint, section B), texture half — regression test,
//! written red-first on a real device: a texture upload that `Assets::fill` REJECTS must give back
//! its bindless slot (and, with it, its image).
//!
//! # The defect, as it stood at d5782d43
//!
//! `boyko_render::gpu_upload::upload_assets` runs `build_texture_gpu` — which uploads a
//! `VulkanTexture` and claims a slot in the `BindlessTextureTable` (`texture.rs`:
//! `let bindless_slot = aux.register(ctx, texture.view());`) — and only then calls
//! `assets.fill(staged.handle, gpu)`. Before the fix, `Err((_, value))` was discarded with `let _ =`
//! (a line dating from b5312171, rung A3b). `TextureGpu`'s drop glue is device-inert, so neither
//! the image nor the slot was released. `OrphanedTextureGpu::drain_ready` (unregister the slot,
//! destroy the image), in the tree since 8e48f7fe, existed for exactly this value and nothing pushed
//! to it.
//!
//! The defect-B fix hands the rejected value to `GpuUpload::orphan`, which for `TextureGpu` pushes it
//! onto `OrphanedTextureGpu`, and that queue's `drain_ready` unregisters the slot and destroys the
//! image. This test goes red again if the routing or the slot release is lost.
//!
//! Staged here, as in the mesh sibling (`asset_upload_reject_mesh_leak.rs`): (a) a handle removed
//! after staging, and (c) one handle staged twice.
//!
//! # How the leak itself is observed
//!
//! No validation layer and no image-allocation counter exist on this boot path (textures use
//! dedicated device memory, outside the block pool the mesh sibling reads). The observable is the
//! bindless slot, and the rule that makes it measurable is in `boyko_render::bindless`:
//! `BindlessSlotAllocator` issues slots from `1..capacity` (slot 0 is the reserved error texture),
//! and a freed slot returns to the free list only through `unregister` + `retire_ready_slots`,
//! which `retire_deferred_frees` threads every frame.
//!
//! So in a table with no leaked slot, registering at least as many new textures as there are holes
//! leaves the in-use slots hole-free: **the highest slot held by any live texture equals the number
//! of live textures.** A leaked slot is a hole no live texture holds, so the maximum exceeds the
//! count by one per leak. This holds for either fix shape — a rejected value routed into
//! `OrphanedTextureGpu` (its slot is recycled into the probes) or an upload skipped for a doomed
//! handle (no slot was ever taken). It does not depend on the allocator's pop order.
//!
//! - **startup**: stage the three 2x2 payloads (no texture is registered before the boot drain in
//!   this scene — `build_texture_gpu` is the only production caller of the table's `register`);
//! - **frame 0**: confirm the legit fill of the double-staged handle landed (the drain ran);
//! - **frame [`PROBE_FRAME`]** (far past `RETIRE_DELAY` + one recycle frame, before the shutdown
//!   force-drain): register [`PROBES`] fresh textures into `Assets<TextureGpu>` (so shutdown frees
//!   them), then read `max(bindless_slot)` and `len()`.
//!
//! What it does NOT catch: a regression that still recycles the slot but loses `destroy_texture`
//! would pass.
//!
//! # Running
//!
//! ```text
//! cargo test -p boyko-app --test asset_upload_reject_texture_leak -- --ignored --test-threads=1
//! ```
//!
//! `BOYKO_DISABLE_VALIDATION` may be set or unset. `--test-threads=1` is required. On a windowless /
//! GPU-less box the test prints `SKIP` and returns: a skip, not a pass.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::{AssetStaging, Staged};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{BindlessTextureTable, ColorSpace, TextureAssetsExt, TextureData, TextureGpu, build_texture_gpu};

/// Rejected payloads staged: (a) one removed handle, (c) the second push of a double-staged handle.
const STAGED_REJECTED: usize = 2;
/// Probe textures registered at [`PROBE_FRAME`]: at least one per possible hole.
const PROBES: usize = STAGED_REJECTED;
/// The Main run that probes: `>> RETIRE_DELAY` (2) plus the one frame a slot waits in
/// `retiring_slots`, with margin for recreate-skip frames.
const PROBE_FRAME: u32 = 12;
const BUDGET: u32 = PROBE_FRAME + 4;

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

/// Post-run evidence; a plain `Resource`, so it survives shutdown.
#[derive(Resource, Default)]
struct SlotLedger {
    frame: u32,
    ran: bool,
    legit_handle: Option<Handle<TextureGpu>>,
    legit_slot: Option<u32>,
    probe_slots: Vec<u32>,
    max_live_slot: Option<u32>,
    live_textures: Option<usize>,
}

fn tiny_texture() -> TextureData {
    TextureData {
        width: 2,
        height: 2,
        rgba8: vec![255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255],
        color_space: ColorSpace::Linear,
    }
}

fn stage_rejected_uploads(
    mut commands: Commands,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut staging: NonSendResMut<AssetStaging<TextureGpu>>,
    mut ledger: ResMut<SlotLedger>,
) {
    // (a) A handle removed after staging.
    let removed = textures.reserve();
    staging.push(Staged { handle: removed, cpu: tiny_texture() });
    assert!(
        textures.remove(removed).is_none(),
        "fixture: a Loading row holds no value, so remove() returns None"
    );

    // (c) One handle staged twice: the first fill succeeds, the second finds the row Loaded.
    let twice = textures.reserve();
    staging.push(Staged { handle: twice, cpu: tiny_texture() });
    staging.push(Staged { handle: twice, cpu: tiny_texture() });
    ledger.legit_handle = Some(twice);

    spawn_minimal_view(&mut commands);
}

fn drive(
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    dev: NonSendRes<GpuDevice>,
    mut ledger: ResMut<SlotLedger>,
) {
    ledger.ran = true;
    let frame = ledger.frame;
    ledger.frame += 1;

    if frame == 0 {
        let handle = ledger.legit_handle.expect("invariant: the startup system staged the double-staged handle");
        ledger.legit_slot = textures.try_get(handle).map(|t| t.bindless_slot);
    } else if frame == PROBE_FRAME {
        for _ in 0..PROBES {
            let probe = build_texture_gpu(dev.get(), &mut bindless, &tiny_texture());
            ledger.probe_slots.push(probe.bindless_slot);
            // Owned by the table from here on, so the runner's shutdown destroys it.
            let _handle = textures.add(probe);
        }
        ledger.max_live_slot = textures.iter().map(|(_, t)| t.bindless_slot).max();
        ledger.live_textures = Some(textures.len());
    }
}

fn spawn_minimal_view(commands: &mut Commands) {
    const SUN_DIR: [f32; 3] = [-0.45, 0.82, 0.36];
    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.96, 0.90], 2.8),
    });
    commands.spawn(SkyLight::new([0.26, 0.32, 0.42], [0.12, 0.11, 0.10]));
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 1.7, 6.0), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
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

#[test]
#[ignore = "needs a real windowed GPU device (validation not required); run with --test-threads=1"]
fn a_fill_rejected_texture_upload_releases_its_bindless_slot() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(SlotLedger::default());
    app.add_systems(exit_after_budget);
    app.add_systems(drive);
    app.add_startup_system(stage_rejected_uploads);
    app.add_plugins(EnginePlugins::window("boyko_app defect B texture reject leak", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP a_fill_rejected_texture_upload_releases_its_bindless_slot: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    let ledger = app.world().resource::<SlotLedger>();
    assert!(ledger.ran, "the drive system must have run — else every field below is a Default");
    eprintln!(
        "[defect B texture] legit slot={:?}; probe slots at frame {PROBE_FRAME}={:?}; max live slot={:?}; \
         live textures={:?}",
        ledger.legit_slot, ledger.probe_slots, ledger.max_live_slot, ledger.live_textures
    );

    let legit_slot = ledger.legit_slot.expect(
        "precondition: the boot drain must have filled the first staging of the double-staged handle — \
         otherwise the drain never ran and nothing below measures anything",
    );
    assert!(legit_slot >= 1, "precondition: a registered texture never binds the reserved slot 0, got {legit_slot}");
    assert_eq!(ledger.probe_slots.len(), PROBES, "precondition: every probe texture registered at frame {PROBE_FRAME}");
    let live = ledger.live_textures.expect("the probe frame was reached");
    let max_slot = ledger.max_live_slot.expect("at least the probes are live") as usize;
    assert_eq!(
        live,
        1 + PROBES,
        "precondition: exactly the legit texture and the {PROBES} probes are live — something else \
         registered or removed a texture, so the slot arithmetic below is not about this fixture"
    );

    let holes = max_slot - live;
    assert_eq!(
        max_slot, live,
        "REGRESSION (defect B, a fill()-rejected texture upload leaks its bindless slot): {holes} bindless \
         slot(s) below slot {max_slot} are held by NO live texture at frame {PROBE_FRAME} ({live} live \
         textures) — the rejected texture uploads (a removed handle, a double-staged handle) kept their \
         slots (and images): either `upload_assets` (boyko_render/src/gpu_upload.rs) no longer hands \
         fill's Err value to GpuUpload::orphan, or OrphanedTextureGpu::drain_ready no longer \
         unregisters the slot. Before the fix (d5782d43) the value was discarded with \
         `let _ = assets.fill(staged.handle, gpu)`"
    );
}
