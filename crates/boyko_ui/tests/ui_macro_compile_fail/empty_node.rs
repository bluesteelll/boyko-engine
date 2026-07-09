//! An empty node body -> "needs at least one component".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        #empty { }
    };
}

fn main() {}
