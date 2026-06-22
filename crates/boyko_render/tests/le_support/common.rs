//! Shared helpers for the `LightEnabled` CPU gate suite (one helper module, included
//! by each single-test binary via `#[path]`).
//!
//! # Single-test-binary isolation (W1, Decision 3C)
//!
//! `register_component_hooks` panics `AlreadyArchetyped` if the light component was
//! EVER archetyped in the process before registration, and `was_ever_archetyped` is
//! process-global and never reset. `LightingPlugin::build` registers the four eviction
//! hooks as its first action, so:
//!
//! * a second `LightingPlugin::build` in the same process panics (the type is already
//!   archetyped by the first test's spawn), and
//! * a test that spawns a light before adding `LightingPlugin` poisons the global for
//!   every later test in that binary.
//!
//! Therefore each gate test lives in its OWN `tests/*.rs` file (cargo runs each as a
//! separate process) and contains exactly ONE `#[test]` that adds `LightingPlugin`
//! and then spawns lights. This file is NOT a test binary itself — it is `#[path]`-
//! included so the helpers are shared without re-running them.

#![allow(dead_code)]

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_render::light::{
    DirectionalLight, LightHeaderGpu, LightingConfig, PointLight, SkyLight, SpotLight,
};
use boyko_render::light_plugin::LightingPlugin;
use boyko_render::light_system::{LightTableStaging, LIGHT_HEADER_BYTES};

use boyko_scene::transform::{GlobalTransform, Transform};

/// Views a `#[repr(C)]` POD as raw bytes for the `create_entity` spawn path.
///
/// # Safety
/// `T` is a `#[repr(C)]` component whose byte image is a valid serialization for its
/// pool (holds for every component spawned here — all fixed-layout PODs).
pub fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we read its `size_of::<T>()` bytes read-only. `T`
    // is `#[repr(C)]`, matching the pool's stored layout; the slice borrows `value`.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Builds an `App` with `LightingPlugin` (registers eviction hooks FIRST, before any
/// light is archetyped) and the `LightTableStaging` staging resource.
///
/// The caller MUST NOT spawn any light before this returns — see the module-level
/// isolation note.
pub fn lighting_app() -> App {
    let mut app = App::new();
    // LightingPlugin's build registers the on_remove hooks before anything else, so it
    // must run before the first light spawn. The staging resource is the light-table
    // sink collect_lights writes into; LightingConfig is the Res<_> collect_lights folds
    // (production wires it via the render setup — the test provides the default anchor).
    app.insert_resource(LightTableStaging::default());
    app.insert_resource(LightingConfig::default());
    app.add_plugins(LightingPlugin);
    app
}

/// Reads `LightHeaderGpu` back out of the staging bytes.
pub fn read_header(bytes: &[u8]) -> LightHeaderGpu {
    assert!(bytes.len() >= LIGHT_HEADER_BYTES, "staging holds at least one header");
    // SAFETY: `bytes` is sized for a header (asserted) and the source is a
    // `repr(C, align(16))` POD written via the staging `write_pod`; read it back
    // unaligned-safe through a byte copy into a typed slot.
    let mut h = core::mem::MaybeUninit::<LightHeaderGpu>::uninit();
    unsafe {
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            h.as_mut_ptr().cast::<u8>(),
            LIGHT_HEADER_BYTES,
        );
        h.assume_init()
    }
}

/// The current light table's total `light_count` (from the staging header).
pub fn light_count(app: &App) -> u32 {
    let staging = app.world().resource::<LightTableStaging>();
    read_header(staging.bytes()).light_count()
}

/// The current table's `point_spot_count` (the L0b block).
pub fn point_spot_count(app: &App) -> u32 {
    let staging = app.world().resource::<LightTableStaging>();
    read_header(staging.bytes()).point_spot_count()
}

/// The current table's `l0a_count` (directionals + sky, the no-`P` front block).
pub fn l0a_count(app: &App) -> u32 {
    let staging = app.world().resource::<LightTableStaging>();
    read_header(staging.bytes()).l0a_count()
}

/// Spawns a directional light (with its required `Transform` / `GlobalTransform`) WITHOUT
/// touching `LightEnabled` — back-compat path (the seed enables it). Returns its handle.
pub fn spawn_dir_light(world: &mut EcsMaster, dir: [f32; 3]) -> Entity {
    let arch = world.create_archetype(&[
        DirectionalLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let light = DirectionalLight { direction: dir, color: [1.0, 1.0, 1.0], illuminance: 1.0 };
    world
        .create_entity(
            arch,
            &[
                (DirectionalLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&GlobalTransform::IDENTITY)),
            ],
        )
        .expect("dir-light archetype accepts its three columns")
}

/// Spawns a point light (with its required pose pair) WITHOUT touching `LightEnabled`.
pub fn spawn_point_light(world: &mut EcsMaster, pos: [f32; 3]) -> Entity {
    let arch = world.create_archetype(&[
        PointLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let light = PointLight { position: pos, color: [1.0, 1.0, 1.0], power: 50.0, range: 5.0 };
    world
        .create_entity(
            arch,
            &[
                (PointLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&GlobalTransform::IDENTITY)),
            ],
        )
        .expect("point-light archetype accepts its three columns")
}

/// Spawns a spot light (with its required pose pair) WITHOUT touching `LightEnabled`.
pub fn spawn_spot_light(world: &mut EcsMaster) -> Entity {
    let arch = world.create_archetype(&[
        SpotLight::component_id(),
        Transform::component_id(),
        GlobalTransform::component_id(),
    ]);
    let light = SpotLight {
        position: [0.0, 0.0, 0.0],
        direction: [0.0, 0.0, 1.0],
        color: [1.0, 1.0, 1.0],
        power: 100.0,
        range: 5.0,
        inner_deg: 15.0,
        outer_deg: 30.0,
    };
    world
        .create_entity(
            arch,
            &[
                (SpotLight::component_id(), as_bytes(&light)),
                (Transform::component_id(), as_bytes(&Transform::IDENTITY)),
                (GlobalTransform::component_id(), as_bytes(&GlobalTransform::IDENTITY)),
            ],
        )
        .expect("spot-light archetype accepts its three columns")
}

/// Spawns a sky light (no pose require) WITHOUT touching `LightEnabled`.
pub fn spawn_sky_light(world: &mut EcsMaster) -> Entity {
    let arch = world.create_archetype(&[SkyLight::component_id()]);
    let light = SkyLight::new([0.2, 0.3, 0.4], [0.05, 0.06, 0.07]);
    world
        .create_entity(arch, &[(SkyLight::component_id(), as_bytes(&light))])
        .expect("sky-light archetype accepts its one column")
}
