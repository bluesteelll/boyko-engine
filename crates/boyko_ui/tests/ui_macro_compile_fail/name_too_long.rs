//! A 61-byte `#name` (CAP is 60) -> "exceeds 60 bytes".
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    // 61 'a's — one over UiName::CAP.
    let _ = ui! {
        #aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa {
            UiLayout::default()
        }
    };
}

fn main() {}
