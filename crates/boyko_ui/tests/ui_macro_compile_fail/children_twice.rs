//! Two `children:` clauses in one body -> "appears twice".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout::default(),
        children: [ { UiLayout::default() } ],
        children: [ { UiLayout::default() } ]
    };
}

fn main() {}
