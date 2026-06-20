//! A `{ .. }` child node among items (not in a `children:` clause) -> error.
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout::default(),
        { UiLayout::default() }
    };
}

fn main() {}
