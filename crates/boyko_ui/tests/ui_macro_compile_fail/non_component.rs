//! A non-`Component` type as a UiLayout-less node still fails validation first;
//! to exercise the `Bundle` bound at type-check, pair a valid `UiLayout` with a
//! non-`Component` literal so the node passes validation and the bad literal
//! reaches the `.insert::<B: Bundle>` bound -> E0277 at the user's token.
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;

struct NotAComponent {
    _x: u32,
}

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout::default(),
        NotAComponent { _x: 1 }
    };
}

fn main() {}
