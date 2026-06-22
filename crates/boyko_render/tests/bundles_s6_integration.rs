//! std-lib S6 cross-crate INTEGRATION gate — a `DynamicBody` falls under physics
//! AND its `Gpu3dInstance` packs at the new world pose (full-pipeline smoke,
//! CPU-level — reads the packed column, no GPU needed).
//!
//! This is the one S6 gate that names BOTH `boyko_physics` (the `DynamicBody`
//! bundle + the fixed-step pipeline) and `boyko_render` (`Gpu3dInstance` +
//! `sync_gpu_3d_instances`). Neither domain crate depends on the other, so the
//! test lives here, where `boyko_render` dev-depends on `boyko_physics` (acyclic,
//! TEST-ONLY — see `boyko_render/Cargo.toml` `[dev-dependencies]`).
//!
//! The pipeline exercised, in order:
//!   1. spawn a `DynamicBody` bundle, then attach the render layer's
//!      `Gpu3dInstance` column + the `RenderEnabled` bit (the implementation
//!      documents these as render-layer attachments, NOT bundle fields);
//!   2. run the physics fixed step (gravity integrates the body down);
//!   3. `sync_body_to_transform` (inside the pipeline) mirrors the body pose into
//!      `Transform`, then `propagate_transforms` composes `GlobalTransform`;
//!   4. `sync_gpu_3d_instances` packs `GlobalTransform` → `Gpu3dInstance`.
//!
//! Assertions: (a) the `Transform.translation` actually fell, and (b) the
//! `Gpu3dInstance` column row's translation tracks the new world pose.

use std::sync::Arc;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::time::FixedTime;

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use boyko_physics::bundles::DynamicBody;
use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::plugin::add_physics_systems_with_scene_sync;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::solver::NoopSolver;

use boyko_render::gpu3d_instance::Gpu3dInstance;
use boyko_render::gpu3d_system::sync_gpu_3d_instances;

use boyko_scene::propagation::propagate_transforms;
use boyko_scene::render_caps::{MaterialHandle, MeshHandle, RenderEnabled, Visibility};
use boyko_scene::transform::{GlobalTransform, Transform};

use bytemuck::Zeroable;

/// A deterministic single-threaded pool.
fn serial_pool() -> Arc<ThreadPool> {
    ThreadPoolBuilder::new().num_threads(1).build()
}

/// The starting pose: high up so gravity has room to integrate downward.
const START: Vec3 = Vec3::new(0.0, 50.0, 0.0);

/// Builds a `DynamicBody` at [`START`] under unit dynamic mass + a unit sphere.
fn dynamic_body_at_start() -> DynamicBody {
    DynamicBody {
        transform: Transform::from_translation(START),
        global: GlobalTransform::default(),
        mesh: MeshHandle(1),
        material: MaterialHandle(0x00CD),
        body: RigidBody {
            position: START,
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        },
        mass: RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass: 1.0,
            restitution: 0.5,
            friction: 0.3,
        },
        collider: Collider {
            shape: ColliderShape::Sphere { radius: 0.5 },
            layer: 1,
            mask: 1,
        },
        visibility: Visibility::Visible,
    }
}

/// Spawns a `DynamicBody` bundle, then attaches the render layer's `Gpu3dInstance`
/// column (zeroed) + the per-frame `RenderEnabled` bit — exactly as the
/// implementation documents ("the render layer attaches them after spawning").
fn spawn_renderable_dynamic_body(world: &mut EcsMaster) -> Entity {
    let sink: Arc<std::sync::Mutex<Option<Entity>>> = Arc::new(std::sync::Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(dynamic_body_at_start())
            .insert(Gpu3dInstance::zeroed())
            .enable::<RenderEnabled>()
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    sink.lock().expect("probe").expect("dynamic-body spawn handle")
}

/// Wires the physics pipeline + S5 scene sync for the `NoopSolver` (so the
/// Foundation integrator runs and gravity moves the body), with a fixed `dt`.
fn build_pipeline(world: &mut EcsMaster, dt: f32) -> Schedule {
    let mut builder = ScheduleBuilder::new(serial_pool());
    let _ = add_physics_systems_with_scene_sync::<NoopSolver>(&mut builder, world);
    world.insert_resource(FixedTime::new(std::time::Duration::from_secs_f32(dt)));
    builder.build(world)
}

#[test]
fn dynamic_body_falls_and_gpu3d_instance_tracks_the_new_world_pose() {
    let mut world = EcsMaster::new();
    let entity = spawn_renderable_dynamic_body(&mut world);

    // Sanity: the spawned bundle carries its physics + render-cap columns AND the
    // render-attached Gpu3dInstance.
    assert!(world.has_component(entity, RigidBody::component_id()), "DynamicBody has RigidBody");
    assert!(world.has_component(entity, Transform::component_id()), "DynamicBody has Transform");
    assert!(world.has_component(entity, GlobalTransform::component_id()), "has GlobalTransform");
    assert!(world.has_component(entity, MaterialHandle::component_id()), "has MaterialHandle");
    assert!(
        world.has_component(entity, Gpu3dInstance::component_id()),
        "render layer attached the Gpu3dInstance column"
    );
    assert!(
        world.is_enabled::<RenderEnabled>(entity),
        "render layer enabled the RenderEnabled bit"
    );

    let dt = 1.0 / 64.0;
    let mut schedule = build_pipeline(&mut world, dt);
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(0.0, -10.0, 0.0);

    // Step the physics + scene-sync pipeline several frames so the body integrates
    // downward and `sync_body_to_transform` mirrors the pose into `Transform`.
    const FRAMES: u32 = 12;
    for _ in 0..FRAMES {
        schedule.run(&mut world);
    }

    // (a) The body fell, and the Transform tracks the integrated RigidBody pose.
    let rb = *world.get_component::<RigidBody>(entity).expect("body lives");
    let t = *world.get_component::<Transform>(entity).expect("transform lives");
    assert!(
        rb.position.y < START.y,
        "the DynamicBody fell under gravity: rb.y = {} (start {})",
        rb.position.y,
        START.y
    );
    assert_eq!(
        t.translation, rb.position,
        "Transform.translation bit-equals the integrated RigidBody.position (scene sync ran)"
    );

    // Compose GlobalTransform from the moved Transform, then pack it to the GPU
    // instance column.
    propagate_transforms(&mut world);
    let global = *world.get_component::<GlobalTransform>(entity).expect("global lives");
    assert_eq!(
        global.translation(),
        t.translation,
        "GlobalTransform tracks the synced Transform (root compose)"
    );

    world.run_system(sync_gpu_3d_instances);

    // (b) The Gpu3dInstance column row tracks the new WORLD pose.
    let inst = *world.get_component::<Gpu3dInstance>(entity).expect("gpu3d inst lives");
    assert_eq!(
        inst.translation,
        [global.translation().x, global.translation().y, global.translation().z],
        "Gpu3dInstance.translation packs the entity's NEW world translation (it fell)"
    );
    // And it is genuinely the moved pose, not the zeroed default it spawned with.
    assert_ne!(
        inst.translation,
        [0.0, 0.0, 0.0],
        "the packed instance is the integrated pose, not the spawn-time zero"
    );
    assert!(
        inst.translation[1] < START.y,
        "the packed Y tracks the fall: {} < {}",
        inst.translation[1],
        START.y
    );
    // The material lane carries the bundle's MaterialHandle (low 16 bits).
    assert_eq!(inst.material & 0xFFFF, 0x00CD, "packed material low-16 == the bundle's handle");
}
