//! The R2 demonstrator: opens a window, clears it to the engine's dark-neutral
//! color every frame, exits on Escape or window close, and tears down cleanly
//! (device singleton ended exactly once — host plan D2).
//!
//! Run: `cargo run -p boyko-app --example clear` — no Vulkan SDK required
//! (the shipped runner does not request the validation layer, so an ABSENT
//! layer — the common case on end-user machines — cannot fail the boot).

use boyko_app::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko clear", 800, 600));
    app.run();
}
