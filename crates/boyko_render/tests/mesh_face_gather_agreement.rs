//! **RD6 / D-DS — the camera gather and the caster gather give every row the same face class.**
//!
//! The production caster passes draw the CAMERA's batches, marking a batch as casting by
//! merge-walking it against the caster gather's batches on their shared key
//! ([`batch_casts_shadow`]). The key carries the face class, so the two gathers must derive the
//! same class for the same entity, or a casting batch silently loses its shadow:
//!
//! - **double-sided** comes from the material row through the SAME guarded resolution, including
//!   the stale-material substitution (a `MaterialStale` row draws — and therefore casts — with the
//!   pinned default material);
//! - **mirrored** comes from the instance's `InstanceModelCol` determinant on both sides — for an
//!   interpolated (dynamic) row too, whose `GpuTransform3D` the caster query does not read.
//!
//! The harness is `asset_streaming_f8_material_gather.rs`'s (a raw `EcsMaster`, a device-inert
//! `MeshGpu`, direct `run_system` calls of the two production gather systems).

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;

use boyko_macros::Bundle;
use boyko_math::Vec3;
use boyko_render::csm_caster::batch_casts_shadow;
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::GpuTransform3D;
use boyko_render::{
    CsmCasterScratch, Material, MaterialStale, MeshBundle, MeshGpu, MeshRenderScratch, ShadowCaster,
    gather_mesh_draws, gather_shadow_casters,
};
use boyko_rhi::enums::{IndexType, MESH_FACE_DOUBLE_SIDED, MESH_FACE_MIRRORED};
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_scene::render_caps::{MaterialHandle, MeshHandle, RenderEnabled, Visibility};
use boyko_scene::transform::GlobalTransform;
use boyko_scene::Transform;

/// A device-inert `MeshGpu` (the gathers read only `index_count` / `index_type`).
fn dummy_mesh_gpu() -> MeshGpu {
    let dummy_buf = || BoundBuffer { buffer: VkBuffer::NULL, offset: 0, size: 0, mapped: None, block: 0 };
    MeshGpu {
        vertex_buffer: dummy_buf(),
        index_buffer: dummy_buf(),
        index_count: 36,
        index_type: IndexType::Uint16,
        vertex_count: 8,
        #[cfg(feature = "hwrt")]
        blas: None,
        geometry_slot: 0,
        local_min: [-0.5; 3],
        local_max: [0.5; 3],
    }
}

/// Material slots: 0 the pinned default (single-sided), 1 double-sided, 2 single-sided.
fn materials() -> Assets<Material> {
    let mut m = Assets::<Material>::with_reserved(4);
    let default = m.add(Material::default());
    m.pin(default.index());
    let mut ds = Material::new([0.2, 0.6, 0.2, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0);
    ds.double_sided = true;
    m.add(ds);
    m.add(Material::new([0.6, 0.2, 0.2, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0));
    m
}

fn world() -> EcsMaster {
    let mut ecs = EcsMaster::new();
    let mut meshes = Assets::<MeshGpu>::with_reserved(2);
    meshes.add(dummy_mesh_gpu());
    ecs.insert_non_send_resource(meshes);
    ecs.insert_resource(materials());
    ecs.insert_resource(MeshRenderScratch::default());
    ecs.insert_resource(CsmCasterScratch::default());
    #[cfg(feature = "hwrt")]
    ecs.insert_resource(boyko_render::ShadowDenoiseConfig::default());
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    ecs
}

/// One drawable of mesh 0 at `x` with `material`, optionally a caster, stale, mirrored (scale
/// `(−1, 1, 1)`) and dynamic (a `GpuTransform3D` pair).
#[derive(Clone, Copy)]
struct Spawn {
    x: f32,
    material: u16,
    caster: bool,
    stale: bool,
    mirrored: bool,
    dynamic: bool,
}

/// [`MeshBundle`]'s fields plus the dense interpolation pair — an interpolated drawable (dense
/// components join through a derived bundle, not a later `insert`).
#[derive(Bundle)]
struct DynamicMeshBundle {
    transform: Transform,
    global: GlobalTransform,
    mesh: MeshHandle,
    material: MaterialHandle,
    visibility: Visibility,
    instance: InstanceModelCol,
    pair: GpuTransform3D,
}

fn spawn(ecs: &mut EcsMaster, s: Spawn) {
    ecs.run_system(move |mut cmds: Commands| {
        let mut t = Transform::from_translation(Vec3::new(s.x, 0.0, 0.0));
        if s.mirrored {
            t.scale = Vec3::new(-1.0, 1.0, 1.0);
        }
        let mut bundle = MeshBundle::new(MeshHandle(0), t);
        bundle.material = MaterialHandle(s.material);
        let mut e = if s.dynamic {
            cmds.spawn(DynamicMeshBundle {
                transform: bundle.transform,
                global: bundle.global,
                mesh: bundle.mesh,
                material: bundle.material,
                visibility: bundle.visibility,
                instance: bundle.instance,
                pair: GpuTransform3D::from_transform(&t),
            })
        } else {
            cmds.spawn(bundle)
        };
        e.enable::<RenderEnabled>();
        if s.caster {
            e.insert(ShadowCaster);
        }
        if s.stale {
            e.enable::<MaterialStale>();
        }
    });
}

/// The camera batches as `(face, instance_count, casts_shadow)`, the casting bit from the walk the
/// runner performs.
fn camera_batches_with_casting(ecs: &mut EcsMaster) -> Vec<(u32, u32, bool)> {
    ecs.run_system(gather_mesh_draws);
    ecs.run_system(gather_shadow_casters);
    let camera: Vec<_> = ecs.resource::<MeshRenderScratch>().batches.as_read_slice().to_vec();
    let casters: Vec<_> = ecs.resource::<CsmCasterScratch>().0.batches.as_read_slice().to_vec();
    let mut cursor = 0usize;
    camera.iter().map(|b| (b.face, b.instance_count, batch_casts_shadow(&mut cursor, &casters, b))).collect()
}

/// A double-sided caster, a stale-material caster whose material is double-sided (it draws
/// single-sided, with the default), a single-sided receiver, and a mirrored dynamic caster: the
/// camera gather splits them into their three classes, and every batch holding a caster is
/// marked casting — the caster gather keyed each caster row exactly as the camera gather did.
#[test]
fn camera_and_caster_gathers_agree_on_every_rows_face_class() {
    let mut ecs = world();
    let base = Spawn { x: 0.0, material: 2, caster: false, stale: false, mirrored: false, dynamic: false };
    spawn(&mut ecs, Spawn { x: 0.0, material: 1, caster: true, ..base });
    spawn(&mut ecs, Spawn { x: 2.0, ..base });
    spawn(&mut ecs, Spawn { x: 4.0, material: 1, caster: true, stale: true, ..base });
    spawn(&mut ecs, Spawn { x: 6.0, caster: true, mirrored: true, dynamic: true, ..base });

    let got = camera_batches_with_casting(&mut ecs);
    assert_eq!(
        got,
        vec![(0, 2, true), (MESH_FACE_DOUBLE_SIDED, 1, true), (MESH_FACE_MIRRORED, 1, true)],
        "RED: (face, instance_count, casts_shadow) per camera batch — the single-sided receiver \
         and the stale row share class 0 (the stale row casts), the double-sided caster and the \
         mirrored dynamic caster get their own classes and cast"
    );
}

/// The control: with no double-sided material and no mirror, both gathers stay mesh-only — one
/// class-0 batch, casting.
#[test]
fn class_zero_scene_is_one_casting_batch() {
    let mut ecs = world();
    let base = Spawn { x: 0.0, material: 2, caster: true, stale: false, mirrored: false, dynamic: false };
    spawn(&mut ecs, base);
    spawn(&mut ecs, Spawn { x: 2.0, caster: false, ..base });
    assert_eq!(camera_batches_with_casting(&mut ecs), vec![(0, 2, true)]);
}
