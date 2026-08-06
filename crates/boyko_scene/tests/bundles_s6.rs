//! std-lib S6 gate suite (boyko_scene half) — object-category bundle presets +
//! the `Name` component and its setup-only string interner.
//!
//! Covers the S6 gates that live entirely in `boyko_scene` (no physics / render
//! crate dependency):
//!
//! * EXACT component set — each scene bundle (`SpatialBundle` / `StaticProp` /
//!   `CameraRig`) spawns precisely its declared columns PLUS the transitive `#[require]`
//!   closure those columns pull in, and nothing else. The "no more" half walks all
//!   `MAX_COMPONENTS` ids. The closure is spelled out per test rather than derived: the
//!   kernel's `get_required_plan` is `pub(crate)`, and naming the expected carriers keeps
//!   the test able to FAIL when a new `#[require]` edge appears unannounced.
//! * WARM-PATH cache — a repeated bundle spawn hits the Phase-8.5 per-impl static
//!   bundle cache: `bundle_archetype_id_for` is idempotent and the world's
//!   archetype count does NOT grow per spawn (no per-spawn archetype rebuild).
//! * 0%-GATE — a bundle spawn lands in the SAME `ArchetypeId` as the equivalent
//!   manual multi-insert into a hand-built archetype with the same component set
//!   (no extra migration, same archetype).
//! * NAME / INTERNER — `intern("foo")` round-trips; two interns of equal strings
//!   yield the SAME `NameId`; the interner is OFF the per-frame path (spawning N
//!   named entities and iterating `Query<&Name>` does NOT grow the interner).
//!
//! The cross-crate physics / render bundle gates (DynamicBody fall + Gpu3dInstance
//! pack, and the light-object bundles) live in their own crates' S6 suites.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` out of the `Send + Sync` one-shot system closure, and the
// file-static `Mutex<()>` guard (`INTERNER_LOCK`) serializes the tests that touch the
// process-global string interner. Neither is engine code — the whole file is compiled out
// of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_macros::Bundle as DeriveBundle;

use boyko_math::Vec3;

use boyko_scene::bundles::{CameraRig, SpatialBundle, StaticProp};
use boyko_scene::camera::{Camera, Projection};
use boyko_scene::identity::{self, Name, NameId};
use boyko_scene::render_caps::{
    MaterialHandle, MaterialRefGen, MeshHandle, MeshRefGen, Visibility,
};
use boyko_scene::transform::{GlobalTransform, Transform};

/// The kernel's component-id ceiling (mirror of `component_registry::MAX_COMPONENTS`,
/// which is crate-private to `boyko_ecs`). The exact-set walk scans `[0, MAX)`.
const MAX_COMPONENTS: usize = 512;

// ── interner serialization ──────────────────────────────────────────────────────
//
// `boyko_scene::identity`'s interner is a PROCESS-GLOBAL mint registry
// (`identity.rs:81`), shared by every test thread in this binary. `interner_len()` is
// therefore a shared counter, and `interner_is_off_the_per_frame_path` asserts that it
// does NOT move across a stretch of work — a claim a SIBLING test can falsify by
// interning at the same moment. libtest runs these tests on parallel threads by default,
// and that is precisely the measured signature: the reader passes alone, passes under
// `--test-threads=1`, and passes in debug, and fails ONLY in release with default
// parallelism — where nothing about the claim changed, only whether a sibling's mint
// lands inside its window.
//
// Every test that READS or WRITES the interner holds this lock for its whole body, so at
// most one of them is live at a time. Nothing else in the file is serialized: the bundle /
// archetype gates own their `EcsMaster` outright and share no global, and serializing them
// would only slow the suite and blur which tests actually share state.
static INTERNER_LOCK: Mutex<()> = Mutex::new(());

/// Takes the interner guard, TOLERATING poison: if a guarded test panics while holding the
/// lock, its siblings must still report their own verdict rather than cascade a
/// `PoisonError` — the protected datum is a process-global the panicking test does not
/// leave in a torn state (a `usize` count and an append-only registry).
fn lock_interner() -> std::sync::MutexGuard<'static, ()> {
    INTERNER_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ── shared exact-membership oracle ──────────────────────────────────────────────

/// Asserts that `entity` carries EXACTLY the components in `expected` — every id in
/// `expected` is present AND no other registered id (across the whole `[0, MAX)`
/// space) is present. This is the "precisely those components, no more/less" gate:
/// `has_component(e, id)` must be the characteristic function of `expected`.
fn assert_exact_component_set(world: &EcsMaster, entity: Entity, expected: &[ComponentId], ctx: &str) {
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
        let want = expected.contains(&id);
        let got = world.has_component(entity, id);
        if got && !want {
            extra.push(raw);
        }
    }
    assert!(
        extra.is_empty(),
        "{ctx}: bundle entity carries UNEXPECTED components {:?} (declared set was {:?})",
        extra,
        expected
    );
    // The entity must have a live archetype (sanity for the membership walk).
    assert!(
        world.get_entity_archetype_id(entity).is_some(),
        "{ctx}: spawned entity has a live archetype"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — EXACT component set per scene bundle
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn spatial_bundle_spawns_exactly_its_three_components() {
    let mut world = EcsMaster::new();
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(SpatialBundle {
                transform: Transform::IDENTITY,
                global: GlobalTransform::default(),
                visibility: Visibility::Visible,
            })
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let e = sink.lock().expect("probe").expect("spatial spawn handle");

    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        Visibility::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "SpatialBundle");
    assert_eq!(
        SpatialBundle::component_ids().len(),
        3,
        "SpatialBundle is arity 3"
    );
}

#[test]
fn static_prop_spawns_its_declared_set_plus_required_closure() {
    let mut world = EcsMaster::new();
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(StaticProp {
                transform: Transform::IDENTITY,
                global: GlobalTransform::default(),
                mesh: MeshHandle(7),
                material: MaterialHandle(3),
                visibility: Visibility::Inherited,
            })
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let e = sink.lock().expect("probe").expect("static-prop spawn handle");

    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        MeshHandle::component_id(),
        MaterialHandle::component_id(),
        Visibility::component_id(),
        // The `#[require]` closure, NOT bundle fields: `MeshHandle` declares
        // `#[require(Transform, GlobalTransform, MeshRefGen)]` and `MaterialHandle`
        // `#[require(MaterialRefGen)]` (asset-streaming F5 generation carriers), so any
        // bundle naming those handles legitimately materialises two extra columns. This
        // suite predates those attributes and went red the moment the 2026-07 audit fixed
        // the vacuously-green CI and it actually ran again. The closure is spelled out so
        // the check still FAILS on a new unannounced `#[require]` edge.
        MeshRefGen::component_id(),
        MaterialRefGen::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "StaticProp");
    assert_eq!(StaticProp::component_ids().len(), 5, "StaticProp is arity 5");
}

#[test]
fn camera_rig_spawns_exactly_its_four_components() {
    let mut world = EcsMaster::new();
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(CameraRig {
                transform: Transform::IDENTITY,
                global: GlobalTransform::default(),
                camera: Camera::DEFAULT,
                projection: Projection::Perspective {
                    fov_y: 1.0,
                    aspect: 1.0,
                    near: 0.1,
                    far: 100.0,
                },
            })
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let e = sink.lock().expect("probe").expect("camera-rig spawn handle");

    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        Camera::component_id(),
        Projection::component_id(),
    ];
    assert_exact_component_set(&world, e, &expected, "CameraRig");
    assert_eq!(CameraRig::component_ids().len(), 4, "CameraRig is arity 4");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — WARM-PATH: repeated spawn hits the static bundle cache (no rebuild)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn warm_path_spawn_hits_static_cache_no_per_spawn_rebuild() {
    let mut world = EcsMaster::new();

    // Cold: first resolution creates the archetype. Warm: the per-impl OnceLock
    // slot returns the SAME id with no rebuild (SBC4).
    let cold = world.bundle_archetype_id_for::<StaticProp>();
    let warm = world.bundle_archetype_id_for::<StaticProp>();
    assert_eq!(
        cold, warm,
        "bundle_archetype_id_for is idempotent — the warm call hits the cached slot"
    );

    // Spawning the SAME bundle many times must NOT grow the archetype count: every
    // spawn after the first reuses the cached archetype (no per-spawn rebuild).
    let before = world.archetype_count();
    for i in 0..32u32 {
        world.run_system(move |mut cmds: Commands| {
            cmds.spawn(StaticProp {
                transform: Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
                global: GlobalTransform::default(),
                mesh: MeshHandle(i),
                material: MaterialHandle(i as u16),
                visibility: Visibility::Visible,
            });
        });
    }
    let after = world.archetype_count();
    assert_eq!(
        before, after,
        "32 repeated StaticProp spawns created ZERO new archetypes (warm cache hit, no rebuild)"
    );
    assert_eq!(world.entity_count(), 32, "all 32 entities spawned");

    // And the cached id still matches what the spawns used.
    assert_eq!(
        world.bundle_archetype_id_for::<StaticProp>(),
        cold,
        "the cached archetype id is stable after the spawn burst"
    );
}

#[test]
fn static_info_pointer_is_stable_per_bundle() {
    // The per-impl `static OnceLock<BundleStaticInfo>` returns the SAME &'static
    // pointer on every call (the cache slot a warm spawn reads — SBC2/SBC3).
    let a = SpatialBundle::static_info();
    let b = SpatialBundle::static_info();
    assert!(
        std::ptr::eq(a, b),
        "SpatialBundle::static_info() is a stable &'static cache pointer"
    );

    // Distinct bundle types own distinct BundleTypeIds.
    assert_ne!(
        SpatialBundle::bundle_type_id(),
        StaticProp::bundle_type_id(),
        "distinct S6 bundles get distinct BundleTypeIds"
    );
    assert_ne!(
        StaticProp::bundle_type_id(),
        CameraRig::bundle_type_id(),
        "distinct S6 bundles get distinct BundleTypeIds"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — 0%-GATE: bundle spawn == equivalent manual multi-insert (same archetype)
// ════════════════════════════════════════════════════════════════════════════

/// Views a `#[repr(C/transparent)]` POD as raw bytes for the manual `create_entity`
/// spawn path.
///
/// # Safety
/// `T` is a `#[repr(C)]` / `#[repr(transparent)]` component whose byte image is a
/// valid serialization for its pool (holds for every component spawned here).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we view its `size_of::<T>()` bytes read-only.
    // `T` is fixed-layout `#[repr(C/transparent)]`, matching the pool's stored
    // layout; the slice borrows `value` so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Manually spawns a StaticProp-equivalent entity into a hand-built archetype with
/// the SAME component set, returning (archetype_id, entity).
fn manual_static_prop(world: &mut EcsMaster) -> (ArchetypeId, Entity) {
    // Must include the `#[require]` closure the bundle path materialises, or the two
    // archetypes differ by two columns and the 0%-gate compares unlike things.
    let arch = world.create_archetype(&[
        Transform::component_id(),
        GlobalTransform::component_id(),
        MeshHandle::component_id(),
        MaterialHandle::component_id(),
        Visibility::component_id(),
        MeshRefGen::component_id(),
        MaterialRefGen::component_id(),
    ]);
    let t = Transform::IDENTITY;
    let g = GlobalTransform::default();
    let mh = MeshHandle(42);
    let mat = MaterialHandle(9);
    let vis = Visibility::Visible;
    // What the require-ctors would write: both carriers default to GEN_UNSYNCED.
    let mesh_gen = MeshRefGen::default();
    let mat_gen = MaterialRefGen::default();
    let e = world
        .create_entity(
            arch,
            &[
                (Transform::component_id(), as_bytes(&t)),
                (GlobalTransform::component_id(), as_bytes(&g)),
                (MeshHandle::component_id(), as_bytes(&mh)),
                (MaterialHandle::component_id(), as_bytes(&mat)),
                (Visibility::component_id(), as_bytes(&vis)),
                (MeshRefGen::component_id(), as_bytes(&mesh_gen)),
                (MaterialRefGen::component_id(), as_bytes(&mat_gen)),
            ],
        )
        .expect("manual StaticProp archetype accepts its seven columns");
    (arch, e)
}

#[test]
fn bundle_spawn_lands_in_same_archetype_as_manual_insert() {
    let mut world = EcsMaster::new();

    // Manual multi-insert first establishes the archetype for the component set.
    let (manual_arch, manual_e) = manual_static_prop(&mut world);

    // The bundle's resolved archetype must be the SAME id (canonical-sorted ids +
    // idempotent get_or_create_archetype): no extra migration, no second archetype.
    let bundle_arch = world.bundle_archetype_id_for::<StaticProp>();
    assert_eq!(
        bundle_arch, manual_arch,
        "0%-gate: a StaticProp bundle resolves to the SAME archetype as the manual \
         multi-insert with the identical component set"
    );

    // And a real bundle spawn lands the entity in that very archetype.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(StaticProp {
                transform: Transform::IDENTITY,
                global: GlobalTransform::default(),
                mesh: MeshHandle(1),
                material: MaterialHandle(1),
                visibility: Visibility::Visible,
            })
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let bundle_e = sink.lock().expect("probe").expect("bundle spawn handle");

    assert_eq!(
        world.get_entity_archetype_id(bundle_e),
        Some(manual_arch),
        "0%-gate: the bundle-spawned entity shares the manual entity's archetype"
    );
    assert_eq!(
        world.get_entity_archetype_id(manual_e),
        Some(manual_arch),
        "manual entity is in its own archetype (sanity)"
    );

    // Both entities carry the identical exact component set.
    let expected = [
        Transform::component_id(),
        GlobalTransform::component_id(),
        MeshHandle::component_id(),
        MaterialHandle::component_id(),
        Visibility::component_id(),
        // The `#[require]` closure, NOT bundle fields: `MeshHandle` declares
        // `#[require(Transform, GlobalTransform, MeshRefGen)]` and `MaterialHandle`
        // `#[require(MaterialRefGen)]` (asset-streaming F5 generation carriers), so any
        // bundle naming those handles legitimately materialises two extra columns. This
        // suite predates those attributes and went red the moment the 2026-07 audit fixed
        // the vacuously-green CI and it actually ran again. The closure is spelled out so
        // the check still FAILS on a new unannounced `#[require]` edge.
        MeshRefGen::component_id(),
        MaterialRefGen::component_id(),
    ];
    assert_exact_component_set(&world, bundle_e, &expected, "StaticProp (bundle)");
    assert_exact_component_set(&world, manual_e, &expected, "StaticProp (manual)");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 4 — NAME / INTERNER round-trip, dedup, and off-the-hot-path
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn intern_round_trips_and_dedups() {
    // Mints into the process-global interner (a WRITER of the count
    // `interner_is_off_the_per_frame_path` pins).
    let _interner = lock_interner();
    let a = identity::intern("player_one_s6");
    let resolved = identity::resolve(a).expect("interned string resolves");
    assert_eq!(resolved, "player_one_s6", "intern→resolve round-trips the string");

    // Two interns of an EQUAL string yield the SAME NameId (and thus equal Name).
    let b = identity::intern("player_one_s6");
    assert_eq!(a, b, "two interns of equal strings yield the SAME Name (dedup)");
    assert_eq!(a.0, b.0, "the underlying NameId is identical (single u32 compare)");

    // A distinct string yields a distinct id.
    let c = identity::intern("player_two_s6");
    assert_ne!(a.0, c.0, "distinct strings get distinct NameIds");

    // An unminted NameId resolves to None (no panic, no UB).
    let bogus = Name(NameId(u32::MAX - 7));
    assert_eq!(identity::resolve(bogus), None, "an unminted NameId resolves to None");
}

#[test]
fn name_is_a_transparent_u32_lane() {
    // Layout pin: a Name column is a dense u32 array (the brief's #[repr(transparent)]).
    assert_eq!(size_of::<Name>(), 4, "Name is a 4-byte transparent u32");
    assert_eq!(align_of::<Name>(), 4);
    assert_eq!(size_of::<NameId>(), 4, "NameId is a 4-byte transparent u32");
}

/// The Principle-0 boundary: the interner is consulted ONLY at intern/setup time,
/// NEVER on a per-frame path. We spawn N named entities (interning each name ONCE
/// at setup), record the interner length, then iterate `Query<&Name>` many times —
/// the interner length must NOT change (iteration reads the inline `NameId`, it
/// never calls back into `intern`/`resolve`).
#[test]
fn interner_is_off_the_per_frame_path() {
    // READS the process-global `identity::interner_len()` and asserts it does not move;
    // the guard keeps every interning sibling out of that window.
    let _interner = lock_interner();
    let mut world = EcsMaster::new();

    // Setup: intern N distinct names ONCE, spawn an entity carrying each Name.
    const N: u32 = 16;
    let mut names = Vec::with_capacity(N as usize);
    for i in 0..N {
        // Distinct, fresh strings (suffix keeps them disjoint from other tests).
        let n = identity::intern(&format!("entity_{i}_s6_offpath"));
        names.push(n);
    }
    let len_after_intern = identity::interner_len();

    let arch = world.create_archetype(&[Name::component_id()]);
    for n in &names {
        world
            .create_entity(arch, &[(Name::component_id(), as_bytes(n))])
            .expect("Name archetype accepts its one column");
    }

    // The interner length must not have moved from spawning (spawn copies the
    // inline NameId bytes — it does NOT re-intern).
    assert_eq!(
        identity::interner_len(),
        len_after_intern,
        "spawning named entities does NOT grow the interner (NameId is carried inline)"
    );

    // Per-frame path: iterate Query<&Name> repeatedly, summing the raw ids. This is
    // the hot read path; it must NOT consult the interner at all.
    let observed = Arc::new(Mutex::new(0u64));
    for _frame in 0..8u32 {
        let acc = Arc::clone(&observed);
        world.run_system(move |q: Query<&Name>| {
            let mut sum = 0u64;
            for name in q.iter() {
                sum = sum.wrapping_add(u64::from(name.0 .0));
            }
            *acc.lock().expect("acc") = sum;
        });
        assert_eq!(
            identity::interner_len(),
            len_after_intern,
            "iterating Query<&Name> on frame {_frame} did NOT re-intern (interner off the per-frame path)"
        );
    }

    // Anti-vacuity: the per-frame query actually visited the N named rows.
    let n_rows = {
        let count = Arc::new(Mutex::new(0usize));
        let c = Arc::clone(&count);
        world.run_system(move |q: Query<&Name>| {
            *c.lock().expect("c") = q.iter().count();
        });
        *count.lock().expect("count")
    };
    assert_eq!(n_rows, N as usize, "the query visited all N named rows (anti-vacuity)");
    let _ = *observed.lock().expect("observed");
}

// A trivial local bundle proves derive(Bundle) works on a Name-carrying named
// struct (the interner+Name integrate with the bundle machinery).
#[derive(DeriveBundle)]
struct NamedSpatial {
    name: Name,
    transform: Transform,
    global: GlobalTransform,
}

#[test]
fn name_participates_in_a_derived_bundle() {
    // Mints into the process-global interner (a WRITER of the count
    // `interner_is_off_the_per_frame_path` pins).
    let _interner = lock_interner();
    let mut world = EcsMaster::new();
    let name = identity::intern("named_spatial_s6");
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(NamedSpatial {
                name,
                transform: Transform::IDENTITY,
                global: GlobalTransform::default(),
            })
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    let e = sink.lock().expect("probe").expect("named-spatial spawn handle");

    let stored = *world.get_component::<Name>(e).expect("Name lives");
    assert_eq!(stored, name, "the Name component round-trips through a bundle spawn");
    assert_eq!(
        identity::resolve(stored),
        Some("named_spatial_s6"),
        "the stored NameId resolves back to its string"
    );
}
