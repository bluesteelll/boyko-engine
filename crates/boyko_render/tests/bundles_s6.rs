//! std-lib S6 gate suite (boyko_render half) — the three light-object bundle
//! presets `DirectionalLightObject` / `PointLightObject` / `SpotLightObject`
//! (3 concrete named bundles, not a generic — the derive rejects generics).
//!
//! These live in `boyko_render` (not `boyko_scene`) because they name this crate's
//! light components; the layering is cycle-free (`boyko_render -> boyko_scene`).
//! Gates covered:
//!
//! * EXACT component set — each light-object bundle spawns precisely its three
//!   declared columns (`Transform` + `GlobalTransform` + the light), membership
//!   being the characteristic function of `component_ids()` over `[0, MAX)`.
//! * WARM-PATH cache — repeated spawns hit the Phase-8.5 per-impl static bundle
//!   cache (idempotent `bundle_archetype_id_for`, ZERO new archetypes per spawn).
//! * 0%-GATE — a bundle spawn lands in the SAME `ArchetypeId` as the equivalent
//!   manual multi-insert with the identical component set.
//!
//! The cross-crate `DynamicBody`-falls + `Gpu3dInstance`-packs INTEGRATION gate
//! (which additionally names `boyko_physics`) lives in the sibling
//! `bundles_s6_integration.rs`.

// Test harness, not an engine path: `Arc<Mutex<Option<Entity>>>` is the established probe that
// carries the spawned `Entity` out of a one-shot `run_system` closure back to the assertions.
// Test-only scaffolding on the harness thread, never linked into a shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

use boyko_render::bundles::{DirectionalLightObject, PointLightObject, SpotLightObject};
use boyko_render::light::{DirectionalLight, PointLight, SpotLight};

use boyko_scene::transform::{GlobalTransform, Transform};

/// The kernel's component-id ceiling (mirror of the crate-private
/// `component_registry::MAX_COMPONENTS`). The exact-set walk scans `[0, MAX)`.
const MAX_COMPONENTS: usize = 512;

// ── helpers ─────────────────────────────────────────────────────────────────

/// Views a `#[repr(C)]` POD as raw bytes for the manual `create_entity` path.
///
/// # Safety
/// `T` is a `#[repr(C)]` component whose byte image is a valid serialization for
/// its pool (holds for every component spawned here — all `#[repr(C)]`).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we read its `size_of::<T>()` bytes read-only.
    // `T` is `#[repr(C)]`, matching the pool's stored layout; the slice borrows
    // `value` so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Asserts `entity` carries EXACTLY the components in `expected` — every declared
/// id present AND no other registered id (over `[0, MAX)`) present.
fn assert_exact_component_set(
    world: &EcsMaster,
    entity: Entity,
    expected: &[ComponentId],
    ctx: &str,
) {
    for id in expected {
        assert!(
            world.has_component(entity, *id),
            "{ctx}: bundle entity must carry its declared component {:?}",
            id
        );
    }
    let mut extra: Vec<usize> = Vec::new();
    for raw in 0..MAX_COMPONENTS {
        let id = ComponentId(raw);
        if world.has_component(entity, id) && !expected.contains(&id) {
            extra.push(raw);
        }
    }
    assert!(
        extra.is_empty(),
        "{ctx}: bundle entity carries UNEXPECTED components {:?} (declared set {:?})",
        extra,
        expected
    );
}

/// Spawns `make()`'s bundle through `Commands::spawn` and returns its `Entity`.
fn spawn_bundle<B, F>(world: &mut EcsMaster, make: F) -> Entity
where
    B: Bundle,
    F: Fn() -> B + Send + Sync + 'static,
{
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(make()).id();
        *probe.lock().expect("probe") = Some(e);
    });
    sink.lock().expect("probe").expect("spawn handle")
}

fn a_dir_light() -> DirectionalLight {
    DirectionalLight::new([0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 10.0)
}
fn a_point_light() -> PointLight {
    PointLight::new([1.0, 2.0, 3.0], [1.0, 1.0, 1.0], 50.0, 5.0)
}
fn a_spot_light() -> SpotLight {
    SpotLight::new(
        [1.0, 2.0, 3.0],
        [0.0, 0.0, -1.0],
        [1.0, 1.0, 1.0],
        100.0,
        8.0,
        15.0,
        30.0,
    )
}

fn a_dir_object() -> DirectionalLightObject {
    DirectionalLightObject {
        transform: Transform::IDENTITY,
        global: GlobalTransform::default(),
        light: a_dir_light(),
    }
}
fn a_point_object() -> PointLightObject {
    PointLightObject {
        transform: Transform::IDENTITY,
        global: GlobalTransform::default(),
        light: a_point_light(),
    }
}
fn a_spot_object() -> SpotLightObject {
    SpotLightObject {
        transform: Transform::IDENTITY,
        global: GlobalTransform::default(),
        light: a_spot_light(),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — EXACT component set per light-object bundle
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn directional_light_object_spawns_exactly_its_three_components() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, a_dir_object);
    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        DirectionalLight::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "DirectionalLightObject");
    assert_eq!(DirectionalLightObject::component_ids().len(), 3, "arity 3");
    // It carries NO point/spot light component.
    assert!(!world.has_component(e, PointLight::component_id()));
    assert!(!world.has_component(e, SpotLight::component_id()));
}

#[test]
fn point_light_object_spawns_exactly_its_three_components() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, a_point_object);
    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        PointLight::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "PointLightObject");
    assert_eq!(PointLightObject::component_ids().len(), 3, "arity 3");
}

#[test]
fn spot_light_object_spawns_exactly_its_three_components() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, a_spot_object);
    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        SpotLight::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "SpotLightObject");
    assert_eq!(SpotLightObject::component_ids().len(), 3, "arity 3");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — distinct bundles + WARM-PATH cache
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn three_light_objects_are_distinct_bundle_types() {
    // Three CONCRETE bundles (the derive rejects a generic LightObject<L>): each
    // owns its own BundleTypeId + static cache slot.
    let d = DirectionalLightObject::bundle_type_id();
    let p = PointLightObject::bundle_type_id();
    let s = SpotLightObject::bundle_type_id();
    assert_ne!(d, p, "DirectionalLightObject vs PointLightObject distinct");
    assert_ne!(p, s, "PointLightObject vs SpotLightObject distinct");
    assert_ne!(d, s, "DirectionalLightObject vs SpotLightObject distinct");

    let a = SpotLightObject::static_info();
    let b = SpotLightObject::static_info();
    assert!(std::ptr::eq(a, b), "SpotLightObject::static_info is a stable cache pointer");
}

#[test]
fn point_light_object_warm_spawn_hits_static_cache_no_rebuild() {
    let mut world = EcsMaster::new();

    let cold = world.bundle_archetype_id_for::<PointLightObject>();
    let warm = world.bundle_archetype_id_for::<PointLightObject>();
    assert_eq!(cold, warm, "bundle_archetype_id_for idempotent (warm cache hit)");

    let before = world.archetype_count();
    for _ in 0..20u32 {
        spawn_bundle(&mut world, a_point_object);
    }
    let after = world.archetype_count();
    assert_eq!(
        before, after,
        "20 repeated PointLightObject spawns created ZERO new archetypes (no per-spawn rebuild)"
    );
    assert_eq!(world.entity_count(), 20, "all 20 entities spawned");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — 0%-GATE: bundle spawn == equivalent manual multi-insert (same archetype)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn directional_light_object_matches_manual_insert_archetype() {
    let mut world = EcsMaster::new();

    let arch = world.create_archetype(&[
        Transform::component_id(),
        GlobalTransform::component_id(),
        DirectionalLight::component_id(),
    ]);
    let t = Transform::IDENTITY;
    let g = GlobalTransform::default();
    let l = a_dir_light();
    let manual = world
        .create_entity(
            arch,
            &[
                (Transform::component_id(), as_bytes(&t)),
                (GlobalTransform::component_id(), as_bytes(&g)),
                (DirectionalLight::component_id(), as_bytes(&l)),
            ],
        )
        .expect("manual dir-light archetype accepts its three columns");

    let bundle_arch = world.bundle_archetype_id_for::<DirectionalLightObject>();
    assert_eq!(
        bundle_arch, arch,
        "0%-gate: DirectionalLightObject resolves to the SAME archetype as the manual insert"
    );

    let spawned = spawn_bundle(&mut world, a_dir_object);
    assert_eq!(
        world.get_entity_archetype_id(spawned),
        Some(arch),
        "0%-gate: the bundle-spawned light shares the hand-built archetype"
    );
    assert_eq!(world.get_entity_archetype_id(manual), Some(arch), "manual entity sanity");
}
