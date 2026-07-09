//! std-lib S4 gate suite (boyko_scene half) — render-capability components.
//!
//! Covers the component-side S4 gates that do NOT require `boyko_render`:
//!
//! * `MeshHandle` / `MaterialHandle` / `Visibility` round-trip (spawn, read back).
//! * `Visibility` discriminant pins (`Inherited = 0`, `Visible = 1`, `Hidden = 2`)
//!   and its `Default` (`Inherited`).
//! * `RenderEnabled` is a BITSET tag (no archetype signature membership) and its
//!   `enable` / `disable` / `Enabled<RenderEnabled>` filter behave as the S4 pack
//!   step relies on (a row is visited iff its bit is set).
//! * Compile-time layout pins (`MeshHandle` 4 B, `MaterialHandle` 2 B,
//!   `Visibility` 1 B) re-affirmed at runtime so the fingerprint shows in the
//!   report.
//!
//! The `Gpu3dInstance` pack, light reconcile, const-asserts for the GPU records,
//! the 0%-gate, and Miri live in `boyko_render/tests/render_upload_s4.rs` (that
//! crate names the GPU types).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::iters::query::{Disabled, Enabled};
use boyko_ecs::ecs::core::component::component_registry::{self, StorageKind};
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;

use boyko_scene::render_caps::{MaterialHandle, MeshHandle, RenderEnabled, Visibility};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Views a `#[repr(C/transparent)]` POD value as its raw bytes for the
/// `create_entity` spawn path.
///
/// # Safety
/// `T` is a `#[repr(C)]` / `#[repr(transparent)]` component whose byte image is a
/// valid serialization for its pool (holds for `MeshHandle`/`MaterialHandle`/
/// `Visibility` — all fixed-layout PODs stored by raw byte copy).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we view its `size_of::<T>()` bytes read-only.
    // `T` is `#[repr(C/transparent)]`, matching the pool's stored layout. The
    // slice borrows `value`, so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// An archetype carrying the three render-cap components.
fn render_cap_archetype(ecs: &mut EcsMaster) -> ArchetypeId {
    ecs.create_archetype(&[
        MeshHandle::component_id(),
        MaterialHandle::component_id(),
        Visibility::component_id(),
    ])
}

/// Spawns an entity with the given mesh/material/visibility.
fn spawn_render_cap(
    ecs: &mut EcsMaster,
    arch: ArchetypeId,
    mesh: MeshHandle,
    mat: MaterialHandle,
    vis: Visibility,
) -> Entity {
    ecs.create_entity(
        arch,
        &[
            (MeshHandle::component_id(), as_bytes(&mesh)),
            (MaterialHandle::component_id(), as_bytes(&mat)),
            (Visibility::component_id(), as_bytes(&vis)),
        ],
    )
    .expect("invariant: render-cap archetype accepts its three columns")
}

// ── Gate: component round-trip ────────────────────────────────────────────────

#[test]
fn mesh_material_visibility_round_trip() {
    let mut ecs = EcsMaster::new();
    let arch = render_cap_archetype(&mut ecs);

    let e = spawn_render_cap(
        &mut ecs,
        arch,
        MeshHandle(0xDEAD_BEEF),
        MaterialHandle(0xC0DE),
        Visibility::Visible,
    );

    let mesh = *ecs.get_component::<MeshHandle>(e).expect("MeshHandle lives");
    let mat = *ecs.get_component::<MaterialHandle>(e).expect("MaterialHandle lives");
    let vis = *ecs.get_component::<Visibility>(e).expect("Visibility lives");

    assert_eq!(mesh, MeshHandle(0xDEAD_BEEF), "MeshHandle round-trips its u32 verbatim");
    assert_eq!(mat, MaterialHandle(0xC0DE), "MaterialHandle round-trips its u16 verbatim");
    assert_eq!(vis, Visibility::Visible, "Visibility round-trips the authored variant");
}

#[test]
fn visibility_variants_round_trip_each() {
    let mut ecs = EcsMaster::new();
    let arch = render_cap_archetype(&mut ecs);

    for vis in [Visibility::Inherited, Visibility::Visible, Visibility::Hidden] {
        let e = spawn_render_cap(&mut ecs, arch, MeshHandle(1), MaterialHandle(2), vis);
        assert_eq!(
            *ecs.get_component::<Visibility>(e).expect("Visibility lives"),
            vis,
            "each Visibility variant round-trips through spawn/read-back"
        );
    }
}

// ── Gate: Visibility discriminant + Default pins ──────────────────────────────

#[test]
fn visibility_default_is_inherited_and_discriminants_are_pinned() {
    assert_eq!(Visibility::default(), Visibility::Inherited, "default is Inherited");
    // The `#[repr(u8)]` discriminants are load-bearing for serialization stability.
    assert_eq!(Visibility::Inherited as u8, 0, "Inherited = 0");
    assert_eq!(Visibility::Visible as u8, 1, "Visible = 1");
    assert_eq!(Visibility::Hidden as u8, 2, "Hidden = 2");
}

// ── Gate: layout pins re-affirmed at runtime (const-asserts mirror) ───────────

#[test]
fn render_cap_layout_pins_hold() {
    assert_eq!(size_of::<MeshHandle>(), 4, "MeshHandle is 4 B (transparent u32)");
    assert_eq!(align_of::<MeshHandle>(), 4);
    assert_eq!(size_of::<MaterialHandle>(), 2, "MaterialHandle is 2 B (transparent u16)");
    assert_eq!(align_of::<MaterialHandle>(), 2);
    assert_eq!(size_of::<Visibility>(), 1, "Visibility is 1 B (repr(u8))");
    assert_eq!(align_of::<Visibility>(), 1);
}

// ── Gate: RenderEnabled is a bitset tag (no signature membership) ─────────────

#[test]
fn render_enabled_is_a_bitset_tag() {
    let id = RenderEnabled::component_id();
    assert_eq!(
        component_registry::storage_kind(id.0),
        StorageKind::Bitset,
        "RenderEnabled must classify as a bitset tag (no ComponentPool, no archetype signature)"
    );
    const {
        assert!(
            <RenderEnabled as Component>::STORAGE_IS_BITSET,
            "RenderEnabled must emit STORAGE_IS_BITSET = true"
        );
    }
}

#[test]
fn render_enabled_enable_disable_round_trip() {
    let mut ecs = EcsMaster::new();
    let arch = render_cap_archetype(&mut ecs);
    let e = spawn_render_cap(&mut ecs, arch, MeshHandle(1), MaterialHandle(1), Visibility::Visible);

    assert!(!ecs.is_enabled::<RenderEnabled>(e), "starts disabled (bit clear)");
    ecs.enable::<RenderEnabled>(e);
    assert!(ecs.is_enabled::<RenderEnabled>(e), "enable sets the RenderEnabled bit");
    ecs.disable::<RenderEnabled>(e);
    assert!(!ecs.is_enabled::<RenderEnabled>(e), "disable clears the RenderEnabled bit");
}

/// The exact mechanism the S4 pack relies on: `Enabled<RenderEnabled>` visits
/// ONLY rows whose bit is set, and `Disabled<RenderEnabled>` visits the
/// complement — so a `Visibility::Hidden` row that leaves its bit CLEAR is
/// skipped branch-free by the pack query.
#[test]
fn enabled_render_enabled_filters_visible_rows_only() {
    let mut ecs = EcsMaster::new();
    let arch = render_cap_archetype(&mut ecs);

    let visible = spawn_render_cap(&mut ecs, arch, MeshHandle(10), MaterialHandle(1), Visibility::Visible);
    let _hidden = spawn_render_cap(&mut ecs, arch, MeshHandle(20), MaterialHandle(2), Visibility::Hidden);

    // Visible row opts INTO the draw (sets its bit); Hidden row leaves it clear.
    ecs.enable::<RenderEnabled>(visible);
    // (hidden left disabled deliberately)

    let enabled_meshes: Vec<u32> = ecs
        .query::<&MeshHandle, Enabled<RenderEnabled>>()
        .iter()
        .map(|m| m.0)
        .collect();
    assert_eq!(
        enabled_meshes,
        vec![10],
        "Enabled<RenderEnabled> visits ONLY the visible (bit-set) row"
    );

    let disabled_meshes: Vec<u32> = ecs
        .query::<&MeshHandle, Disabled<RenderEnabled>>()
        .iter()
        .map(|m| m.0)
        .collect();
    assert_eq!(
        disabled_meshes,
        vec![20],
        "Disabled<RenderEnabled> visits ONLY the hidden (bit-clear) row"
    );
}
