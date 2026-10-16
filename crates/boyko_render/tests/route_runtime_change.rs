//! **RD2 / TRANSPARENCY R0a — the route is structural; runtime changes are explicit.**
//!
//! Tests 4 and 5 of the rung: the debug `check_route_markers` fires on a marker set that
//! disagrees with `Material::route()` (a hand-inserted mismatch, and a route-defining edit that
//! skipped `reassign_route`), and `reassign_route` is the remedy in both directions.
//!
//! The check is `#[cfg(debug_assertions)]` (release builds compile it out), so is this file.
//! These tests run the system directly; `boyko_app/tests/route_markers_check_is_wired.rs` is the
//! gate on the production registration.
#![cfg(debug_assertions)]

use boyko_ecs::ecs::core::asset::{Assets, Handle};
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Commands, Res};

use boyko_render::route::{check_route_markers, reassign_route};
use boyko_render::{AlphaMasked, AlphaMode, BlendClass, Material, MaterialRoute, Translucent};
use boyko_scene::render_caps::MaterialHandle;

/// A world with the default material at slot 0 and one opaque material at slot 1.
fn world() -> (EcsMaster, Handle<Material>) {
    let mut ecs = EcsMaster::new();
    let mut materials = Assets::<Material>::with_reserved(4);
    let default = materials.add(Material::default());
    materials.pin(default.index());
    let opaque = materials.add(Material::new([0.5, 0.5, 0.5, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0));
    ecs.insert_resource(materials);
    let _ = MaterialHandle::component_id();
    (ecs, opaque)
}

fn spawn_with(ecs: &mut EcsMaster, material: Handle<Material>, translucent: bool) -> Entity {
    let handle = MaterialHandle::from_handle(material);
    ecs.run_system(move |mut cmds: Commands| -> Entity {
        let mut e = cmds.spawn(handle);
        if translucent {
            e.insert(Translucent);
        }
        e.id()
    })
}

/// The markers `entity` carries now, as the route they spell.
fn markers(ecs: &mut EcsMaster) -> Vec<(bool, bool)> {
    ecs.run_system(
        |q: Query<(&MaterialHandle, Option<&Translucent>, Option<&AlphaMasked>)>| -> Vec<(bool, bool)> {
            q.iter().map(|(_, t, m)| (t.is_some(), m.is_some())).collect()
        },
    )
}

fn edit(ecs: &mut EcsMaster, material: Handle<Material>, alpha: AlphaMode) {
    let materials = ecs.resource_mut::<Assets<Material>>();
    materials.get_mut(material).expect("the material row is live").alpha = alpha;
}

fn reassign(ecs: &mut EcsMaster, entity: Entity, material: Handle<Material>) {
    ecs.run_system(move |mut cmds: Commands, materials: Res<Assets<Material>>| {
        let m = *materials.get(material).expect("the material row is live");
        reassign_route(&mut cmds, entity, &m);
    });
}

/// Test 4: a `Translucent` marker hand-inserted on an entity whose material is opaque.
#[test]
#[should_panic(expected = "reassign_route")]
fn check_fires_on_a_hand_inserted_mismatch() {
    let (mut ecs, opaque) = world();
    spawn_with(&mut ecs, opaque, true);
    ecs.run_system(check_route_markers);
}

/// The control for test 4: agreeing markers are silent.
#[test]
fn check_is_silent_when_markers_agree() {
    let (mut ecs, opaque) = world();
    spawn_with(&mut ecs, opaque, false);
    ecs.run_system(check_route_markers);
}

/// Test 5, first half: an Opaque material edited to `Blend` at runtime, with no
/// `reassign_route` call — the check fires and names the remedy.
#[test]
#[should_panic(expected = "reassign_route")]
fn a_runtime_route_edit_without_reassign_fires() {
    let (mut ecs, opaque) = world();
    spawn_with(&mut ecs, opaque, false);
    ecs.run_system(check_route_markers);
    edit(&mut ecs, opaque, AlphaMode::Blend(BlendClass::PremulOver));
    ecs.run_system(check_route_markers);
}

/// Test 5, second half: `reassign_route` after the edit adds `Translucent` and the check is
/// silent; the reverse edit (Blend → Opaque) removes it; a `Mask` edit swaps to `AlphaMasked`.
#[test]
fn reassign_route_moves_the_entity_both_ways() {
    let (mut ecs, opaque) = world();
    let e = spawn_with(&mut ecs, opaque, false);

    edit(&mut ecs, opaque, AlphaMode::Blend(BlendClass::PremulOver));
    reassign(&mut ecs, e, opaque);
    assert_eq!(markers(&mut ecs), vec![(true, false)], "RED: reassign_route missed Translucent");
    ecs.run_system(check_route_markers);

    edit(&mut ecs, opaque, AlphaMode::Mask { cutoff: 0.5 });
    reassign(&mut ecs, e, opaque);
    assert_eq!(markers(&mut ecs), vec![(false, true)], "Blend → Mask swaps the marker");
    ecs.run_system(check_route_markers);

    edit(&mut ecs, opaque, AlphaMode::Opaque);
    reassign(&mut ecs, e, opaque);
    assert_eq!(markers(&mut ecs), vec![(false, false)], "→ Opaque removes every marker");
    ecs.run_system(check_route_markers);

    let m = *ecs.resource::<Assets<Material>>().get(opaque).expect("live");
    assert_eq!(m.route(), MaterialRoute::Opaque);
}
