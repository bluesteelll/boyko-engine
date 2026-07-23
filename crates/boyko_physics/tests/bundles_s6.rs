//! std-lib S6 gate suite (boyko_physics half) — the physics object-category
//! bundle presets `DynamicBody` (arity 8) and `Trigger` (arity 4).
//!
//! These live in `boyko_physics` (not `boyko_scene`) because they name this
//! crate's physics columns; the layering is cycle-free (`boyko_physics ->
//! boyko_scene`, never the reverse). Gates covered here (no render crate needed):
//!
//! * EXACT component set — each bundle spawns precisely its declared columns PLUS the
//!   transitive `#[require]` closure those columns pull in, and nothing else (membership is
//!   checked over the whole `[0, MAX_COMPONENTS)` id space, so an unexpected extra is caught
//!   wherever its id landed). The closure is spelled out per test rather than derived: the
//!   kernel's `get_required_plan` is `pub(crate)`, and naming the expected carriers keeps the
//!   test able to FAIL when a new `#[require]` edge appears unannounced.
//! * WARM-PATH cache — repeated spawns hit the Phase-8.5 per-impl static bundle
//!   cache (idempotent `bundle_archetype_id_for`, ZERO new archetypes per spawn).
//! * 0%-GATE — a bundle spawn lands in the SAME `ArchetypeId` as the equivalent
//!   manual multi-insert with the identical component set (no extra migration).
//! * ARITY — DynamicBody is 8 (<= MAX_BUNDLE_ARITY 16); Trigger is 4.
//!
//! The cross-crate INTEGRATION gate (a `DynamicBody` falls under physics AND its
//! `Gpu3dInstance` packs at the new world pose) needs `boyko_render` too, so it
//! lives in `boyko_render/tests/bundles_s6_integration.rs` (render dev-depends on
//! physics; physics never depends on render — acyclic).

// Test-only: `HashSet` is the ORACLE model here — the reference "no color reuses a
// dynamic body" / injectivity checker the constraint-graph coloring is differentially
// verified against — and `Arc<Mutex<…>>` is the established probe for smuggling a spawned
// `Entity` out of the `Send + Sync` one-shot system closure. The solver's own structures
// stay VM-native; this file is compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

use boyko_physics::bundles::{DynamicBody, Trigger};
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyMass, Sensor,
};
use boyko_physics::math::{Mat3, Quat, Vec3};

use boyko_scene::render_caps::{
    MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen, Visibility,
};
use boyko_scene::transform::{GlobalTransform, Transform};

/// The kernel's component-id ceiling (mirror of the crate-private
/// `component_registry::MAX_COMPONENTS`). The exact-set walk scans `[0, MAX)`.
const MAX_COMPONENTS: usize = 512;

/// The bundle-arity ceiling the derive enforces.
const MAX_BUNDLE_ARITY: usize = 16;

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
/// id present AND no other registered id (over the whole `[0, MAX)` space) present.
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

fn unit_sphere() -> Collider {
    Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    }
}

fn dynamic_mass() -> RigidBodyMass {
    RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.5,
        friction: 0.3,
    }
}

fn a_dynamic_body() -> DynamicBody {
    DynamicBody {
        transform: Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        global: GlobalTransform::default(),
        mesh: MeshHandle(11),
        material: MaterialHandle(5),
        body: RigidBody {
            position: Vec3::new(1.0, 2.0, 3.0),
            linear_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
        },
        mass: dynamic_mass(),
        collider: unit_sphere(),
        visibility: Visibility::Visible,
    }
}

fn a_trigger() -> Trigger {
    Trigger {
        transform: Transform::from_translation(Vec3::ZERO),
        global: GlobalTransform::default(),
        collider: unit_sphere(),
        sensor: Sensor,
    }
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

// ════════════════════════════════════════════════════════════════════════════
// Gate — EXACT component set + arity
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dynamic_body_spawns_its_declared_set_plus_required_closure() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, a_dynamic_body);

    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        MeshHandle::component_id(),
        MaterialHandle::component_id(),
        RigidBody::component_id(),
        RigidBodyMass::component_id(),
        Collider::component_id(),
        Visibility::component_id(),
        // The `#[require]` closure, NOT bundle fields. `MeshHandle` declares
        // `#[require(Transform, GlobalTransform, MeshRefGen)]` and `MaterialHandle`
        // `#[require(MaterialRefGen)]` (asset-streaming F5 generation carriers), so a spawn
        // legitimately materialises two columns the bundle never names. This test predates
        // those attributes and asserted an EXACT arity-8 set; it went red the moment the
        // 2026-07 audit fixed the vacuously-green CI and the suite actually ran again.
        // The invariant worth keeping is "nothing beyond the declared set AND its required
        // closure", so the closure is spelled out rather than the check weakened.
        MeshRefGen::component_id(),
        MaterialRefGen::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "DynamicBody");
    assert_eq!(DynamicBody::component_ids().len(), 8, "DynamicBody is arity 8");
    assert!(
        DynamicBody::component_ids().len() <= MAX_BUNDLE_ARITY,
        "DynamicBody arity 8 <= MAX_BUNDLE_ARITY 16"
    );
}

#[test]
fn trigger_spawns_exactly_its_four_components() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, a_trigger);

    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        Collider::component_id(),
        Sensor::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "Trigger");
    assert_eq!(Trigger::component_ids().len(), 4, "Trigger is arity 4");
    // A Trigger has NO RigidBody / RigidBodyMass (it is not integrated).
    assert!(
        !world.has_component(e, RigidBody::component_id()),
        "a Trigger carries no RigidBody (not integrated)"
    );
    assert!(
        !world.has_component(e, RigidBodyMass::component_id()),
        "a Trigger carries no RigidBodyMass"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — WARM-PATH: repeated spawn hits the static bundle cache (no rebuild)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dynamic_body_warm_spawn_hits_static_cache_no_rebuild() {
    let mut world = EcsMaster::new();

    let cold = world.bundle_archetype_id_for::<DynamicBody>();
    let warm = world.bundle_archetype_id_for::<DynamicBody>();
    assert_eq!(cold, warm, "bundle_archetype_id_for is idempotent (warm cache hit)");

    let before = world.archetype_count();
    for _ in 0..24u32 {
        spawn_bundle(&mut world, a_dynamic_body);
    }
    let after = world.archetype_count();
    assert_eq!(
        before, after,
        "24 repeated DynamicBody spawns created ZERO new archetypes (no per-spawn rebuild)"
    );
    assert_eq!(world.entity_count(), 24, "all 24 entities spawned");
    assert_eq!(
        world.bundle_archetype_id_for::<DynamicBody>(),
        cold,
        "the cached archetype id is stable after the spawn burst"
    );
}

#[test]
fn distinct_physics_bundles_have_distinct_type_ids() {
    assert_ne!(
        DynamicBody::bundle_type_id(),
        Trigger::bundle_type_id(),
        "DynamicBody and Trigger are distinct Bundle impls (distinct BundleTypeIds)"
    );
    // static_info() is a stable &'static cache pointer per impl.
    let a = DynamicBody::static_info();
    let b = DynamicBody::static_info();
    assert!(std::ptr::eq(a, b), "DynamicBody::static_info is a stable cache pointer");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — 0%-GATE: bundle spawn == equivalent manual multi-insert (same archetype)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dynamic_body_bundle_matches_manual_insert_archetype() {
    let mut world = EcsMaster::new();

    // Manual multi-insert into a hand-built archetype with the identical set.
    // The hand-built set must include the `#[require]` closure the bundle path materialises
    // (`MeshRefGen` via `MeshHandle`, `MaterialRefGen` via `MaterialHandle`) — otherwise the
    // two archetypes differ by two columns and the 0%-gate below compares unlike things.
    let arch = world.create_archetype(&[
        Transform::component_id(),
        GlobalTransform::component_id(),
        MeshHandle::component_id(),
        MaterialHandle::component_id(),
        RigidBody::component_id(),
        RigidBodyMass::component_id(),
        Collider::component_id(),
        Visibility::component_id(),
        MeshRefGen::component_id(),
        MaterialRefGen::component_id(),
    ]);
    let b = a_dynamic_body();
    // The require-ctors the bundle path would run: both carriers default to GEN_UNSYNCED.
    let mesh_ref_gen = MeshRefGen::default();
    let material_ref_gen = MaterialRefGen::default();
    let manual = world
        .create_entity(
            arch,
            &[
                (Transform::component_id(), as_bytes(&b.transform)),
                (GlobalTransform::component_id(), as_bytes(&b.global)),
                (MeshHandle::component_id(), as_bytes(&b.mesh)),
                (MaterialHandle::component_id(), as_bytes(&b.material)),
                (RigidBody::component_id(), as_bytes(&b.body)),
                (RigidBodyMass::component_id(), as_bytes(&b.mass)),
                (Collider::component_id(), as_bytes(&b.collider)),
                (Visibility::component_id(), as_bytes(&b.visibility)),
                (MeshRefGen::component_id(), as_bytes(&mesh_ref_gen)),
                (MaterialRefGen::component_id(), as_bytes(&material_ref_gen)),
            ],
        )
        .expect("manual DynamicBody archetype accepts its ten columns");

    let bundle_arch = world.bundle_archetype_id_for::<DynamicBody>();
    assert_eq!(
        bundle_arch, arch,
        "0%-gate: DynamicBody resolves to the SAME archetype as the manual multi-insert"
    );

    let spawned = spawn_bundle(&mut world, a_dynamic_body);
    assert_eq!(
        world.get_entity_archetype_id(spawned),
        Some(arch),
        "0%-gate: the bundle-spawned DynamicBody shares the manual entity's archetype"
    );
    assert_eq!(
        world.get_entity_archetype_id(manual),
        Some(arch),
        "manual entity is in the hand-built archetype (sanity)"
    );
}

#[test]
fn trigger_bundle_matches_manual_insert_archetype() {
    let mut world = EcsMaster::new();

    let arch: ArchetypeId = world.create_archetype(&[
        Transform::component_id(),
        GlobalTransform::component_id(),
        Collider::component_id(),
        Sensor::component_id(),
    ]);
    let bundle_arch = world.bundle_archetype_id_for::<Trigger>();
    assert_eq!(
        bundle_arch, arch,
        "0%-gate: Trigger resolves to the SAME archetype as the manual multi-insert"
    );

    let spawned = spawn_bundle(&mut world, a_trigger);
    assert_eq!(
        world.get_entity_archetype_id(spawned),
        Some(arch),
        "0%-gate: the bundle-spawned Trigger shares the hand-built archetype"
    );
}
