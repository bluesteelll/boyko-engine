//! The `eframe::App` implementation: egui UI + wgpu paint-callback registration
//! (plan §4 / D14). Waves 0-2 render a static instanced-quad scene with a
//! control-panel placeholder; ECS-driven state lands in Wave 3.

use eframe::CreationContext;
use eframe::egui;
use rand::Rng;

use crate::render::instance::GpuInstance;
use crate::render::{RenderCallback, RenderResources, WORLD_HALF_EXTENT};

/// Number of instanced quads in the Wave 2 static scene (plan §10 Wave 2:
/// "~50-100k quads"). Replaced by the ECS entity count in Wave 3.
const DEMO_INSTANCE_COUNT: usize = 75_000;

/// The demo application state.
///
/// Waves 0-2: holds a precomputed static instance scene. The GPU resources live
/// in egui's `callback_resources` type-map (inserted in [`DemoApp::new`]), not
/// here, because the `'static` paint callback reads them from there (D6 / D8).
pub struct DemoApp {
    /// Precomputed instance scene (Wave 2 hardcoded data). Cloned into each
    /// frame's paint callback. Wave 3 replaces this with an ECS-sourced upload.
    scene: Vec<GpuInstance>,
    /// Smoothed frames-per-second estimate for the stats label.
    fps: f32,
}

impl DemoApp {
    /// Builds the app: creates the wgpu render resources, registers them in egui's
    /// callback type-map, and generates the static instance scene.
    ///
    /// # Panics
    /// Panics if `wgpu_render_state` is absent. eframe always provides it when
    /// built with the `wgpu` feature (the only configuration this binary uses), so
    /// its absence is an unrecoverable setup error rather than a runtime
    /// condition.
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("invariant: eframe is built with the wgpu backend");

        let resources = RenderResources::new(&render_state.device, render_state.target_format);

        // Store GPU resources in egui's per-renderer type-map so the `'static`
        // paint callback can reach them (plan D8).
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(resources);

        Self {
            scene: generate_scene(DEMO_INSTANCE_COUNT),
            fps: 0.0,
        }
    }

    /// Draws the floating control/stats window (plan D14: an always-visible FPS
    /// label; the frame-time plot is additive in Wave 4).
    fn draw_controls(&self, ctx: &egui::Context) {
        egui::Window::new("boyko_demo")
            .default_pos([16.0, 16.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("boyko_demo");
                ui.separator();
                ui.label(format!("FPS: {:.1}", self.fps));
                ui.label(format!("Instances: {}", self.scene.len()));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Wave 2: static instanced quads (no ECS yet)").weak(),
                );
            });
    }
}

impl eframe::App for DemoApp {
    // eframe 0.34 made `App::ui` the primary entry point (`App::update` is
    // deprecated with an empty default body). `ui` hands us the whole app area as a
    // `Ui` equivalent to a top-level `CentralPanel`; side panels and windows go on
    // top via `ui.ctx()`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Smooth the FPS readout from egui's stable frame delta (wasm-safe; no
        // `std::time::Instant`).
        let dt = ctx.input(|i| i.stable_dt).max(f32::EPSILON);
        let instant_fps = 1.0 / dt;
        // Exponential moving average to keep the label readable.
        self.fps = if self.fps == 0.0 {
            instant_fps
        } else {
            self.fps * 0.9 + instant_fps * 0.1
        };

        // Fill the whole app area with the instanced scene (plan D14). The paint
        // callback draws under any floating panels added afterward.
        let rect = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        let viewport_px = [rect.width() * ppp, rect.height() * ppp];

        // Register the wgpu paint callback for this rect. The callback is
        // `'static`: it owns a snapshot of the scene rather than borrowing `self`
        // (D6 / D8 / plan H4).
        let callback = RenderCallback {
            viewport_px,
            instances: self.scene.clone(),
        };
        let paint_callback = eframe::egui_wgpu::Callback::new_paint_callback(rect, callback);
        ui.painter().add(paint_callback);

        self.draw_controls(&ctx);

        // Keep animating so the FPS label stays live and Wave 3's movement runs
        // every frame.
        ctx.request_repaint();
    }
}

/// Generates a random scatter of `count` instanced quads within the world bounds,
/// each with a random color and a small random size (plan §10 Wave 2: a hardcoded
/// scatter de-risks instancing with no ECS).
fn generate_scene(count: usize) -> Vec<GpuInstance> {
    let mut rng = rand::rng();
    let mut scene = Vec::with_capacity(count);
    for _ in 0..count {
        let x = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let y = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let scale = rng.random_range(0.4..1.0);
        let color = [
            rng.random_range(40..=255),
            rng.random_range(40..=255),
            rng.random_range(40..=255),
            255,
        ];
        scene.push(GpuInstance::new([x, y], scale, color));
    }
    scene
}
