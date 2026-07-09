//! R3 regression witness for plan P0-3: a STRUCTURAL change (a mesh-entity
//! spawn) on a frame with ZERO fixed substeps (dt below the fixed timestep)
//! must still reach the instance ring — the gather output changes AND the
//! per-slot upload happens that same frame.
//!
//! With the D5 UNCONDITIONAL upload this is trivially true (the whole point:
//! correct by construction); the test stays as the tripwire for any future
//! gating that keys the upload off substep counts and would drop
//! structural-change-only frames.
//!
//! # Scoping (documented)
//!
//! Headless: `MeshRegistry` needs a live device, so the gather runs through a
//! test-local system over the SAME `(MeshHandle, InstanceModelCol)` +
//! `Enabled<RenderEnabled>` query shape `gather_mesh_draws` uses, with a fixed
//! meta table instead of the registry; the upload is the production
//! `upload_instance_models` over a fake mapped slot.

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_ecs::prelude::*;
use boyko_macros::Resource;
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::mesh_draw::MeshRenderScratch;
use boyko_render::{GpuTransform3D, MeshBundle, upload_instance_models};
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::swapchain::FrameWriteToken;
use boyko_scene::render_caps::{MeshHandle, RenderEnabled};
use boyko_scene::Transform;

use std::time::Duration;

/// Counts fixed substeps so the test can PROVE the spawn frame ran zero.
#[derive(Resource, Default)]
struct Substeps(u32);

fn count_substep(mut s: ResMut<Substeps>) {
    s.0 += 1;
}

/// The headless twin of `gather_mesh_draws`: the SAME unified query shape
/// (`Option<&GpuTransform3D>` keys static vs interpolated) + the same
/// count→prefix-sum→scatter core, with a fixed meta table (no GPU registry). The
/// test spawns only static meshes, so every row takes the `None` branch.
#[allow(clippy::needless_pass_by_value)]
fn gather_headless(
    q: Query<
        (&MeshHandle, &InstanceModelCol, Option<&GpuTransform3D>),
        Enabled<RenderEnabled>,
    >,
    mut scratch: ResMut<MeshRenderScratch>,
) {
    scratch.gather_mixed_into(
        1,
        |_mesh| (36u32, IndexType::Uint16),
        || q.iter().map(|(h, col, pair)| (h.0, col, pair)),
    );
}

#[test]
fn structural_spawn_on_zero_substep_frame_reaches_the_instance_ring() {
    // A dt safely below the default 64 Hz fixed timestep: the fixed schedule
    // accumulates but runs ZERO substeps on each of these frames.
    let below_timestep = Duration::from_millis(2);

    let mut app = App::new();
    app.insert_resource(Substeps(0));
    app.insert_resource(MeshRenderScratch::default());
    app.add_systems_in(CoreSchedule::Fixed, count_substep);
    app.add_systems(gather_headless);
    app.finish();

    // The fake mapped instance-ring slot the production upload writes into.
    let mut storage = vec![0u8; 16 * 48];
    let slot = BoundBuffer {
        buffer: VkBuffer::NULL,
        offset: 0,
        size: storage.len() as u64,
        mapped: core::ptr::NonNull::new(storage.as_mut_ptr()),
    };
    // SAFETY: no GPU work exists in this process (no device was booted), so
    // nothing submitted can reference the fake slot — the `forge_unfenced`
    // setup-seeding contract holds trivially.
    let token = unsafe { FrameWriteToken::forge_unfenced(0) };

    // ── Frame 1: one visible mesh entity; a 0-substep frame gathers it.
    // Spawn via `Commands` (the bundle-spawn surface; `run_system` flushes the
    // deferred queue before returning) + the `EnableTag` bit the gather
    // filters on — the render layer's documented attach-after-spawn contract.
    app.world_mut().run_system(|mut cmds: Commands| {
        cmds.spawn(MeshBundle::new(MeshHandle(0), Transform::IDENTITY))
            .enable::<RenderEnabled>();
    });
    app.update_with_delta(below_timestep);
    assert_eq!(
        app.world().resource::<Substeps>().0,
        0,
        "precondition: the first frame ran zero fixed substeps"
    );
    let ring_before = app.world().resource::<MeshRenderScratch>().ring.clone();
    assert_eq!(ring_before.len(), 1, "frame 1 gathered the first instance");
    // SAFETY: the fake slot's `mapped` points to the LIVE heap `storage` Vec of
    // exactly `size` bytes (outliving every upload), satisfying the memory
    // precondition; the token/slot contract holds trivially — no GPU work
    // exists in this process, so nothing reads the slot.
    unsafe {
        upload_instance_models(&token, &slot, app.world().resource::<MeshRenderScratch>());
    }

    // ── Frame 2: the STRUCTURAL change — spawn a second mesh entity — on
    // another 0-substep frame. The gather output must change and the
    // (unconditional) upload must land it in the slot the SAME frame. ────────
    app.world_mut().run_system(|mut cmds: Commands| {
        cmds.spawn(MeshBundle::new(
            MeshHandle(0),
            Transform::from_translation(boyko_math::Vec3::new(3.0, 0.0, 0.0)),
        ))
        .enable::<RenderEnabled>();
    });
    app.update_with_delta(below_timestep);
    assert_eq!(
        app.world().resource::<Substeps>().0,
        0,
        "precondition: the spawn frame ALSO ran zero fixed substeps"
    );

    let scratch = app.world().resource::<MeshRenderScratch>();
    assert_eq!(
        scratch.ring.len(),
        2,
        "the gather output changed on the structural-change frame"
    );
    assert_ne!(
        scratch.ring, ring_before,
        "the gathered ring differs from the pre-spawn frame"
    );

    // SAFETY: identical contract to the frame-1 upload above (the same live
    // fake slot, the same no-GPU token argument).
    unsafe {
        upload_instance_models(&token, &slot, scratch);
    }

    // The upload happened: the slot's leading bytes are the NEW two records
    // (the second one did not exist before the spawn), proving a 0-substep
    // structural-change frame re-uploads — the P0-3 regression witness.
    let expect: &[u8] = bytemuck::cast_slice(scratch.ring.as_slice());
    assert_eq!(
        &storage[..expect.len()],
        expect,
        "the instance ring re-uploaded on the 0-substep structural-change frame"
    );
}
