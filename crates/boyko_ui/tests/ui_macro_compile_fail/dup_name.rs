//! Two `#foo` declarations in one invocation -> "duplicate ui name".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        #foo {
            UiLayout::default(),
            children: [
                #foo { UiLayout::default() }
            ]
        }
    };
}

fn main() {}
