//! std-lib S8 gate suite (boyko_render half) — `#[require(...)]` wiring on the
//! light components, and the SkyLight NEGATIVE gate.
//!
//! S8 is a THIN wiring phase: the `#[require(...)]` machinery is shipped + tested
//! in `boyko_ecs/tests/required_*.rs`. This suite pins ONLY the render-side wiring
//! contract (cross-crate: the lights `#[require(...)]` the `boyko_scene` pose pair):
//!
//! * each of `DirectionalLight` / `PointLight` / `SpotLight` spawned ALONE
//!   auto-inserts `Transform` + `GlobalTransform` — a positioned/oriented light
//!   always has a pose `light_reconcile` can read.
//! * `SkyLight` alone gains NO `Transform` / `GlobalTransform` (it is an
//!   environment term — NO require).
//! * a manually-supplied `Transform` is NOT clobbered/duplicated.
//! * COMPATIBILITY (0%-gate): a light's require expansion lands in the SAME
//!   `ArchetypeId` as the matching `DirectionalLightObject`-shaped `#[derive(Bundle)]`
//!   over `{Transform, GlobalTransform, <Light>}` — the same Phase-8.5 cached path.
//!
//! Spawns go through single-field `#[derive(Bundle)]`s via `Commands::spawn` so the
//! require funnel (`cold_register_bundle_archetype`) fires (the direct `spawn_one`
//! API bypasses it).

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle as DeriveBundle;

use boyko_render::light::{DirectionalLight, PointLight, SkyLight, SpotLight};
use boyko_scene::transform::{GlobalTransform, Transform};

use boyko_math::Vec3;

/// The kernel's component-id ceiling (mirror of the crate-private
/// `component_registry::MAX_COMPONENTS`). The exact-set walk scans `[0, MAX)`.
const MAX_COMPONENTS: usize = 512;

// ── single-field bundles that drive the require funnel ──────────────────────────

#[derive(DeriveBundle)]
struct DirOnly {
    light: DirectionalLight,
}

#[derive(DeriveBundle)]
struct PointOnly {
    light: PointLight,
}

#[derive(DeriveBundle)]
struct SpotOnly {
    light: SpotLight,
}

/// SkyLight alone — must NOT pull a pose pair (NO require on SkyLight).
#[derive(DeriveBundle)]
struct SkyOnly {
    light: SkyLight,
}

/// A DirectionalLight WITH an explicit, non-default Transform — the require must
/// keep the supplied value (present ⇒ skip) and NOT double-insert.
#[derive(DeriveBundle)]
struct DirWithTransform {
    transform: Transform,
    light: DirectionalLight,
}

// ── helpers ─────────────────────────────────────────────────────────────────

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

fn live_component_count(world: &EcsMaster, entity: Entity) -> usize {
    (0..MAX_COMPONENTS)
        .filter(|&raw| world.has_component(entity, ComponentId(raw)))
        .count()
}

fn a_dir_light() -> DirectionalLight {
    DirectionalLight::new([0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 10.0)
}
fn a_point_light() -> PointLight {
    PointLight::new([1.0, 2.0, 3.0], [1.0, 1.0, 1.0], 50.0, 5.0)
}
fn a_spot_light() -> SpotLight {
    SpotLight::new([1.0, 2.0, 3.0], [0.0, 0.0, -1.0], [1.0, 1.0, 1.0], 100.0, 8.0, 15.0, 30.0)
}

/// Asserts a light spawned ALONE carries EXACTLY `{light, Transform,
/// GlobalTransform}` (3 columns), each pose auto-inserted at its Default.
fn assert_light_auto_inserts_pose(world: &EcsMaster, e: Entity, light_id: ComponentId, ctx: &str) {
    assert!(world.has_component(e, light_id), "{ctx}: the explicit light is present");
    assert!(
        world.has_component(e, Transform::component_id()),
        "{ctx}: S8 require auto-inserts a Transform"
    );
    assert!(
        world.has_component(e, GlobalTransform::component_id()),
        "{ctx}: S8 require auto-inserts a GlobalTransform"
    );
    assert_eq!(
        world.get_component::<Transform>(e).copied(),
        Some(Transform::default()),
        "{ctx}: the auto-inserted Transform holds Transform::default()"
    );
    assert_eq!(
        world.get_component::<GlobalTransform>(e).copied(),
        Some(GlobalTransform::default()),
        "{ctx}: the auto-inserted GlobalTransform holds GlobalTransform::default()"
    );
    assert_eq!(
        live_component_count(world, e),
        3,
        "{ctx}: light + the two auto-inserted pose components = exactly 3 columns"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — each light alone auto-inserts Transform + GlobalTransform
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn directional_light_alone_auto_inserts_pose_pair() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, || DirOnly { light: a_dir_light() });
    assert_light_auto_inserts_pose(&world, e, DirectionalLight::component_id(), "DirectionalLight");
}

#[test]
fn point_light_alone_auto_inserts_pose_pair() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, || PointOnly { light: a_point_light() });
    assert_light_auto_inserts_pose(&world, e, PointLight::component_id(), "PointLight");
}

#[test]
fn spot_light_alone_auto_inserts_pose_pair() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle(&mut world, || SpotOnly { light: a_spot_light() });
    assert_light_auto_inserts_pose(&world, e, SpotLight::component_id(), "SpotLight");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — SkyLight NEGATIVE: alone gains NO Transform / GlobalTransform
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn sky_light_alone_has_no_pose_require() {
    let mut world = EcsMaster::new();
    let _ = Transform::component_id();
    let _ = GlobalTransform::component_id();

    let e = spawn_bundle(&mut world, || SkyOnly {
        light: SkyLight::new([0.2, 0.3, 0.4], [0.05, 0.06, 0.07]),
    });

    assert!(world.has_component(e, SkyLight::component_id()), "SkyLight present");
    assert!(
        !world.has_component(e, Transform::component_id()),
        "S8: SkyLight has NO require — it must NOT gain a Transform (environment term, not positioned)"
    );
    assert!(
        !world.has_component(e, GlobalTransform::component_id()),
        "S8: SkyLight must NOT gain a GlobalTransform"
    );
    assert_eq!(
        live_component_count(&world, e),
        1,
        "SkyLight spawned alone has EXACTLY one column (no auto-inserted pose)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — manual supply does NOT clobber / duplicate
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn manual_transform_on_a_light_is_not_clobbered() {
    let mut world = EcsMaster::new();

    let authored = Transform::from_translation(Vec3::new(9.0, 8.0, 7.0));
    let e = spawn_bundle(&mut world, move || DirWithTransform {
        transform: authored,
        light: a_dir_light(),
    });

    assert!(world.has_component(e, DirectionalLight::component_id()), "light present");
    assert!(world.has_component(e, Transform::component_id()), "Transform present");
    assert!(
        world.has_component(e, GlobalTransform::component_id()),
        "GlobalTransform still auto-inserted (it was absent)"
    );
    assert_eq!(
        world.get_component::<Transform>(e).copied(),
        Some(authored),
        "present ⇒ skip: the authored Transform(9,8,7) is KEPT, not clobbered by the require default"
    );
    assert_eq!(
        live_component_count(&world, e),
        3,
        "exactly ONE Transform (no duplicate) + DirectionalLight + GlobalTransform"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — COMPATIBILITY (0%-gate): require expansion == #[derive(Bundle)] path
// ════════════════════════════════════════════════════════════════════════════

/// A `#[derive(Bundle)]` over EXACTLY the require-expanded DirectionalLight set
/// (`Transform` + `GlobalTransform` + `DirectionalLight`) — the same shape as the
/// S6 `DirectionalLightObject`. The `DirOnly` require expansion must resolve to
/// the SAME `ArchetypeId`.
#[derive(DeriveBundle)]
struct DirLightSpatial {
    transform: Transform,
    global: GlobalTransform,
    light: DirectionalLight,
}

#[test]
fn light_require_expansion_matches_derive_bundle_archetype() {
    let mut world = EcsMaster::new();

    let bundle_arch = world.bundle_archetype_id_for::<DirLightSpatial>();
    let require_arch = world.bundle_archetype_id_for::<DirOnly>();
    assert_eq!(
        require_arch, bundle_arch,
        "0%-gate: the DirectionalLight require-expansion resolves to the SAME ArchetypeId as the \
         #[derive(Bundle)] over {{Transform, GlobalTransform, DirectionalLight}}"
    );

    let from_require = spawn_bundle(&mut world, || DirOnly { light: a_dir_light() });
    let from_bundle = spawn_bundle(&mut world, || DirLightSpatial {
        transform: Transform::IDENTITY,
        global: GlobalTransform::default(),
        light: a_dir_light(),
    });

    assert_eq!(
        world.get_entity_archetype_id(from_require),
        Some(bundle_arch),
        "the require-spawned light is in the shared archetype"
    );
    assert_eq!(
        world.get_entity_archetype_id(from_bundle),
        Some(bundle_arch),
        "the bundle-spawned light is in the very same archetype"
    );
    assert_eq!(live_component_count(&world, from_require), 3, "require: exactly 3 columns");
    assert_eq!(live_component_count(&world, from_bundle), 3, "bundle: exactly 3 columns");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate — WARM-PATH 0%-gate: repeated light require-spawn does NOT rebuild
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn warm_light_require_spawn_does_not_rebuild_archetype() {
    let mut world = EcsMaster::new();

    let cold = world.bundle_archetype_id_for::<PointOnly>();
    let warm = world.bundle_archetype_id_for::<PointOnly>();
    assert_eq!(cold, warm, "bundle_archetype_id_for is idempotent on the require-expanded light bundle");

    let before = world.archetype_count();
    for _ in 0..20u32 {
        spawn_bundle(&mut world, || PointOnly { light: a_point_light() });
    }
    let after = world.archetype_count();
    assert_eq!(
        before, after,
        "20 repeated PointLight require-spawns created ZERO new archetypes (warm cache hit, no rebuild)"
    );
    assert_eq!(world.entity_count(), 20, "all 20 entities spawned");
}
