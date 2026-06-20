//! A wrong-type literal whose head path is `UiLayout` but whose VALUE is not a
//! `UiLayout` lands in the `UiNodeBundle.layout` field. With `quote_spanned!` at
//! the bundle-field site, the type mismatch must point at the user's literal, not
//! at `UiNodeBundle`. Here the set contains both `UiLayout`(head) + `ComputedRect`
//! so the canonical bundle path is taken; the `UiLayout` literal has a bad field
//! type (a `&str` where a `Unit` is expected), forcing the mismatch at the user's
//! token inside the `layout:` field position.
use boyko_ecs::ecs::core::system::Commands;
use boyko_ui::components::{ComputedRect, UiLayout};
use boyko_ui::prelude::ui;

fn build(mut cmds: Commands) {
    let _ = ui! {
        UiLayout { width: "not a unit", ..UiLayout::default() },
        ComputedRect::default()
    };
}

fn main() {}
