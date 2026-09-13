//! Defect B, related gap (2026-09-11 latent-defects checkpoint, section B, last paragraph) —
//! regression test, written red-first on a real device: under a VisibilityBuffer boot, a mesh
//! RETIRED through the normal refcount path must give back its geometry-table slot.
//!
//! # The gap, as it stood at d5782d43 (the table and its slot API arrived in 2322eacb)
//!
//! - A VB-aware registration claims a slot: `build_mesh_gpu` →
//!   `MeshGeometryTable::register` (`mesh_assets.rs`,
//!   `Some(table) if ctx.vb_geometry_table_armed() => table.register(`).
//! - The only retire path for a mesh is `retire_deferred_frees`' mesh branch
//!   (`asset_refcount.rs`). Before the fix it ran `mesh_assets.retire(entry.slot)`, then
//!   `destroy_blas` (hwrt) and `destroy_buffer` × 2, and never read `mesh.geometry_slot` or touched
//!   `MeshGeometryTableSlot`.
//! - `MeshGeometryTable::unregister` (`mesh_geometry_table.rs`) had NO caller anywhere in the tree,
//!   and `MeshGeometryTable::retire_ready_slots` had none outside its own unit test — so even a
//!   staged slot could never have returned to the free list.
//! - `OrphanedMeshGpu::drain_ready` (the fill-reject teardown) did not unregister it either.
//!
//! So every retired mesh leaked one slot of `MESH_GEOMETRY_TABLE_CAPACITY` (4096), and its
//! descriptor kept naming the destroyed buffers, because the slot was never reissued.
//!
//! The defect-B fix makes that mesh branch stage the slot with `MeshGeometryTable::unregister` at
//! `epoch.saturating_add(RETIRE_DELAY)`, makes `OrphanedMeshGpu::drain_ready` stage it at `epoch`,
//! and has `retire_deferred_frees` call `MeshGeometryTable::retire_ready_slots` whenever the armed
//! table has work. This test pins the retire-branch half; `vb_geometry_slot_fill_reject.rs` pins the
//! orphan half.
//!
//! # Scenario and measure
//!
//! The f6 churn (`asset_streaming_f6_churn_headless.rs`) on a `VisibilityBuffer × Mesh` boot, with
//! every mesh registered through `MeshAssetsVbExt::cube_vb` so each claims a real slot:
//! [`INITIAL_MESHES`] single-owner meshes at startup, then for [`CHURN_FRAMES`] frames despawn the
//! oldest and register + spawn a fresh one, then a [`DRAIN_FRAMES`] tail. Every registration's
//! `geometry_slot` is recorded.
//!
//! The allocator is `BindlessSlotAllocator`, which issues fresh slots in ascending order. Without
//! recycling, `INITIAL_MESHES + CHURN_FRAMES` registrations occupy exactly that many distinct slots
//! (span = 27). With recycling, at most `INITIAL_MESHES` live meshes plus those inside the retire
//! horizon (`RETIRE_DELAY + 1` frames to retire, another `RETIRE_DELAY + 1` for the slot's own fence)
//! hold a slot at once, so the span stays near `INITIAL_MESHES + 2 * (RETIRE_DELAY + 1)` ≈ 9. The
//! bound asserted is `INITIAL_MESHES + CHURN_FRAMES / 2` = 15 — the f6 `high_water` shape.
//!
//! Preconditions checked first, so a red on the span is only ever the slot leak: the frame loop
//! ran, the geometry table was armed, every registration got a real slot, and every churned mesh
//! actually retired (`DeferredFree` empty, `free_epoch == 2 * CHURN_FRAMES`).
//!
//! # Running
//!
//! ```text
//! cargo test -p boyko-app --test vb_geometry_slot_retire_churn -- --ignored --test-threads=1
//! ```
//!
//! `BOYKO_DISABLE_VALIDATION` may be set or unset. `--test-threads=1` is required. A windowless /
//! GPU-less box, or a device whose VB resolve degrades to Deferred (no geometry table), prints
//! `SKIP` and returns: a skip, not a pass.

#![cfg(windows)]

use std::collections::VecDeque;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig};
use boyko_scene::DeferredFree;

const INITIAL_MESHES: u32 = 3;
const CHURN_FRAMES: u32 = 24;
const DRAIN_FRAMES: u32 = 8;
const BUDGET: u32 = CHURN_FRAMES + DRAIN_FRAMES;

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

#[derive(Resource)]
struct ChurnState {
    queue: VecDeque<Entity>,
    remaining: u32,
}

/// Post-run evidence; a plain `Resource`, so it survives shutdown.
#[derive(Resource, Default)]
struct SlotLedger {
    ran: bool,
    armed: bool,
    registrations: u32,
    reserved_slot_registrations: u32,
    min_slot: Option<u32>,
    max_slot: Option<u32>,
    free_epoch: u64,
}

impl SlotLedger {
    fn record(&mut self, slot: u32) {
        self.registrations += 1;
        if slot == 0 {
            self.reserved_slot_registrations += 1;
        }
        self.min_slot = Some(self.min_slot.map_or(slot, |m| m.min(slot)));
        self.max_slot = Some(self.max_slot.map_or(slot, |m| m.max(slot)));
    }
}

/// Registers one cube through the VB-aware path and returns its handle and slot, or `None` when the
/// geometry table is not armed on this device.
fn register_vb_cube(
    meshes: &mut Assets<MeshGpu>,
    geo: &mut MeshGeometryTableSlot,
    dev: &GpuDevice,
) -> Option<(MeshHandle, u32)> {
    let table = geo.0.as_mut()?;
    let handle = meshes.cube_vb(dev.get(), 1.0, table);
    Some((handle, meshes.mesh(handle).geometry_slot))
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
    mut churn: ResMut<ChurnState>,
    mut ledger: ResMut<SlotLedger>,
) {
    ledger.armed = geo.0.is_some();
    if !ledger.armed {
        return;
    }
    for i in 0..INITIAL_MESHES {
        let (handle, slot) = register_vb_cube(&mut meshes, &mut geo, &dev).expect("invariant: armed checked above");
        ledger.record(slot);
        let e = commands
            .spawn(MeshBundle::new(handle, Transform::from_translation(Vec3::new(i as f32, 0.5, 0.0))))
            .id();
        churn.queue.push_back(e);
    }
    spawn_minimal_view(&mut commands);
}

fn churn_step(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
    mut churn: ResMut<ChurnState>,
    mut ledger: ResMut<SlotLedger>,
) {
    ledger.ran = true;
    ledger.free_epoch = meshes.free_epoch();
    if !ledger.armed || churn.remaining == 0 {
        return;
    }
    churn.remaining -= 1;

    if let Some(old) = churn.queue.pop_front() {
        commands.entity(old).despawn();
    }
    let (handle, slot) = register_vb_cube(&mut meshes, &mut geo, &dev).expect("invariant: armed checked above");
    ledger.record(slot);
    let e = commands
        .spawn(MeshBundle::new(handle, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0))))
        .id();
    churn.queue.push_back(e);
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
#[ignore = "needs a real windowed GPU device with the VisibilityBuffer descriptor-indexing cap; run with --test-threads=1"]
fn a_retired_mesh_releases_its_vb_geometry_table_slot() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(ChurnState { queue: VecDeque::new(), remaining: CHURN_FRAMES });
    app.insert_resource(SlotLedger::default());
    app.add_systems(exit_after_budget);
    app.add_systems(churn_step);
    app.add_startup_system(setup);
    app.add_plugins(EnginePlugins::window("boyko_app VB geometry slot retire churn", 320, 240));
    // Inserted AFTER `add_plugins` (which installs the Deferred default), as `vb_mesh.rs` does.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP a_retired_mesh_releases_its_vb_geometry_table_slot: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    let ledger = app.world().resource::<SlotLedger>();
    assert!(ledger.ran, "the churn system must have run — else every field below is a Default");
    if !ledger.armed {
        eprintln!(
            "SKIP a_retired_mesh_releases_its_vb_geometry_table_slot: the VisibilityBuffer boot built no \
             MeshGeometryTable on this device (resolve degraded) — nothing was measured"
        );
        return;
    }

    let (min_slot, max_slot) = (ledger.min_slot.expect("registrations happened"), ledger.max_slot.expect("registrations happened"));
    let span = max_slot - min_slot + 1;
    let deferred_empty = app.world().resource::<DeferredFree>().is_empty();
    eprintln!(
        "[VB slot churn] registrations={} (reserved-slot={}) slots {min_slot}..={max_slot} span={span}; \
         free_epoch={} DeferredFree empty={deferred_empty}",
        ledger.registrations, ledger.reserved_slot_registrations, ledger.free_epoch
    );

    assert_eq!(
        ledger.registrations,
        INITIAL_MESHES + CHURN_FRAMES,
        "precondition: every startup and churn registration was recorded"
    );
    assert_eq!(
        ledger.reserved_slot_registrations, 0,
        "precondition: every VB-aware registration claimed a real slot (never the reserved slot 0)"
    );
    assert!(
        deferred_empty,
        "precondition: every churned mesh's FreeEntry drained past its fence horizon by the DRAIN tail"
    );
    assert_eq!(
        ledger.free_epoch,
        2 * u64::from(CHURN_FRAMES),
        "precondition: every churned mesh fully retired (two free_epoch bumps each) — the retire path ran"
    );

    let bound = INITIAL_MESHES + CHURN_FRAMES / 2;
    assert!(
        span <= bound,
        "REGRESSION (defect B, VB geometry-slot leak on the refcount retire path): {} registrations used a \
         slot span of {span} ({min_slot}..={max_slot}); with recycling it stays <= {bound}, without it every \
         registration takes a fresh slot (span {}). {CHURN_FRAMES} meshes retired through \
         retire_deferred_frees and their slots were not reissued: its mesh branch no longer stages \
         mesh.geometry_slot through MeshGeometryTable::unregister, or MeshGeometryTable::retire_ready_slots \
         is no longer called for the armed table (both calls were missing at d5782d43, which measured span 27)",
        ledger.registrations,
        INITIAL_MESHES + CHURN_FRAMES
    );
}
