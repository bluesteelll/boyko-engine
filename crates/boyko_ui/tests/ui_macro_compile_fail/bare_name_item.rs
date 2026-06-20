//! A bare `#title` as a body item (not inside a field) -> "a node reference …".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout::default(),
        #title
    };
}

fn main() {}
