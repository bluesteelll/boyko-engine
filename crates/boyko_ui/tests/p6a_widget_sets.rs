//! GATE 1 — each of the 6 widgets spawns EXACTLY its intended component set.
//!
//! For every widget we spawn it via its preset bundle (the hand-spawn convenience
//! layer) and assert, by archetype membership (`has_component`), that EXACTLY the
//! intended components are present and that the widget-distinguishing marker of a
//! DIFFERENT widget is absent. This pins each widget's component substrate so a
//! future change that adds/drops a component to a bundle is caught.
//!
//! The canonical authorable form is the explicit component list (the `.ui`/`ui!`
//! equivalence gate, `p6a_equivalence.rs`, proves the bundle ≡ explicit set);
//! here we assert the SET each form materializes.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_ui::binding::{UiTextBuffer, UiValue};
use boyko_ui::bundles::{
    BarBundle, ButtonBundle, GridBundle, ImageBundle, LabelBundle, PanelBundle,
};
use boyko_ui::components::{
    Bar, Button, ComputedRect, ContentSize, UiBackground, UiGrid, UiImage, UiLayout,
};
use boyko_ui::interaction::action::OnClick;
use boyko_ui::interaction::components::{Focusable, Interaction};
use boyko_ui::text::UiText;

/// Spawns one bundle, harvesting the live handle.
fn spawn_bundle<B, F>(world: &mut EcsMaster, make: F) -> Entity
where
    B: boyko_ecs::ecs::core::bundle::bundle::Bundle + Send + Sync + 'static,
    F: FnOnce() -> B + Send + Sync + 'static,
{
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let make = Mutex::new(Some(make));
    world.run_system(move |mut cmds: Commands| {
        let make = make.lock().unwrap().take().expect("once");
        let ec = cmds.spawn(make());
        *probe.lock().unwrap() = Some(ec.id());
    });
    let e = sink.lock().unwrap().expect("spawned handle");
    assert!(world.has_entity(e), "bundle entity is live");
    e
}

/// Asserts `e` HAS every id in `present` and LACKS every id in `absent`.
#[track_caller]
fn assert_exact_set(
    world: &EcsMaster,
    e: Entity,
    present: &[(&str, boyko_ecs::ecs::identifiers::primitives::ComponentId)],
    absent: &[(&str, boyko_ecs::ecs::identifiers::primitives::ComponentId)],
) {
    for (name, id) in present {
        assert!(world.has_component(e, *id), "widget must carry {name}");
    }
    for (name, id) in absent {
        assert!(!world.has_component(e, *id), "widget must NOT carry {name}");
    }
}

#[test]
fn panel_spawns_layout_rect_background_only() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle::<PanelBundle, _>(&mut world, || PanelBundle {
        layout: UiLayout::default(),
        rect: ComputedRect::default(),
        background: UiBackground::default(),
    });
    assert_exact_set(
        &world,
        e,
        &[
            ("UiLayout", UiLayout::component_id()),
            ("ComputedRect", ComputedRect::component_id()),
            ("UiBackground", UiBackground::component_id()),
        ],
        &[
            // A Panel is NOT a Button / Bar / textful Label / Image.
            ("Button", Button::component_id()),
            ("Bar", Bar::component_id()),
            ("UiText", UiText::component_id()),
            ("UiImage", UiImage::component_id()),
            ("Interaction", Interaction::component_id()),
        ],
    );
}

#[test]
fn label_spawns_text_buffer_content_set() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle::<LabelBundle, _>(&mut world, || LabelBundle {
        layout: UiLayout::default(),
        rect: ComputedRect::default(),
        text: UiText::default(),
        buffer: UiTextBuffer::default(),
        content: ContentSize::default(),
    });
    assert_exact_set(
        &world,
        e,
        &[
            ("UiLayout", UiLayout::component_id()),
            ("ComputedRect", ComputedRect::component_id()),
            ("UiText", UiText::component_id()),
            ("UiTextBuffer", UiTextBuffer::component_id()),
            ("ContentSize", ContentSize::component_id()),
        ],
        &[
            ("UiBackground", UiBackground::component_id()),
            ("Button", Button::component_id()),
            ("Bar", Bar::component_id()),
            ("UiImage", UiImage::component_id()),
        ],
    );
}

#[test]
fn button_spawns_interactive_panel_set() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle::<ButtonBundle, _>(&mut world, || ButtonBundle {
        layout: UiLayout::default(),
        rect: ComputedRect::default(),
        background: UiBackground::default(),
        marker: Button,
        interaction: Interaction::None,
        focusable: Focusable::default(),
        on_click: OnClick(0),
    });
    assert_exact_set(
        &world,
        e,
        &[
            ("UiLayout", UiLayout::component_id()),
            ("ComputedRect", ComputedRect::component_id()),
            ("UiBackground", UiBackground::component_id()),
            ("Button", Button::component_id()),
            ("Interaction", Interaction::component_id()),
            ("Focusable", Focusable::component_id()),
            ("OnClick", OnClick::component_id()),
        ],
        &[
            ("Bar", Bar::component_id()),
            ("UiText", UiText::component_id()),
            ("UiImage", UiImage::component_id()),
            ("UiValue", UiValue::component_id()),
        ],
    );
}

#[test]
fn bar_track_spawns_marker_value_panel_set() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle::<BarBundle, _>(&mut world, || BarBundle {
        layout: UiLayout::default(),
        rect: ComputedRect::default(),
        background: UiBackground::default(),
        marker: Bar,
        value: UiValue::default(),
    });
    assert_exact_set(
        &world,
        e,
        &[
            ("UiLayout", UiLayout::component_id()),
            ("ComputedRect", ComputedRect::component_id()),
            ("UiBackground", UiBackground::component_id()),
            ("Bar", Bar::component_id()),
            ("UiValue", UiValue::component_id()),
        ],
        &[
            ("Button", Button::component_id()),
            ("UiText", UiText::component_id()),
            ("UiImage", UiImage::component_id()),
            ("Interaction", Interaction::component_id()),
            // The track is NOT the fill.
            ("BarFill", boyko_ui::components::BarFill::component_id()),
        ],
    );
}

#[test]
fn image_spawns_image_layout_set() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle::<ImageBundle, _>(&mut world, || ImageBundle {
        layout: UiLayout::default(),
        rect: ComputedRect::default(),
        image: UiImage::default(),
    });
    assert_exact_set(
        &world,
        e,
        &[
            ("UiLayout", UiLayout::component_id()),
            ("ComputedRect", ComputedRect::component_id()),
            ("UiImage", UiImage::component_id()),
        ],
        &[
            ("UiBackground", UiBackground::component_id()),
            ("Button", Button::component_id()),
            ("Bar", Bar::component_id()),
            ("UiText", UiText::component_id()),
        ],
    );
}

#[test]
fn grid_spawns_grid_config_set() {
    let mut world = EcsMaster::new();
    let e = spawn_bundle::<GridBundle, _>(&mut world, || GridBundle {
        layout: UiLayout {
            layout_type: boyko_ui::units::LayoutType::Grid,
            ..UiLayout::default()
        },
        rect: ComputedRect::default(),
        grid: UiGrid { columns: 2, rows: 2 },
    });
    assert_exact_set(
        &world,
        e,
        &[
            ("UiLayout", UiLayout::component_id()),
            ("ComputedRect", ComputedRect::component_id()),
            ("UiGrid", UiGrid::component_id()),
        ],
        &[
            ("UiBackground", UiBackground::component_id()),
            ("Button", Button::component_id()),
            ("Bar", Bar::component_id()),
            ("UiImage", UiImage::component_id()),
        ],
    );
    // The grid's layout_type is Grid.
    assert_eq!(
        world.get_component::<UiLayout>(e).unwrap().layout_type,
        boyko_ui::units::LayoutType::Grid,
        "GridBundle node lays out as a Grid"
    );
}
