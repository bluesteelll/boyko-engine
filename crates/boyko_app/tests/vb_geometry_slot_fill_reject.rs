//! Defect B, coverage gap left by the fix's review (the orphan half of the VB geometry-slot
//! release) — real-device regression test: under an armed VisibilityBuffer boot, a mesh upload that
//! `Assets::fill` REJECTS must give its geometry-table slot back.
//!
//! # The path under test
//!
//! - On an armed VB boot the boot drain (`upload_mesh_assets` → `upload_assets` →
//!   `GpuUpload for MeshGpu::upload` → `build_mesh_gpu`) claims a slot through
//!   `MeshGeometryTable::register` for EVERY staged payload, before `fill` answers.
//! - The defect-B fix routes a payload `fill` rejects through `GpuUpload::orphan` onto
//!   `OrphanedMeshGpu`. `retire_deferred_frees` hands the armed table to
//!   `OrphanedMeshGpu::drain_ready`, which stages the slot with `MeshGeometryTable::unregister`; a
//!   later pass's `MeshGeometryTable::retire_ready_slots` returns it to the free list.
//! - Neither sibling reaches that `unregister`: `asset_upload_reject_mesh_leak.rs` boots Deferred,
//!   where every `geometry_slot` is the reserved slot 0, and `vb_geometry_slot_retire_churn.rs`
//!   retires meshes through the refcount path and stages no upload.
//!
//! At d5782d43, the fix's parent, the rejected value was dropped at the fill site and its slot was
//! never staged. With the routing but without that `unregister`, nothing stages it either: the slot
//! is never reissued, and its row keeps naming the buffers the drain destroyed.
//!
//! # Scenario and measure
//!
//! A `VisibilityBuffer × Mesh` boot. The startup system stages three small payloads, as the Deferred
//! sibling does: (a) a handle removed after staging, and (c) one handle staged twice. The boot drain
//! uploads all three, so three slots are claimed; the first fill of (c) lands and the other two are
//! rejected onto `OrphanedMeshGpu` with `retire_frame = RETIRE_DELAY` (the boot `RenderEpoch` is 0).
//!
//! The measure is the texture sibling's (`asset_upload_reject_texture_leak.rs`). A slot comes back
//! only through `unregister` + `retire_ready_slots`, so once at least as many meshes are registered as
//! there are holes, **the highest slot held by any live mesh equals the number of live meshes holding
//! a slot**. A slot no live mesh holds is a hole, and the maximum exceeds the count by one per leaked
//! slot. That check does not depend on the allocator's pop order.
//!
//! - **frame 0** (the frame loop runs `app.update` before its first `retire_deferred_frees` pass, so
//!   nothing can have been recycled yet): record every live mesh's slot, then register one sentinel
//!   through `cube_vb`. `BindlessSlotAllocator` issues fresh slots in ascending order, so the
//!   sentinel's slot minus one is the number of slots issued so far, and subtracting the live holders
//!   leaves the slots held by the two rejected values. That count must be exactly 2, or nothing below
//!   reaches `drain_ready`'s `unregister`. This precondition, unlike the check, relies on the order;
//! - **frame [`PROBE_FRAME`]** (far past `RETIRE_DELAY` plus the recycle pass, before the shutdown
//!   force-drain): register [`PROBES`] meshes through `cube_vb`, then read every live mesh's slot.
//!
//! # Running
//!
//! ```text
//! cargo test -p boyko-app --test vb_geometry_slot_fill_reject -- --ignored --test-threads=1
//! ```
//!
//! `BOYKO_DISABLE_VALIDATION` may be set or unset. `--test-threads=1` is required. A windowless /
//! GPU-less box, or a device whose VB resolve degrades to Deferred (no geometry table), prints
//! `SKIP` and returns: a skip, not a pass.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::{AssetStaging, Staged};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, MeshAssetsVbExt, MeshData, MeshGeometryTableSlot, RenderPath, RenderPathConfig, VB_GEOMETRY_RESERVED_SLOT,
};

/// Rejected payloads staged: (a) one removed handle, (c) the second push of a double-staged handle.
const STAGED_REJECTED: usize = 2;
/// Probe meshes registered at [`PROBE_FRAME`]: at least one per possible hole.
const PROBES: usize = STAGED_REJECTED;
/// The Main run that probes: `>> RETIRE_DELAY` (2) plus the pass that recycles a staged slot, with
/// margin for recreate-skip frames.
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
    armed: bool,
    legit_handle: Option<Handle<MeshGpu>>,
    legit_slot: Option<u32>,
    /// Sorted real slots of every live mesh at frame 0, before the sentinel.
    live_slots_at_frame0: Vec<u32>,
    sentinel_slot: Option<u32>,
    probe_slots: Vec<u32>,
    /// Sorted real slots of every live mesh at [`PROBE_FRAME`], after the probes.
    live_slots_at_probe: Vec<u32>,
}

/// A one-triangle mesh with a non-degenerate model-space AABB.
fn small_mesh() -> MeshData {
    const NORMAL: [f32; 3] = [0.0, 0.0, 1.0];
    const COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    MeshData {
        vertices: vec![
            Vertex::new([0.0, 0.0, 0.0], NORMAL, COLOR),
            Vertex::new([1.0, 0.0, 0.0], NORMAL, COLOR),
            Vertex::new([0.0, 1.0, 0.0], NORMAL, COLOR),
        ],
        indices: vec![0, 1, 2],
    }
}

/// Every live mesh's geometry slot other than the reserved one, sorted ascending.
fn live_real_slots(meshes: &Assets<MeshGpu>) -> Vec<u32> {
    let mut slots: Vec<u32> = meshes
        .iter()
        .map(|(_, mesh)| mesh.geometry_slot)
        .filter(|&slot| slot != VB_GEOMETRY_RESERVED_SLOT)
        .collect();
    slots.sort_unstable();
    slots
}

/// Registers one cube through the VB-aware path and returns its slot, or `None` when the geometry
/// table is not armed. The mesh stays in `Assets<MeshGpu>`, so the runner's shutdown frees it.
fn register_vb_cube(meshes: &mut Assets<MeshGpu>, geo: &mut MeshGeometryTableSlot, dev: &GpuDevice) -> Option<u32> {
    let table = geo.0.as_mut()?;
    let handle = meshes.cube_vb(dev.get(), 1.0, table);
    Some(meshes.mesh(handle).geometry_slot)
}

fn stage_rejected_uploads(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut staging: NonSendResMut<AssetStaging<MeshGpu>>,
    geo: NonSendRes<MeshGeometryTableSlot>,
    mut ledger: ResMut<SlotLedger>,
) {
    ledger.armed = geo.0.is_some();

    // (a) A handle removed after staging: the drain's `fill` meets a stale generation.
    let removed = meshes.reserve();
    staging.push(Staged { handle: removed, cpu: small_mesh() });
    assert!(
        meshes.remove(removed).is_none(),
        "fixture: a Loading row holds no value, so remove() returns None"
    );

    // (c) One handle staged twice: the first fill succeeds, the second finds the row Loaded.
    let twice = meshes.reserve();
    staging.push(Staged { handle: twice, cpu: small_mesh() });
    staging.push(Staged { handle: twice, cpu: small_mesh() });
    ledger.legit_handle = Some(twice);

    spawn_minimal_view(&mut commands);
}

fn drive(
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
    mut ledger: ResMut<SlotLedger>,
) {
    ledger.ran = true;
    let frame = ledger.frame;
    ledger.frame += 1;
    if !ledger.armed {
        return;
    }

    if frame == 0 {
        let handle = ledger.legit_handle.expect("invariant: the startup system staged the double-staged handle");
        ledger.legit_slot = meshes.get(handle).map(|mesh| mesh.geometry_slot);
        ledger.live_slots_at_frame0 = live_real_slots(&meshes);
        ledger.sentinel_slot = register_vb_cube(&mut meshes, &mut geo, &dev);
    } else if frame == PROBE_FRAME {
        for _ in 0..PROBES {
            if let Some(slot) = register_vb_cube(&mut meshes, &mut geo, &dev) {
                ledger.probe_slots.push(slot);
            }
        }
        ledger.live_slots_at_probe = live_real_slots(&meshes);
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
#[ignore = "needs a real windowed GPU device with the VisibilityBuffer descriptor-indexing cap; run with --test-threads=1"]
fn a_fill_rejected_mesh_upload_releases_its_vb_geometry_table_slot() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(SlotLedger::default());
    app.add_systems(exit_after_budget);
    app.add_systems(drive);
    app.add_startup_system(stage_rejected_uploads);
    app.add_plugins(EnginePlugins::window("boyko_app defect B VB geometry slot fill reject", 320, 240));
    // Inserted AFTER `add_plugins` (which installs the Deferred default), as `vb_mesh.rs` does.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP a_fill_rejected_mesh_upload_releases_its_vb_geometry_table_slot: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    let ledger = app.world().resource::<SlotLedger>();
    assert!(ledger.ran, "the drive system must have run — else every field below is a Default");
    if !ledger.armed {
        eprintln!(
            "SKIP a_fill_rejected_mesh_upload_releases_its_vb_geometry_table_slot: the VisibilityBuffer boot \
             built no MeshGeometryTable on this device (resolve degraded) — nothing was measured"
        );
        return;
    }

    let sentinel = ledger.sentinel_slot.expect("frame 0 ran on an armed table");
    let orphan_slots_at_frame0: Vec<u32> =
        (1..sentinel).filter(|slot| ledger.live_slots_at_frame0.binary_search(slot).is_err()).collect();
    let live = ledger.live_slots_at_probe.len();
    let max_slot = ledger.live_slots_at_probe.last().copied().unwrap_or(0) as usize;
    let unheld_at_probe: Vec<u32> = (1..=max_slot as u32)
        .filter(|slot| ledger.live_slots_at_probe.binary_search(slot).is_err())
        .collect();
    eprintln!(
        "[defect B VB fill reject] legit slot={:?}; live slots at frame 0={:?}; sentinel slot={sentinel}; \
         slots held by no live mesh at frame 0={orphan_slots_at_frame0:?}; probe slots at frame \
         {PROBE_FRAME}={:?}; live slots at frame {PROBE_FRAME}={:?}; max={max_slot} count={live}; \
         unheld={unheld_at_probe:?}",
        ledger.legit_slot, ledger.live_slots_at_frame0, ledger.probe_slots, ledger.live_slots_at_probe
    );

    let legit_slot = ledger.legit_slot.expect(
        "precondition: the boot drain must have filled the first staging of the double-staged handle — \
         otherwise the drain never ran and nothing below measures anything",
    );
    assert_ne!(
        legit_slot, VB_GEOMETRY_RESERVED_SLOT,
        "precondition: an upload on an armed VB boot claims a real geometry slot, never the reserved slot 0"
    );
    assert_ne!(
        sentinel, VB_GEOMETRY_RESERVED_SLOT,
        "precondition: the frame-0 sentinel claimed a real slot (the table is not exhausted)"
    );
    assert_eq!(
        orphan_slots_at_frame0.len(),
        STAGED_REJECTED,
        "precondition: at frame 0, before any recycle pass, exactly the {STAGED_REJECTED} fill()-rejected \
         uploads hold a slot no live mesh holds (got {orphan_slots_at_frame0:?} below sentinel slot \
         {sentinel}). Fewer means the rejected uploads never claimed a slot, so nothing below reaches \
         OrphanedMeshGpu::drain_ready's unregister; more means something outside Assets<MeshGpu> holds \
         slots, or fresh slots are no longer issued in ascending order"
    );
    assert_eq!(ledger.probe_slots.len(), PROBES, "precondition: every probe mesh registered at frame {PROBE_FRAME}");
    assert!(
        !ledger.probe_slots.contains(&VB_GEOMETRY_RESERVED_SLOT),
        "precondition: every probe claimed a real slot (the table is not exhausted), got {:?}",
        ledger.probe_slots
    );
    assert_eq!(
        live,
        ledger.live_slots_at_frame0.len() + 1 + PROBES,
        "precondition: exactly the frame-0 live meshes, the sentinel and the {PROBES} probes hold a slot — \
         something else registered or removed a mesh, so the slot arithmetic below is not about this fixture"
    );

    let holes = max_slot.saturating_sub(live);
    assert_eq!(
        max_slot, live,
        "REGRESSION (defect B, a fill()-rejected mesh upload leaks its VB geometry-table slot): {holes} \
         slot(s) at or below slot {max_slot} are held by NO live mesh at frame {PROBE_FRAME} ({live} live \
         meshes hold a slot; unheld: {unheld_at_probe:?}). The {STAGED_REJECTED} rejected uploads held \
         {orphan_slots_at_frame0:?} at frame 0 and those slots were never reissued: \
         OrphanedMeshGpu::drain_ready no longer stages a real geometry_slot through \
         MeshGeometryTable::unregister, or retire_deferred_frees no longer threads the armed table to it \
         or no longer calls MeshGeometryTable::retire_ready_slots"
    );
}
