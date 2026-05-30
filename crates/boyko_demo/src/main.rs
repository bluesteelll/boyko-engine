//! `boyko_demo` — interactive showcase for the boyko-engine ECS.
//!
//! Waves 0-2 cover the rendering foundation only: an `eframe` + `egui` + `wgpu`
//! application that draws instanced quads in a single draw call. ECS wiring
//! (components, movement, sync-to-instances) arrives in Wave 3.
//!
//! The native entry point is gated behind `cfg(not(target_arch = "wasm32"))` so a
//! wasm32 entry can be added later without restructuring (plan §4 / OQ1).

mod app;
mod render;

use app::DemoApp;

/// Window title and `eframe` app id.
const APP_NAME: &str = "boyko_demo";

/// Native entry point. Boots `eframe` with the wgpu backend and hands control to
/// [`DemoApp`].
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    // `env_logger` surfaces wgpu/winit diagnostics (adapter selection, validation
    // errors). Controlled via `RUST_LOG`; the wgpu backends are noisy at `info`,
    // so they are clamped to `warn` by default.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(|cc| Ok(Box::new(DemoApp::new(cc)))),
    )
}
