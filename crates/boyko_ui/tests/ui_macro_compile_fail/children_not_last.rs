//! `children:` followed by another component -> "must be the last clause".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::{UiLayout, UiRoot};
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout::default(),
        children: [ { UiLayout::default() } ],
        UiRoot
    };
}

fn main() {}
