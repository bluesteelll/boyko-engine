//! `boyko_demo` binary entry point.
//!
//! A thin wrapper over the [`boyko_demo`] library crate: boots `eframe` with the
//! wgpu backend and hands control to [`boyko_demo::app::DemoApp`]. The crate is
//! library-shaped (see `lib.rs`) so the simulation can be tested headlessly.
//!
//! Two entry points behind `cfg(target_arch)`:
//!
//! * **Native** (`cfg(not(wasm32))`): `eframe::run_native` opens an OS window.
//! * **wasm** (`cfg(wasm32)`): `#[wasm_bindgen(start)]` → `WebRunner::start`
//!   mounts the app on the page's `<canvas>` (plan §8.2 / Wave 7). The wasm
//!   build runs the sim sequentially (plan D10); see `app`/`sim::runner`.

use boyko_demo::app::DemoApp;

// `JsCast::dyn_into` (the canvas-element downcast) lives on this trait; wasm-only.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast as _;

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

/// The DOM id of the `<canvas>` the web build mounts on. Must match the
/// `id="..."` on the canvas in `index.html`.
#[cfg(target_arch = "wasm32")]
const CANVAS_ID: &str = "boyko_demo_canvas";

/// wasm entry point (plan §8.2 / Wave 7).
///
/// Called automatically by the wasm-bindgen glue on module load
/// (`#[wasm_bindgen(start)]`). Installs the panic hook + logger so browser
/// devtools surface Rust panics/logs, then hands the canvas to
/// [`eframe::WebRunner`], which owns the render/event loop on the main thread
/// (plan D8 — GPU + surface are main-thread bound).
///
/// The future is driven by `wasm_bindgen_futures::spawn_local` because
/// `WebRunner::start` is `async` (it awaits adapter/device creation). A failure
/// to find the canvas or initialize wgpu is logged; there is no window to fall
/// back to on the web.
///
/// `web_sys` / `wasm_bindgen` are reached through eframe's re-exports (the
/// single-version discipline, same as `eframe::egui` / `eframe::egui_wgpu::wgpu`);
/// `wasm_bindgen_futures` is a direct dep pinned to eframe's minor (eframe does
/// not re-export it).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn wasm_start() {
    // Forward Rust panics to the browser console with a readable backtrace.
    console_error_panic_hook::set_once();
    // Route `log::*` records to the browser console (info-level default; wgpu
    // backends are clamped to `warn` to match the native filter).
    let _ = console_log::init_with_level(log::Level::Info);

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        // Resolve the target canvas element from the DOM by id.
        let document = eframe::web_sys::window()
            .expect("invariant: a browser window exists")
            .document()
            .expect("invariant: the window has a document");
        let canvas = document
            .get_element_by_id(CANVAS_ID)
            .unwrap_or_else(|| panic!("invariant: a <canvas id=\"{CANVAS_ID}\"> exists in the page"))
            .dyn_into::<eframe::web_sys::HtmlCanvasElement>()
            .expect("invariant: the element with the canvas id is a <canvas>");

        let result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(DemoApp::new(cc)))),
            )
            .await;

        if let Err(err) = result {
            // No window to fall back to on the web — surface the failure in the
            // console so a blank canvas is diagnosable.
            log::error!("boyko_demo failed to start: {err:?}");
        }
    });
}
