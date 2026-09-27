//! **DM1: the shipped `Main` schedule stages every material edit, once, in compact runs.** A
//! device-free gate on `stage_material_edits` as `EnginePlugins` registers it
//! (`register_main_frame_systems`, after `gather_mesh_draws`), reading its output resource
//! `MaterialUploadStaging` — the bytes and copy regions the runner hands to the GPU.
//!
//! # What runs
//!
//! The real `EnginePlugins` composition and its real `Main` schedule, one `update` per frame. The
//! window runner is never installed, so the test inserts the two world residents the runner would
//! (`Assets<MeshGpu>`, `Assets<Material>`) — the `host_frame_zero_draws_every_mesh.rs` idiom. The
//! edits are made between frames through the world, the way a gameplay system's `get_mut` lands
//! in the asset table's edited set.
//!
//! # What it pins, frame by frame
//!
//! 1. Frame 0 drains the ten boot mints as ONE run `[0, 10)` (the runner's frame 0 uploads a full
//!    image regardless; the drain is what keeps later frames clean).
//! 2. An idle frame stages 0 rows and 0 runs, so the runner declares no copy.
//! 3. Edits to rows {2, 3, 4, 9} stage 4 packed rows and 2 runs: `(src 0, dst 2·48, 3·48)` and
//!    `(src 3·48, dst 9·48, 48)`.
//! 4. A freed row stages one all-zero row.
//! 5. The next idle frame clears what the edit frame staged.
//!
//! The kernel half — marks persist until drained, however many frames pass — is
//! `boyko_ecs`'s `assets::tests::edited_set::marks_persist_until_drained`.
//!
//! # Why its own binary
//!
//! `EnginePlugins` can be built only once per process (see `particle_host_reachable.rs`), so this
//! file holds one `#[test]`. It needs no device.

use std::time::Duration;

use boyko_app::EnginePlugins;
use boyko_ecs::App;
use boyko_ecs::ecs::core::asset::{Assets, Handle};
use boyko_render::{MATERIAL_ROW_BYTES, Material, MaterialGpu, MaterialUploadStaging, MeshGpu};
use boyko_rhi::BufferCopy;

/// Materials minted before the first frame.
const BOOT_MATERIALS: usize = 10;

fn mat(v: f32) -> Material {
    Material::from(MaterialGpu { base_color: [v; 4], mrr: [v, v, v, 0.0], emissive: [v; 4] })
}

fn run(src_row: u64, dst_row: u64, rows: u64) -> BufferCopy {
    BufferCopy {
        src_offset: src_row * MATERIAL_ROW_BYTES,
        dst_offset: dst_row * MATERIAL_ROW_BYTES,
        size: rows * MATERIAL_ROW_BYTES,
    }
}

fn frame(app: &mut App) {
    app.update_with_delta(Duration::from_millis(16));
}

/// `(row count, runs)` of the staging the last frame left.
fn staged(app: &App) -> (usize, Vec<BufferCopy>) {
    let s = app.world().resource::<MaterialUploadStaging>();
    (s.row_count(), s.runs().to_vec())
}

#[test]
fn the_main_schedule_stages_each_material_edit_once_in_compact_runs() {
    // `EnginePlugins::window` needs a title and a size; neither is consulted until `App::run`
    // installs the windowed runner, which this never calls.
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("dm1-stager-main-schedule", 64, 64));
    app.world_mut().insert_non_send_resource(Assets::<MeshGpu>::with_reserved(1));
    let mut materials = Assets::<Material>::with_reserved(16);
    let handles: Vec<Handle<Material>> = (0..BOOT_MATERIALS).map(|i| materials.add(mat(i as f32))).collect();
    app.insert_resource(materials);
    app.finish();

    // 1. Frame 0 drains the boot mints.
    frame(&mut app);
    assert_eq!(
        staged(&app),
        (BOOT_MATERIALS, vec![run(0, 0, BOOT_MATERIALS as u64)]),
        "frame 0 stages the boot mints as one run — a missing row means the stager is not in the \
         shipped Main schedule"
    );
    assert!(!app.world().resource::<Assets<Material>>().edited_any(), "the drain cleared every mark");

    // 2. Idle.
    frame(&mut app);
    assert_eq!(staged(&app), (0, vec![]), "an idle frame stages nothing, so the runner copies nothing");

    // 3. Edits to rows {2, 3, 4, 9}, made in a scrambled order.
    for &row in &[9usize, 3, 2, 4] {
        app.world_mut()
            .resource_mut::<Assets<Material>>()
            .get_mut(handles[row])
            .expect("invariant: a boot row is live")
            .gpu
            .emissive = [100.0 + row as f32; 4];
    }
    frame(&mut app);
    assert_eq!(staged(&app), (4, vec![run(0, 2, 3), run(3, 9, 1)]));
    let emissive: Vec<f32> = app
        .world()
        .resource::<MaterialUploadStaging>()
        .rows()
        .iter()
        .map(|r| r.emissive[0])
        .collect();
    assert_eq!(emissive, vec![102.0, 103.0, 104.0, 109.0], "the rows pack in ascending row order");

    // 4. A freed row stages zeros.
    app.world_mut().resource_mut::<Assets<Material>>().remove(handles[5]).expect("invariant: row 5 is live");
    frame(&mut app);
    assert_eq!(staged(&app), (1, vec![run(0, 5, 1)]));
    assert_eq!(
        app.world().resource::<MaterialUploadStaging>().rows()[0],
        MaterialGpu { base_color: [0.0; 4], mrr: [0.0; 4], emissive: [0.0; 4] },
        "a freed row stages the zero row a full image writes for it"
    );

    // 5. The next idle frame clears the staging.
    frame(&mut app);
    assert_eq!(staged(&app), (0, vec![]), "an idle frame clears what the edit frame staged");
}
