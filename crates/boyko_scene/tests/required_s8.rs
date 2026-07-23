//! std-lib S8 gate suite (boyko_scene half) — `#[require(...)]` wiring on the
//! render-capability + camera components.
//!
//! S8 is a THIN wiring phase: the `#[require(...)]` machinery itself is shipped
//! and exhaustively tested in `boyko_ecs/tests/required_*.rs`. This suite pins
//! ONLY the scene-side wiring contract:
//!
//! * `MeshHandle` alone (spawned via a single-field bundle so the require funnel
//!   fires) auto-inserts `Transform` + `GlobalTransform` — a renderable can never
//!   exist without a pose.
//! * A manually-supplied `Transform` is NOT clobbered/duplicated by the require
//!   (present ⇒ skip; the explicit value survives).
//! * `Camera` alone auto-inserts `Transform` + `GlobalTransform` + `Projection`
//!   (the perspective placeholder); a designer-supplied `Projection` is kept.
//! * COMPATIBILITY (0%-gate): inserting `MeshHandle` (require-expanded) lands in
//!   the SAME `ArchetypeId` as a `#[derive(Bundle)]` over the identical component
//!   set — the require expansion hits the SAME Phase-8.5 bundle static-cache path
//!   (no extra archetype churn, no extra migration).
//! * WARM-PATH: a repeated require-spawn does NOT rebuild the archetype.
//!
//! # Why single-field bundles
//!
//! Required-component expansion is wired into the bundle-resolution funnel
//! (`cold_register_bundle_archetype` for spawn). The direct `spawn_one` API takes
//! a pre-resolved archetype and bypasses the funnel. So each "insert the
//! capability component alone" gate spawns a single-field `#[derive(Bundle)]`
//! (e.g. `MeshOnly { h: MeshHandle }`) via `Commands::spawn`, exactly like the
//! `boyko_ecs/tests/required_components.rs` suite.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` out of the `Send + Sync` one-shot system closure, and the
// file-static `Mutex<()>` guards serialize tests that arm a process-global (allocator /
// propagation counter). Neither is engine code — the whole file is compiled out of every
// shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle as DeriveBundle;

use boyko_math::Vec3;

use boyko_scene::camera::{Camera, Projection};
use boyko_scene::render_caps::{MeshHandle, MeshRefGen};
use boyko_scene::transform::{GlobalTransform, Transform};

/// The kernel's component-id ceiling (mirror of the crate-private
/// `component_registry::MAX_COMPONENTS`). The exact-set walk scans `[0, MAX)`.
const MAX_COMPONENTS: usize = 512;

// ── single-field bundles that drive the require funnel ──────────────────────────

/// A bundle carrying ONLY a `MeshHandle` — spawning it exercises the require
/// closure on `MeshHandle` (`#[require(Transform, GlobalTransform)]`).
#[derive(DeriveBundle)]
struct MeshOnly {
    mesh: MeshHandle,
}

/// A bundle carrying a `MeshHandle` AND an explicit `Transform`. The require must
/// NOT clobber / duplicate the supplied `Transform` (present ⇒ skip).
#[derive(DeriveBundle)]
struct MeshWithTransform {
    transform: Transform,
    mesh: MeshHandle,
}

/// A bundle carrying ONLY a `Camera` — exercises
/// `#[require(Transform, GlobalTransform, Projection = <perspective preset>)]`.
#[derive(DeriveBundle)]
struct CameraOnly {
    camera: Camera,
}

/// A bundle carrying a `Camera` AND an explicit (orthographic) `Projection`. The
/// require's perspective placeholder must NOT overwrite the designer projection.
#[derive(DeriveBundle)]
struct CameraWithProjection {
    camera: Camera,
    projection: Projection,
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Spawns `make()`'s bundle through `Commands::spawn` (the require funnel) and
/// returns its `Entity`.
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

/// Counts how many registered component ids (over `[0, MAX)`) `entity` carries —
/// used to assert "exactly N columns, no duplicate / spurious migration".
fn live_component_count(world: &EcsMaster, entity: Entity) -> usize {
    (0..MAX_COMPONENTS)
        .filter(|&raw| world.has_component(entity, ComponentId(raw)))
        .count()
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — MeshHandle alone auto-inserts Transform + GlobalTransform
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn mesh_handle_alone_auto_inserts_pose_pair() {
    let mut world = EcsMaster::new();
    let _ = MeshHandle::component_id();
    let _ = Transform::component_id();
    let _ = GlobalTransform::component_id();

    let e = spawn_bundle(&mut world, || MeshOnly { mesh: MeshHandle(7) });

    assert!(
        world.has_component(e, MeshHandle::component_id()),
        "the explicitly-spawned MeshHandle is present"
    );
    assert!(
        world.has_component(e, Transform::component_id()),
        "S8 require: MeshHandle alone auto-inserts a Transform (renderable always has a pose)"
    );
    assert!(
        world.has_component(e, GlobalTransform::component_id()),
        "S8 require: MeshHandle alone auto-inserts a GlobalTransform"
    );
    // The auto-inserted pose holds each type's Default (a valid origin pose).
    assert_eq!(
        world.get_component::<Transform>(e).copied(),
        Some(Transform::default()),
        "the auto-inserted Transform holds Transform::default()"
    );
    assert_eq!(
        world.get_component::<GlobalTransform>(e).copied(),
        Some(GlobalTransform::default()),
        "the auto-inserted GlobalTransform holds GlobalTransform::default()"
    );
    // `MeshHandle` also requires `MeshRefGen` (asset-streaming F5 generation carrier), so
    // the closure is four columns, not three. The count was written before that attribute
    // existed and only went red when the 2026-07 audit made the vacuously-green CI run this
    // suite again — the engine expands the closure correctly.
    assert!(
        world.has_component(e, MeshRefGen::component_id()),
        "S8 require: MeshHandle alone auto-inserts its MeshRefGen generation carrier"
    );
    // Exactly the four columns — no spurious extra migration.
    assert_eq!(
        live_component_count(&world, e),
        4,
        "MeshHandle + the two auto-inserted pose components + MeshRefGen = exactly 4 columns"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — manual supply does NOT double-insert / clobber
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn manual_transform_is_not_clobbered_or_duplicated() {
    let mut world = EcsMaster::new();

    // A non-default Transform: if the require clobbered it, the value would reset
    // to Transform::default() (translation 0); if it double-inserted, the column
    // count would differ from the StaticProp-equivalent set.
    let authored = Transform::from_translation(Vec3::new(3.0, 4.0, 5.0));
    let e = spawn_bundle(&mut world, move || MeshWithTransform {
        transform: authored,
        mesh: MeshHandle(11),
    });

    assert!(world.has_component(e, MeshHandle::component_id()), "MeshHandle present");
    assert!(world.has_component(e, Transform::component_id()), "Transform present");
    assert!(
        world.has_component(e, GlobalTransform::component_id()),
        "GlobalTransform still auto-inserted (it was absent)"
    );

    // The manually-supplied Transform value is PRESERVED (present ⇒ skip, no
    // overwrite by Transform::default()).
    assert_eq!(
        world.get_component::<Transform>(e).copied(),
        Some(authored),
        "present ⇒ skip: the authored Transform(3,4,5) is KEPT, not clobbered by the require's default"
    );

    // Exactly FOUR columns: MeshHandle + the one supplied Transform (not two) + the one
    // auto-inserted GlobalTransform + the required MeshRefGen. A double-insert would not
    // change the archetype set (same id), but the value check above already proves no
    // clobber; this pins "no spurious extra component".
    assert_eq!(
        live_component_count(&world, e),
        4,
        "exactly ONE Transform (no duplicate column) + MeshHandle + GlobalTransform + MeshRefGen"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3a — Camera alone auto-inserts Transform + GlobalTransform + Projection
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn camera_alone_auto_inserts_pose_and_projection() {
    let mut world = EcsMaster::new();
    let _ = Camera::component_id();
    let _ = Projection::component_id();

    let e = spawn_bundle(&mut world, || CameraOnly { camera: Camera::DEFAULT });

    assert!(world.has_component(e, Camera::component_id()), "Camera present");
    assert!(
        world.has_component(e, Transform::component_id()),
        "S8 require: Camera alone auto-inserts a Transform"
    );
    assert!(
        world.has_component(e, GlobalTransform::component_id()),
        "S8 require: Camera alone auto-inserts a GlobalTransform"
    );
    assert!(
        world.has_component(e, Projection::component_id()),
        "S8 require: Camera alone auto-inserts a Projection (capture-free perspective preset)"
    );

    // The auto-inserted Projection is the perspective placeholder the require
    // declares (Projection has no Default — it MUST come from the `= expr` ctor).
    let proj = world.get_component::<Projection>(e).copied().expect("Projection lives");
    match proj {
        Projection::Perspective { fov_y, aspect, near, far } => {
            assert!(
                (fov_y - core::f32::consts::FRAC_PI_3).abs() < 1e-6,
                "the placeholder is a 60-degree (FRAC_PI_3) vertical-FOV perspective"
            );
            assert!((aspect - 16.0 / 9.0).abs() < 1e-6, "16:9 aspect placeholder");
            assert!((near - 0.1).abs() < 1e-6, "near 0.1 placeholder");
            assert!((far - 1000.0).abs() < 1e-3, "far 1000 placeholder");
        }
        Projection::Orthographic { .. } => {
            panic!("the require's Projection placeholder must be the perspective preset")
        }
    }

    assert_eq!(
        live_component_count(&world, e),
        4,
        "Camera + Transform + GlobalTransform + Projection = exactly 4 columns"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3b — Camera with an explicit Projection keeps the designer projection
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn camera_keeps_an_explicitly_supplied_projection() {
    let mut world = EcsMaster::new();

    // A 2D-first caller supplies an ORTHOGRAPHIC projection; the require's
    // perspective placeholder must NOT overwrite it (present ⇒ skip).
    let ortho = Projection::Orthographic { half_height: 5.0, aspect: 1.0, near: 0.0, far: 100.0 };
    let e = spawn_bundle(&mut world, move || CameraWithProjection {
        camera: Camera::DEFAULT,
        projection: ortho,
    });

    assert!(world.has_component(e, Projection::component_id()), "Projection present");
    assert_eq!(
        world.get_component::<Projection>(e).copied(),
        Some(ortho),
        "present ⇒ skip: the explicit Orthographic projection is KEPT, not replaced by the perspective placeholder"
    );
    // Pose pair still auto-inserted; exactly 4 columns, ONE Projection.
    assert!(world.has_component(e, Transform::component_id()), "Transform auto-inserted");
    assert!(world.has_component(e, GlobalTransform::component_id()), "GlobalTransform auto-inserted");
    assert_eq!(
        live_component_count(&world, e),
        4,
        "exactly ONE Projection (no duplicate) + Camera + the pose pair"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 4 — COMPATIBILITY (0%-gate): require expansion == #[derive(Bundle)] path
// ════════════════════════════════════════════════════════════════════════════

/// A `#[derive(Bundle)]` over EXACTLY the require-expanded MeshHandle set
/// (`Transform` + `GlobalTransform` + `MeshHandle`). Spawning `MeshOnly` (which
/// the require expands to this same set) MUST land in the SAME `ArchetypeId`.
#[derive(DeriveBundle)]
struct MeshSpatialBundle {
    transform: Transform,
    global: GlobalTransform,
    mesh: MeshHandle,
}

#[test]
fn require_expansion_matches_derive_bundle_archetype() {
    let mut world = EcsMaster::new();

    // The explicit bundle over the full set resolves an archetype id (the
    // Phase-8.5 cached path).
    let bundle_arch = world.bundle_archetype_id_for::<MeshSpatialBundle>();

    // The single-field MeshOnly bundle's require expansion ALSO goes through the
    // bundle funnel (`cold_register_bundle_archetype` expands the closure before
    // `get_or_create_archetype`), so its resolved archetype must be IDENTICAL.
    let require_arch = world.bundle_archetype_id_for::<MeshOnly>();
    assert_eq!(
        require_arch, bundle_arch,
        "0%-gate: the MeshHandle require-expansion resolves to the SAME ArchetypeId as the \
         #[derive(Bundle)] over {{Transform, GlobalTransform, MeshHandle}}"
    );

    // And a REAL spawn of each lands its entity in that one archetype (no extra
    // migration, no second archetype).
    let from_require = spawn_bundle(&mut world, || MeshOnly { mesh: MeshHandle(1) });
    let from_bundle = spawn_bundle(&mut world, || MeshSpatialBundle {
        transform: Transform::IDENTITY,
        global: GlobalTransform::default(),
        mesh: MeshHandle(2),
    });

    assert_eq!(
        world.get_entity_archetype_id(from_require),
        Some(bundle_arch),
        "the require-spawned entity is in the shared archetype"
    );
    assert_eq!(
        world.get_entity_archetype_id(from_bundle),
        Some(bundle_arch),
        "the bundle-spawned entity is in the very same archetype"
    );

    // Identical exact component set on both.
    for id in [
        Transform::component_id(),
        GlobalTransform::component_id(),
        MeshHandle::component_id(),
        // Pulled in by `MeshHandle`'s own `#[require]` on BOTH paths — which is exactly the
        // equivalence this gate exists to prove: the require expansion and the derive-bundle
        // expansion reach the identical column set.
        MeshRefGen::component_id(),
    ] {
        assert!(world.has_component(from_require, id), "require entity carries {id:?}");
        assert!(world.has_component(from_bundle, id), "bundle entity carries {id:?}");
    }
    assert_eq!(live_component_count(&world, from_require), 4, "require: exactly 4 columns");
    assert_eq!(live_component_count(&world, from_bundle), 4, "bundle: exactly 4 columns");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 5 — WARM-PATH 0%-gate: repeated require-spawn does NOT rebuild
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn warm_require_spawn_does_not_rebuild_archetype() {
    let mut world = EcsMaster::new();

    // Cold-resolve the require-expanded archetype once.
    let cold = world.bundle_archetype_id_for::<MeshOnly>();
    let warm = world.bundle_archetype_id_for::<MeshOnly>();
    assert_eq!(cold, warm, "bundle_archetype_id_for is idempotent on the require-expanded bundle");

    // A burst of require-spawns must create ZERO new archetypes (the expansion
    // hits the cached slot, same as a plain bundle).
    let before = world.archetype_count();
    for i in 0..32u32 {
        spawn_bundle(&mut world, move || MeshOnly { mesh: MeshHandle(i) });
    }
    let after = world.archetype_count();
    assert_eq!(
        before, after,
        "32 repeated MeshHandle require-spawns created ZERO new archetypes (warm cache hit, no rebuild)"
    );
    assert_eq!(world.entity_count(), 32, "all 32 entities spawned");
    assert_eq!(
        world.bundle_archetype_id_for::<MeshOnly>(),
        cold,
        "the cached require-expanded archetype id is stable after the spawn burst"
    );
}
