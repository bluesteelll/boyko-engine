//! A typo'd field name in a component literal -> E0560 at the user's token
//! (span-forwarding: the literal is emitted verbatim inside the expansion).
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout { widht: boyko_ui::units::Unit::Px(1.0), ..UiLayout::default() }
    };
}

fn main() {}
