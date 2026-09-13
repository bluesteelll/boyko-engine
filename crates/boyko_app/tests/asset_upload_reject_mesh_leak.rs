//! Defect B (2026-09-11 latent-defects checkpoint, section B) — regression test, written red-first
//! on a real device: a mesh upload that `Assets::fill` REJECTS must not keep its device buffers alive.
//!
//! # The defect, as it stood at d5782d43
//!
//! `boyko_render::gpu_upload::upload_assets` builds the resident value first (`A::upload` — for
//! `MeshGpu`, a vertex and an index `BoundBuffer`, plus a BLAS under `hwrt`) and only then calls
//! `assets.fill(staged.handle, gpu)`. Before the fix, `Err((_, value))` was discarded with `let _ =`
//! (a line dating from b5312171, rung A3b). `MeshGpu` implements no `Drop` (`mesh.rs`: "`MeshGpu`
//! does NOT implement `Drop`"), so nothing ever destroyed those buffers. `OrphanedMeshGpu` —
//! inserted by the runner, drained behind the fence gate every frame and force-drained at shutdown,
//! and in the tree since F6 (9370cdb1) — existed for exactly this value, and nothing pushed to it.
//!
//! The defect-B fix routes the value instead: `if let Err((_, rejected)) = assets.fill(..)` hands it
//! to `GpuUpload::orphan`, which for `MeshGpu` pushes it onto `OrphanedMeshGpu` stamped
//! `epoch.saturating_add(RETIRE_DELAY)`, and `OrphanedMeshGpu::drain_ready` destroys its buffers.
//! This test goes red again if either half is lost.
//!
//! A startup system reaches the `Err` arm at the boot drain in two ways, both staged here:
//! - **(a) removed handle**: `Assets::reserve`, `AssetStaging::push`, then `Assets::remove` — the
//!   row becomes `Vacant` with a bumped generation, so the drain's `fill` finds a stale handle;
//! - **(c) double stage**: the same handle pushed twice (`AssetStaging::push` is `pub`) — the first
//!   fill succeeds, the second finds the row already `Loaded`.
//!
//! # How the leak itself is observed
//!
//! The windowed runner enables validation only when `BOYKO_ENABLE_VALIDATION` is set, and when
//! this test was written the engine exposed no live-allocation count (`pool_live_allocations` came
//! later), so `asset_streaming_f6_churn_headless.rs` had no leak detector to reuse — its numeric
//! checks are CPU bookkeeping. This test therefore observes the host-visible block pool, whose
//! rules are fixed in `boyko_rhi_vulkan::memory::BlockPool` and `SubAllocator`:
//!
//! - every `HostVisibleCoherent` buffer (both mesh buffers are) is first-fit sub-allocated from a
//!   pool of blocks; a request that fits no existing block appends a block of
//!   `capacity_for(size) = 64 MiB * ceil((size + 1 MiB) / 64 MiB)`;
//! - blocks are never released before the device dies, and a destroyed buffer returns its range to
//!   its block's free list.
//!
//! Each staged payload is one [`UPLOAD_BYTES`] (66 MiB) vertex buffer, so every upload that
//! actually allocates mints its own 128 MiB block: exactly one "excess unit" of
//! `(total_capacity - 64 MiB * block_count) / 64 MiB`. A [`PROBE_BYTES`] (65 MiB) probe cannot fit
//! any default 64 MiB block, nor the tail any block keeps after its live allocation
//! (`capacity_for(s) - s < 65 MiB` for every `s`), so a probe fits WITHOUT growing the pool if and
//! only if some big buffer's region has been freed. Counting how many probes fit before the pool
//! grows counts the freed 66 MiB regions.
//!
//! The runner's own drains run exactly as in production; this test only reads the pool:
//! - **startup**: record the excess units, stage the three 66 MiB payloads;
//! - **frame 0** (after the boot drain): record the excess units again — the difference is the
//!   number of staged uploads that allocated. Then remove the LEGITIMATELY filled payload and
//!   destroy its buffers by hand: its region is the positive control, a region the detector MUST
//!   find reusable;
//! - **frame [`PROBE_FRAME`]** (far past `RETIRE_DELAY`, so any fence-gated orphan drain has run,
//!   and long before the shutdown force-drain): count reusable regions.
//!
//! `still_held = (uploads that allocated - 1 legit) - (reusable regions - 1 control)` must be 0.
//! At d5782d43 it measured 2 (3 uploads allocated, 1 reusable region). The formula also stays correct
//! for a fix that skips the upload of a doomed handle altogether (then only the legit upload
//! allocates and the control is the only region).
//!
//! # Running
//!
//! ```text
//! cargo test -p boyko-app --test asset_upload_reject_mesh_leak -- --ignored --test-threads=1
//! ```
//!
//! `BOYKO_DISABLE_VALIDATION` may be set or unset — the windowed runner requests validation only
//! when `BOYKO_ENABLE_VALIDATION` is set. `--test-threads=1` is required (one process-global GPU
//! device). On a windowless / GPU-less box the runner exits before the frame loop and this test
//! prints `SKIP` and returns: a skip, not a pass. It allocates ~4 x 128 MiB of host-visible
//! device memory.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::{AssetStaging, Staged};
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::MeshData;
use boyko_render::mesh::Vertex;
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;

const MIB: u64 = 1024 * 1024;
/// `boyko_rhi_vulkan`'s private `SHARED_HOST_BLOCK_CAPACITY`. If it changes, the frame-0
/// precondition (`uploads that allocated >= 1`) goes red with a message saying so.
const DEFAULT_HOST_BLOCK: u64 = 64 * MIB;
/// Each staged payload's vertex-buffer size: above one default block, below two.
const UPLOAD_BYTES: u64 = DEFAULT_HOST_BLOCK + 2 * MIB;
/// The probe size: fits a freed [`UPLOAD_BYTES`] region, fits no default block and no block tail.
const PROBE_BYTES: u64 = DEFAULT_HOST_BLOCK + MIB;
/// Payloads staged: (a) one removed handle, (c) one handle staged twice.
const STAGED_UPLOADS: u64 = 3;
/// Upper bound on probes, so a contaminated pool cannot loop far.
const MAX_PROBES: u32 = 8;
/// The Main run that probes: `>> RETIRE_DELAY` (2), margin for recreate-skip frames.
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

/// Everything the post-run assertions read. A plain `Resource`, so it survives the runner's
/// shutdown (the NonSend asset tables do not).
#[derive(Resource, Default)]
struct PoolLedger {
    frame: u32,
    ran: bool,
    units_at_startup: Option<u64>,
    units_after_drain: Option<u64>,
    capacity_is_block_multiple: bool,
    legit_handle: Option<Handle<MeshGpu>>,
    legit_was_loaded: bool,
    reusable_regions: Option<u32>,
    blocks_before_probe: usize,
    bytes_before_probe: u64,
}

/// Excess 64 MiB units in the host pool: one per block minted for an oversize request of
/// `(64 MiB, 128 MiB - 1 MiB]`.
fn excess_units(ctx: &VulkanContext) -> u64 {
    let (blocks, _) = ctx.pool_block_counts();
    let (bytes, _) = ctx.pool_total_capacities();
    (bytes - DEFAULT_HOST_BLOCK * blocks as u64) / DEFAULT_HOST_BLOCK
}

/// A mesh whose vertex buffer is at least [`UPLOAD_BYTES`]; three indices, so the index buffer is
/// tiny and lands in a default block.
fn oversize_mesh() -> MeshData {
    let stride = core::mem::size_of::<Vertex>() as u64;
    let count = UPLOAD_BYTES.div_ceil(stride) as usize;
    let vertex = Vertex::new([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0]);
    MeshData { vertices: vec![vertex; count], indices: vec![0, 1, 2] }
}

fn stage_rejected_uploads(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut staging: NonSendResMut<AssetStaging<MeshGpu>>,
    dev: NonSendRes<GpuDevice>,
    mut ledger: ResMut<PoolLedger>,
) {
    ledger.units_at_startup = Some(excess_units(dev.get()));

    // (a) A handle removed after staging: the drain's `fill` meets a stale generation.
    let removed = meshes.reserve();
    staging.push(Staged { handle: removed, cpu: oversize_mesh() });
    assert!(
        meshes.remove(removed).is_none(),
        "fixture: a Loading row holds no value, so remove() returns None"
    );

    // (c) One handle staged twice: the first fill succeeds (the positive control), the second
    // finds the row Loaded.
    let twice = meshes.reserve();
    staging.push(Staged { handle: twice, cpu: oversize_mesh() });
    staging.push(Staged { handle: twice, cpu: oversize_mesh() });
    ledger.legit_handle = Some(twice);

    spawn_minimal_view(&mut commands);
}

/// Destroys a mesh the test owns by value. The mesh was never referenced by any entity, so no
/// submission can read it.
fn destroy_mesh(ctx: &VulkanContext, mesh: MeshGpu) {
    #[cfg_attr(not(feature = "hwrt"), allow(unused_mut))]
    let mut mesh = mesh;
    #[cfg(feature = "hwrt")]
    if let Some(blas) = mesh.blas.take() {
        // SAFETY: the BLAS belongs to a mesh no entity ever referenced, so no TLAS build or trace
        // read it; `take` hands it over exactly once and it was built on this `ctx`.
        unsafe { boyko_rhi_vulkan::accel_build::destroy_blas(ctx, blas) };
    }
    // SAFETY: both buffers were created by the boot drain's `build_mesh_gpu` on this `ctx`; the
    // mesh was never drawn (no entity carries its handle), so no submission references them; the
    // by-value move destroys each exactly once.
    unsafe {
        ctx.destroy_buffer(mesh.vertex_buffer);
        ctx.destroy_buffer(mesh.index_buffer);
    }
}

/// Allocates [`PROBE_BYTES`] host-visible buffers until one grows the pool; returns how many fit
/// without growth. Every probe is destroyed before returning.
fn count_reusable_oversize_regions(ctx: &VulkanContext) -> u32 {
    let mut held = Vec::with_capacity(MAX_PROBES as usize);
    let mut reusable = 0u32;
    for _ in 0..MAX_PROBES {
        let blocks_before = ctx.pool_block_counts().0;
        let probe = ctx
            .create_buffer(&BufferDesc {
                size: PROBE_BYTES,
                usage: BufferUsage::STORAGE,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("fixture: a host-visible probe allocates (the pool grows on demand)");
        let grew = ctx.pool_block_counts().0 != blocks_before;
        held.push(probe);
        if grew {
            break;
        }
        reusable += 1;
    }
    for probe in held {
        // SAFETY: created just above on this `ctx`, never bound to a descriptor nor submitted,
        // destroyed exactly once.
        unsafe { ctx.destroy_buffer(probe) };
    }
    reusable
}

fn drive(mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>, mut ledger: ResMut<PoolLedger>) {
    let ctx = dev.get();
    ledger.ran = true;
    let frame = ledger.frame;
    ledger.frame += 1;

    if frame == 0 {
        ledger.units_after_drain = Some(excess_units(ctx));
        ledger.capacity_is_block_multiple = ctx.pool_total_capacities().0.is_multiple_of(DEFAULT_HOST_BLOCK);
        let handle = ledger.legit_handle.expect("invariant: the startup system staged the double-staged handle");
        if let Some(mesh) = meshes.remove(handle) {
            ledger.legit_was_loaded = true;
            destroy_mesh(ctx, mesh);
        }
    } else if frame == PROBE_FRAME {
        ledger.blocks_before_probe = ctx.pool_block_counts().0;
        ledger.bytes_before_probe = ctx.pool_total_capacities().0;
        ledger.reusable_regions = Some(count_reusable_oversize_regions(ctx));
    }
}

/// A minimal sun + sky + camera, verbatim in shape from `asset_streaming_f6_churn_headless.rs`.
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
fn a_fill_rejected_mesh_upload_releases_its_device_buffers() {
    let mut app = App::new();
    app.insert_resource(FrameBudget(BUDGET));
    app.insert_resource(PoolLedger::default());
    app.add_systems(exit_after_budget);
    app.add_systems(drive);
    app.add_startup_system(stage_rejected_uploads);
    app.add_plugins(EnginePlugins::window("boyko_app defect B mesh reject leak", 320, 240));

    let exit = app.run();
    assert!(exit.0, "the windowed runner returns AppExit(true)");

    let remaining = app.world().resource::<FrameBudget>().0;
    if remaining == BUDGET {
        eprintln!("SKIP a_fill_rejected_mesh_upload_releases_its_device_buffers: windowed boot unavailable");
        return;
    }
    assert_eq!(remaining, 0, "the frame loop ran the full {BUDGET}-frame budget");

    let ledger = app.world().resource::<PoolLedger>();
    assert!(ledger.ran, "the drive system must have run — else every field below is a Default");
    let units_at_startup = ledger.units_at_startup.expect("the startup system ran");
    let units_after_drain = ledger.units_after_drain.expect("frame 0 ran");
    let reusable = ledger.reusable_regions.expect("the probe frame was reached");
    let minted = units_after_drain.saturating_sub(units_at_startup);

    eprintln!(
        "[defect B mesh] excess units: startup={units_at_startup} after-drain={units_after_drain} \
         (uploads that allocated={minted} of {STAGED_UPLOADS}); legit fill loaded={}; at frame \
         {PROBE_FRAME}: host blocks={} bytes={}; reusable 65 MiB regions={reusable}",
        ledger.legit_was_loaded, ledger.blocks_before_probe, ledger.bytes_before_probe
    );

    // Preconditions: each names what it would mean, so a red here is never mistaken for the defect.
    assert!(
        ledger.legit_was_loaded,
        "precondition: the boot drain must have filled the first staging of the double-staged handle \
         — otherwise the drain never ran and nothing below measures anything"
    );
    assert!(
        ledger.capacity_is_block_multiple,
        "precondition: the host pool's total capacity is no longer a multiple of 64 MiB — the default \
         block size changed; re-derive UPLOAD_BYTES / PROBE_BYTES"
    );
    assert!(
        (1..=STAGED_UPLOADS).contains(&minted),
        "detector precondition: {minted} oversize host blocks were minted across the boot drain; \
         expected 1..={STAGED_UPLOADS} (one per staged 66 MiB upload that allocated, the legit one at \
         least). 0 means the default block grew past 66 MiB; more means the engine minted oversize \
         blocks of its own in the window and the count is contaminated"
    );
    assert!(
        reusable >= 1,
        "detector broken: the legit payload's region, destroyed by hand at frame 0, was not reusable \
         by a {PROBE_BYTES}-byte probe at frame {PROBE_FRAME} — the probe cannot see freed regions, \
         so its count proves nothing"
    );
    assert!(
        u64::from(reusable) <= minted,
        "detector contaminated: {reusable} reusable oversize regions but only {minted} were minted by \
         this test's uploads — an engine-owned freed region is inflating the count"
    );

    // The regression check: defect B's leak.
    let rejected_that_allocated = minted - 1;
    let released = u64::from(reusable) - 1;
    let still_held = rejected_that_allocated - released;
    assert_eq!(
        still_held, 0,
        "REGRESSION (defect B, a fill()-rejected mesh upload leaks its device buffers): {still_held} of \
         the {rejected_that_allocated} rejected mesh uploads (a removed handle, a double-staged handle) \
         still hold their device buffers at frame {PROBE_FRAME}, {} frames past RETIRE_DELAY. MeshGpu \
         has no Drop, so the rejected value must reach OrphanedMeshGpu: either `upload_assets` \
         (boyko_render/src/gpu_upload.rs) no longer hands fill's Err value to GpuUpload::orphan, or \
         OrphanedMeshGpu::drain_ready no longer destroys it. Before the fix (d5782d43) the value was \
         discarded with `let _ = assets.fill(staged.handle, gpu)`",
        PROBE_FRAME - 2
    );
}
