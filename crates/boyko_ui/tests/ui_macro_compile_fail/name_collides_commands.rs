//! A `#cmds` name collides with the default commands binding -> error.
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        #cmds { UiLayout::default() }
    };
}

fn main() {}
