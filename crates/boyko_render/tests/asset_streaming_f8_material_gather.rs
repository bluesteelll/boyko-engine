//! Asset-streaming plan F8 — integration coverage for the REAL per-instance material
//! clamp inside [`gather_mesh_draws`] (the OOB `raw >= material_high_water -> 0` safety
//! net, F8 §4.2) and the two-pass (count + scatter) `iter_input()` determinism the
//! unified gather depends on (F8 §4.3, finding 11).
//!
//! # Why an ECS-level test, not `mesh_draw.rs`'s hand-tuple idiom
//!
//! `mesh_draw.rs`'s own unit tests call [`MeshRenderScratch::gather_mixed_into`]
//! directly with hand-built `(mesh_id, &InstanceModelCol, Option<&GpuTransform3D>, u32)`
//! tuples — but the OOB clamp itself (`let id = if raw >= material_high_water { 0 } else
//! { raw };`) is NOT inside `gather_mixed_into`; it lives in `gather_mesh_draws`'s query-map
//! closure (mesh_draw.rs, both the hwrt and non-hwrt variants), which reads a REAL
//! `Res<Assets<MaterialGpu>>` and a REAL `Query<..., Option<&MaterialHandle>>`. A hand-tuple
//! unit test can only feed `gather_mixed_into` an ALREADY-clamped id — it cannot exercise the
//! clamp comparison itself, nor the two SEPARATE `q.iter()` invocations (`bucket_lanes_mixed`'s
//! count pass, `gather_mixed_into`'s scatter pass) a real ECS `Query` performs (a hand-built
//! `&[T]` slice's `.iter()` is trivially stable across two calls; a real `Query` iterating
//! live archetype storage is the thing finding 11 actually worries about). This file therefore
//! runs the REAL `gather_mesh_draws` system through an `EcsMaster`, mirroring
//! `asset_streaming_f5_validation.rs`'s harness shape (a raw `EcsMaster` + direct
//! `run_system` calls, a device-inert `dummy_mesh_gpu` — see that file's module doc for why a
//! `VkBuffer::NULL`-handled `MeshGpu` is sound: `gather_mesh_draws` only reads
//! `index_count`/`index_type` off the mesh row, never a device handle).

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;

use boyko_render::{MaterialGpu, MeshBundle, MeshGpu, MeshRenderScratch, gather_mesh_draws};
use boyko_scene::Transform;
use boyko_scene::render_caps::{MaterialHandle, MeshHandle, RenderEnabled};

use boyko_math::Vec3;
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;

/// A device-inert `MeshGpu` — see this file's module doc for why a `VkBuffer::NULL`-handled
/// dummy is sound here (mirrors `asset_streaming_f5_validation.rs::dummy_mesh_gpu`).
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer { buffer: VkBuffer::NULL, offset: 0, size: 0, mapped: None };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 36,
        index_type: IndexType::Uint16,
        vertex_count: 8,
        #[cfg(feature = "hwrt")]
        blas: None,
    }
}

/// Builds an `EcsMaster` with just enough state for `gather_mesh_draws` to run: the mesh +
/// material asset tables (caller-seeded) and a fresh `MeshRenderScratch`. Deliberately
/// omits `RefcountDeltas`/`DeferredFree`/`ValidateCursor`/`RenderEpoch` — this suite never
/// runs `apply_refcount_deltas`/`validate_asset_refs`, and the F2 hooks' `dm.resource_mut::
/// <RefcountDeltas>()` probe is `Option`-safe (a graceful no-op) when that resource is
/// absent, so omitting it is sound (mirrors `asset_streaming_f5_validation.rs`'s
/// `world_with`, minus the refcount-pipeline resources this suite does not exercise).
fn world_with(mesh_assets: Assets<MeshGpu>, material_assets: Assets<MaterialGpu>) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    ecs.insert_non_send_resource(mesh_assets);
    ecs.insert_resource(material_assets);
    ecs.insert_resource(MeshRenderScratch::default());
    // The hwrt `gather_mesh_draws` variant additionally reads `Res<ShadowDenoiseConfig>`;
    // harmless to insert unconditionally-under-cfg (a no-op line on the default build, the
    // import itself is cfg-gated out — mirrors `asset_streaming_f7_grow_headless.rs`'s idiom).
    #[cfg(feature = "hwrt")]
    ecs.insert_resource(boyko_render::ShadowDenoiseConfig::default());
    // Prime both ids (installs the on_insert/on_replace hooks) before any spawn — the
    // established idiom (see phase14a_hooks_firing.rs's T2/T7 tests / asset_streaming_f5_
    // validation.rs's world_with).
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    ecs
}

fn spawn_drawable(ecs: &mut EcsMaster, mesh_id: u32, x: f32, material_raw: u16) {
    ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(MeshBundle::new(MeshHandle(mesh_id), Transform::from_translation(Vec3::new(x, 0.0, 0.0))))
            .insert(MaterialHandle(material_raw))
            .enable::<RenderEnabled>();
    });
}

/// F8 §4.2 / §7c: a raw `MaterialHandle` slot `>= material_high_water` (a garbage /
/// never-minted handle) must clamp to material id `0` — the PINNED, actually-minted default
/// slot (never a zeroed hole) — while an IN-BOUNDS non-default handle keeps its real id and
/// still flips [`MeshRenderScratch::any_non_default_material`].
#[test]
fn oob_material_clamps_to_zero() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh = mesh_assets.add(dummy_mesh_gpu());
    let mesh_idx = mesh.index();

    let mut material_assets = Assets::<MaterialGpu>::with_reserved(4);
    let default_slot = material_assets.add(MaterialGpu::default());
    assert_eq!(default_slot.index(), 0, "test precondition: the first mint is slot 0");
    let real_slot = material_assets.add(MaterialGpu::new(
        [0.1, 0.2, 0.3, 1.0],
        0.0,
        0.5,
        0.5,
        [0.0, 0.0, 0.0],
        0,
    ));
    assert_eq!(real_slot.index(), 1, "test precondition: the second mint is slot 1");
    // `high_water() == 2` at this point — any raw handle `>= 2` is OOB.

    let mut ecs = world_with(mesh_assets, material_assets);

    // Three drawables, distinguished by a unique translation-x (mirrors mesh_draw.rs's own
    // `affine(mesh, ord)` idiom) so each one's ring slot can be identified regardless of the
    // real `Query`'s internal iteration order.
    spawn_drawable(&mut ecs, mesh_idx, 0.0, 0); // the explicit default (in-bounds).
    spawn_drawable(&mut ecs, mesh_idx, 1.0, 1); // in-bounds, non-default (slot 1).
    spawn_drawable(&mut ecs, mesh_idx, 2.0, 200); // OOB: raw 200 >= high_water 2.

    ecs.run_system(gather_mesh_draws);

    let scratch = ecs.resource::<MeshRenderScratch>();
    assert_eq!(scratch.instance_count(), 3, "all three drawables were gathered");

    let ring = scratch.ring.as_read_slice();
    let material_ids = scratch.material_ids.as_read_slice();
    let slot_of = |x: f32| {
        ring.iter()
            .position(|c| c.rows[0][3] == x)
            .unwrap_or_else(|| panic!("a drawable at x={x} must have a ring slot"))
    };

    assert_eq!(material_ids[slot_of(0.0)].id, 0, "the explicit default entity keeps id 0");
    assert_eq!(
        material_ids[slot_of(1.0)].id, 1,
        "the in-bounds non-default entity keeps its real id"
    );
    assert_eq!(
        material_ids[slot_of(2.0)].id, 0,
        "a raw material id >= material_high_water must clamp to the pinned default slot 0, \
         not scatter the garbage index 200 (F8 §4.2 OOB clamp)"
    );
    // F8+ (owner: material-drives-albedo-too): the CLAMPED id's base_color, not the
    // OOB entity's own (never-minted) slot's — the pinned default material's color.
    assert_eq!(
        material_ids[slot_of(2.0)].base_color, [0.8, 0.8, 0.8, 1.0],
        "an OOB-clamped instance's base_color must be the DEFAULT material's, not garbage"
    );
    assert_eq!(
        material_ids[slot_of(1.0)].base_color, [0.1, 0.2, 0.3, 1.0],
        "the in-bounds non-default entity's base_color is its OWN material's"
    );
    assert!(
        scratch.any_non_default_material(),
        "the in-bounds slot-1 entity must still flip the PM gate even though a DIFFERENT \
         entity's OOB material clamped to 0"
    );
}

/// F8 §4.3 finding 11: `gather_mesh_draws`'s clamp closure is invoked via TWO SEPARATE
/// `q.iter()` calls within one system run (`bucket_lanes_mixed`'s count pass,
/// `gather_mixed_into`'s scatter pass) — this proves that a REAL ECS `Query`'s row order is
/// stable across those two invocations (not merely a hand-built slice's, which is trivially
/// stable) by checking that EVERY drawable's own material id lands at ITS OWN ring slot
/// (identified by mesh id + a unique translation), never a sibling drawable's — a
/// cross-row mismatch here is exactly what a query-iteration-order divergence between the
/// two internal passes would produce.
#[test]
fn clamp_is_deterministic_across_both_passes() {
    let mut mesh_assets = Assets::<MeshGpu>::with_reserved(4);
    let mesh_a = mesh_assets.add(dummy_mesh_gpu()).index();
    let mesh_b = mesh_assets.add(dummy_mesh_gpu()).index();

    let mut material_assets = Assets::<MaterialGpu>::with_reserved(4);
    let _default = material_assets.add(MaterialGpu::default()); // slot 0
    let mat1 = material_assets
        .add(MaterialGpu::new([0.2, 0.2, 0.9, 1.0], 0.0, 0.4, 0.5, [0.0, 0.0, 0.0], 0))
        .index() as u16; // slot 1
    let mat2 = material_assets
        .add(MaterialGpu::new([0.9, 0.6, 0.1, 1.0], 1.0, 0.2, 0.5, [0.0, 0.0, 0.0], 0))
        .index() as u16; // slot 2

    let mut ecs = world_with(mesh_assets, material_assets);

    // Five drawables interleaved across the two meshes, each a DISTINCT
    // (translation-x, mesh, material) triple.
    let rows: [(u32, f32, u16); 5] = [
        (mesh_a, 0.0, 0),
        (mesh_b, 1.0, mat1),
        (mesh_a, 2.0, mat2),
        (mesh_b, 3.0, 0),
        (mesh_a, 4.0, mat1),
    ];
    for &(mesh_id, x, mat) in &rows {
        spawn_drawable(&mut ecs, mesh_id, x, mat);
    }

    ecs.run_system(gather_mesh_draws);

    let scratch = ecs.resource::<MeshRenderScratch>();
    assert_eq!(scratch.instance_count(), 5, "all five drawables were gathered");
    assert_eq!(scratch.material_ids.len(), scratch.ring.len(), "material_ids is parallel to ring");
    assert_eq!(scratch.mesh_ids.len(), scratch.ring.len(), "mesh_ids is parallel to ring");

    let ring = scratch.ring.as_read_slice();
    let material_ids = scratch.material_ids.as_read_slice();
    let mesh_ids = scratch.mesh_ids.as_read_slice();

    for &(mesh_id, x, mat) in &rows {
        let slot = ring
            .iter()
            .position(|c| c.rows[0][3] == x)
            .unwrap_or_else(|| panic!("a drawable at x={x} must have a ring slot"));
        assert_eq!(
            mesh_ids[slot], mesh_id,
            "x={x}: the mesh-id lane must match THIS row's own mesh, never a sibling row's \
             (real Query iteration-order stability across the two internal iter_input() passes)"
        );
        assert_eq!(
            material_ids[slot].id,
            u32::from(mat),
            "x={x}: the material-id lane must match THIS row's own material, never a sibling \
             row's (F8 finding 11: the count pass and the scatter pass must agree)"
        );
    }

    assert!(
        scratch.any_non_default_material(),
        "the mat1/mat2-bearing rows must flip the PM gate"
    );
}
