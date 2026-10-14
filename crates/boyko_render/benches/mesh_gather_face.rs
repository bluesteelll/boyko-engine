//! RD6 / D-DS budget row: the main-thread cost of the production mesh gather
//! ([`gather_mesh_draws`]) before and after the face class joins its bucket key.
//!
//! The scene is the common case the class bit is almost always 0 for: N drawables over 64
//! meshes, every one on the pinned default material, no mirror — exactly every pin's shape.
//! The system runs as the host runs it (a pre-built, cached `FunctionSystem` over a real
//! `EcsMaster` query, `run_cached_system` per iteration), so the per-row closure, the material
//! resolution and the count / scatter / textured passes are all in the measured loop.
//!
//! The cut's budget (and its derivation) lives in `D:/tmp/render-orch/rd2-rd6/cut.md`'s addendum
//! (W4): the ceiling on the all-default 10 k row is +35 % over the parent commit's build, a
//! number derived from the per-row byte model, not a placeholder. Timing verdicts are window
//! rows (the orchestrator's), never this file's.
//!
//! Run: `cargo bench -p boyko-render --bench mesh_gather_face`

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::{Commands, IntoSystem};
use boyko_math::Vec3;
use boyko_render::{Material, MeshBundle, MeshGpu, MeshRenderScratch, gather_mesh_draws};
use boyko_rhi::enums::IndexType;
use boyko_rhi_vulkan::ffi::VkBuffer;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_scene::Transform;
use boyko_scene::render_caps::{MaterialHandle, MeshHandle, RenderEnabled};

/// Distinct meshes the drawables spread over (each batch holds N / 64 instances).
const MESHES: u32 = 64;

/// A device-inert `MeshGpu`: the gather reads only `index_count` / `index_type`.
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

/// N all-default drawables over [`MESHES`] meshes, interleaved so the scatter does real work.
fn world(n: u32) -> EcsMaster {
    let mut ecs = EcsMaster::new();
    let mut meshes = Assets::<MeshGpu>::with_reserved(MESHES as usize);
    for _ in 0..MESHES {
        meshes.add(dummy_mesh_gpu());
    }
    ecs.insert_non_send_resource(meshes);
    let mut materials = Assets::<Material>::with_reserved(1);
    let default = materials.add(Material::default());
    materials.pin(default.index());
    ecs.insert_resource(materials);
    ecs.insert_resource(MeshRenderScratch::default());
    #[cfg(feature = "hwrt")]
    ecs.insert_resource(boyko_render::ShadowDenoiseConfig::default());
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    ecs.run_system(move |mut cmds: Commands| {
        for i in 0..n {
            let t = Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0));
            cmds.spawn(MeshBundle::new(MeshHandle(i % MESHES), t)).enable::<RenderEnabled>();
        }
    });
    ecs
}

fn bench_gather(c: &mut Criterion) {
    let mut group = c.benchmark_group("mesh_gather_face");
    for &n in &[1_000u32, 10_000, 100_000] {
        let mut ecs = world(n);
        let mut system = IntoSystem::into_system(gather_mesh_draws);
        // Warm: the first run initialises the system state and commits the scratch pages.
        ecs.run_cached_system(&mut system);
        assert_eq!(ecs.resource::<MeshRenderScratch>().instance_count(), n as usize, "the gather saw every row");
        group.bench_with_input(BenchmarkId::new("all_default", n), &n, |b, _| {
            b.iter(|| ecs.run_cached_system(&mut system));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_gather);
criterion_main!(benches);
