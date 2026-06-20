//! A node with components but no `UiLayout` -> "requires a `UiLayout`".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::ContentSize;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        ContentSize { width: 10.0, height: 10.0 }
    };
}

fn main() {}
