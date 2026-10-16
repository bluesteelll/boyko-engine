//! The material route markers (RD2 / TRANSPARENCY R0a).
//!
//! A material's rail — opaque, alpha-tested or sorted translucent — is a STRUCTURAL fact of
//! the entity that draws it: [`Translucent`] or [`AlphaMasked`] is present on the entity, or
//! neither is. The engine's glb spawn path
//! ([`spawn_glb_scene`](crate::loaders::glb_spawn::spawn_glb_scene)) derives it from
//! [`Material::route`](crate::material::Material::route) when it spawns the entity — the one
//! authority (09-10 D3).
//!
//! Both markers are ordinary components, never `EnableTag`s: a flag filter's `With<F>` matches
//! a disabled flag too, which is the wrong polarity for a rail split.
//!
//! Until the translucent rail lands (R1) nothing reads either marker: a `Translucent` entity
//! renders on the opaque rail exactly as it did before RD2.
//!
//! The route is fixed at spawn. A runtime edit of a route-defining field (`Material::alpha`,
//! `MaterialX::transmission`) does not move the entities that draw the row — the engine keeps no
//! reverse index from a material to its entities — so the code that edits one calls
//! [`reassign_route`] for the entities it changed. The debug-only [`check_route_markers`] (G3)
//! asserts the agreement every frame and names that remedy.

#[cfg(debug_assertions)]
use boyko_ecs::ecs::core::asset::{AssetLoadState, Assets};
#[cfg(debug_assertions)]
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
#[cfg(debug_assertions)]
use boyko_ecs::ecs::core::system::Res;
#[cfg(debug_assertions)]
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Component;
#[cfg(debug_assertions)]
use boyko_scene::render_caps::MaterialHandle;

use crate::material::{Material, MaterialRoute};

/// The entity's material routes to the sorted translucent rail (`alphaMode = BLEND` or
/// `transmission > 0`).
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Translucent;

/// The entity's material routes to the alpha-tested rail (`alphaMode = MASK`).
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct AlphaMasked;

const _: () = assert!(size_of::<Translucent>() == 0);
const _: () = assert!(size_of::<AlphaMasked>() == 0);

/// Makes `entity`'s route markers match `material.route()` — the supported path for a
/// route-defining edit at runtime (an Opaque → Blend fade, a transmission change).
///
/// The route is structural (the markers are archetype facts), and the engine keeps no reverse
/// index from a material row to the entities that reference it, so an `Assets<Material>` edit
/// cannot move those entities by itself: the code that edits a route-defining field calls this
/// for the entities it changed. Two deferred component edits (one insert, one remove — a remove
/// of an absent marker is a no-op), no allocation. A forgotten call fails `check_route_markers`
/// in a debug build; a release build renders the entity on its spawn-time rail.
#[inline]
pub fn reassign_route(commands: &mut Commands<'_>, entity: Entity, material: &Material) {
    let mut e = commands.entity(entity);
    match material.route() {
        MaterialRoute::Opaque => {
            e.remove::<Translucent>();
            e.remove::<AlphaMasked>();
        }
        MaterialRoute::Masked => {
            e.remove::<Translucent>();
            e.insert(AlphaMasked);
        }
        MaterialRoute::Translucent => {
            e.remove::<AlphaMasked>();
            e.insert(Translucent);
        }
    }
}

/// The route an entity's markers spell, or `None` when it carries both (no route has both).
#[inline]
pub fn marker_route(translucent: bool, masked: bool) -> Option<MaterialRoute> {
    match (translucent, masked) {
        (false, false) => Some(MaterialRoute::Opaque),
        (false, true) => Some(MaterialRoute::Masked),
        (true, false) => Some(MaterialRoute::Translucent),
        (true, true) => None,
    }
}

/// Debug-only (G3): every entity whose material row is `Loaded` carries exactly the markers
/// [`Material::route`] derives — the marker set and the material agree, or a second authority
/// wrote one of them (or a route-defining edit skipped [`reassign_route`]).
///
/// Registered by the host in `CoreSchedule::Main`, after `AssetValidateSet` (the rows it reads
/// are this frame's). A row that is not `Loaded` is skipped: its route is not yet a fact.
/// Release builds compile it out.
#[cfg(debug_assertions)]
#[allow(clippy::needless_pass_by_value, clippy::type_complexity)] // SystemParams are by value; the query IS the access contract
pub fn check_route_markers(
    q: Query<(&MaterialHandle, Option<&Translucent>, Option<&AlphaMasked>)>,
    materials: Res<Assets<Material>>,
) {
    for (entity, (handle, translucent, masked)) in q.iter_entities() {
        let row = u32::from(handle.0);
        if materials.state_of_index(row) != Some(AssetLoadState::Loaded) {
            continue;
        }
        let Some(material) = materials.get_by_index(row) else {
            continue;
        };
        let want = material.route();
        if marker_route(translucent.is_some(), masked.is_some()) != Some(want) {
            route_marker_mismatch(entity, row, want, translucent.is_some(), masked.is_some());
        }
    }
}

#[cfg(debug_assertions)]
#[cold]
#[inline(never)]
fn route_marker_mismatch(entity: EntityId, row: u32, want: MaterialRoute, translucent: bool, masked: bool) -> ! {
    panic!(
        "route markers disagree with the material: entity {entity:?} references material row {row} \
         whose route() is {want:?}, but it carries Translucent={translucent}, AlphaMasked={masked}. \
         If the material's route changed at runtime (alpha mode or transmission edited), call \
         `boyko_render::route::reassign_route` for its entities"
    );
}
