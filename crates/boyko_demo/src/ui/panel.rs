//! The floating control/stats window (plan §7 / D14 / Wave 4 / Wave 5).
//!
//! [`draw`] renders a single `egui::Window` containing the mode buttons, the
//! per-mode simulation sliders/toggles (which mutate the borrowed params in
//! place), live readouts, and a hand-rolled rolling frame-time plot fed from
//! [`FrameStats`]. It returns the [`Mode`] the user clicked, if any, so the app
//! can queue the state transition (`NextState<Mode>`) — the panel itself owns no
//! ECS state.
//!
//! ## Why the plot is hand-rolled (no `egui_plot`)
//! The plan (§7 / H2) calls for an `egui_plot::Plot` line. `eframe 0.34` pins
//! `egui 0.34`, but no `egui_plot 0.34` is published (crates.io has only 0.35.0
//! and 0.33.0). Adding `egui_plot 0.35` would pull a **second** `egui` copy into
//! the graph, producing the "two `egui::Ui` types" mismatch the plan explicitly
//! warns against (§2.2 H2). Rather than skew the egui version, the FPS history is
//! drawn directly with `egui::Painter` line segments — a few lines, one egui
//! version, zero extra dependencies. The data source is identical: the fixed-size
//! [`FrameStats`] ring (no per-frame allocation, plan §11.2).

use eframe::egui;

use crate::sim::modes::Mode;
use crate::sim::resources::{BoidParams, FrameStats, SimParams};

/// Height of the frame-time plot in logical points.
const PLOT_HEIGHT: f32 = 64.0;

/// Vertical-axis ceiling for the plot, in milliseconds. Frame times are drawn
/// relative to this; spikes above it clamp to the top edge. ~33 ms ≈ 30 FPS, so
/// a healthy 60 FPS trace sits in the lower half.
const PLOT_MAX_MS: f32 = 33.0;

/// The 60 FPS budget in milliseconds, drawn as a reference guide line.
const TARGET_FRAME_MS: f32 = 1000.0 / 60.0;

/// Lower bound the slider for `target_count` exposes.
const TARGET_COUNT_MIN: u32 = 0;

/// Upper bound for the `target_count` slider — the instance-buffer cap
/// (`render::MAX_INSTANCES`), so the slider can never request more than the GPU
/// buffer holds.
const TARGET_COUNT_MAX: u32 = crate::render::MAX_INSTANCES as u32;

/// Everything the panel reads/writes for one frame (plan §7).
///
/// Bundles the borrows so [`draw`]'s signature stays small. `sim`/`boids` are
/// `&mut` because the sliders mutate them in place (the app copies them back into
/// the world after `draw`); `stats` is read-only (the shell produces it). `mode`
/// is the active mode, used to highlight the selected button and show only that
/// mode's controls.
pub struct PanelState<'a> {
    /// The active simulation mode (drives button highlight + which controls show).
    pub mode: Mode,
    /// Particle tunables, mutated in place by the Particles-mode sliders.
    pub sim: &'a mut SimParams,
    /// Boid tunables, mutated in place by the Boids-mode sliders.
    pub boids: &'a mut BoidParams,
    /// Rolling frame/sim stats for the readouts + plot.
    pub stats: &'a FrameStats,
    /// Instances uploaded this frame (shown next to the entity count).
    pub instances_drawn: u32,
    /// `true` when the world hit the instance cap (surfaces the "at capacity"
    /// note for click-to-spawn).
    pub at_capacity: bool,
}

/// Draws the control/stats window for one frame (plan §7 / Wave 5).
///
/// Returns `Some(mode)` if the user clicked a mode button this frame (the app
/// queues the transition via `NextState<Mode>`); `None` otherwise. All slider
/// edits land directly in the borrowed `sim`/`boids` params.
pub fn draw(ctx: &egui::Context, state: PanelState<'_>) -> Option<Mode> {
    let PanelState {
        mode,
        sim,
        boids,
        stats,
        instances_drawn,
        at_capacity,
    } = state;

    let mut requested_mode = None;

    egui::Window::new("boyko_demo")
        .default_pos([16.0, 16.0])
        .resizable(false)
        .show(ctx, |ui| {
            requested_mode = mode_buttons(ui, mode);
            ui.separator();
            readouts(ui, stats, instances_drawn);
            ui.separator();
            plot(ui, stats);
            ui.separator();
            // Pause is global (the runner reads `SimParams.paused` in every
            // mode), so it lives above the per-mode split.
            ui.checkbox(&mut sim.paused, "pause simulation");
            ui.separator();
            // Per-mode controls: only the active mode's tunables are shown so the
            // panel stays focused (plan §7 "per-mode controls").
            match mode {
                Mode::Particles => particle_controls(ui, sim, at_capacity),
                Mode::Boids => boid_controls(ui, boids),
            }
        });

    requested_mode
}

/// Renders the mode selector buttons; returns the clicked mode, if any
/// (plan §7 / D15). The active mode's button is shown selected.
fn mode_buttons(ui: &mut egui::Ui, current: Mode) -> Option<Mode> {
    let mut requested = None;
    ui.horizontal(|ui| {
        ui.label("mode:");
        if ui
            .selectable_label(current == Mode::Particles, "Particles")
            .clicked()
        {
            requested = Some(Mode::Particles);
        }
        if ui
            .selectable_label(current == Mode::Boids, "Boids")
            .clicked()
        {
            requested = Some(Mode::Boids);
        }
    });
    requested
}

/// Live numeric readouts: FPS, frame/sim milliseconds, entity count, instances
/// (plan §7 readouts).
fn readouts(ui: &mut egui::Ui, stats: &FrameStats, instances_drawn: u32) {
    // FPS from the measured frame time; guard the divide for the first frame.
    let fps = if stats.frame_ms > f32::EPSILON {
        1000.0 / stats.frame_ms
    } else {
        0.0
    };
    ui.heading(format!("{fps:.0} FPS"));
    ui.label(format!(
        "frame: {:.2} ms   sim: {:.2} ms",
        stats.frame_ms, stats.sim_ms
    ));
    ui.label(format!("entities: {}", stats.entity_count));
    ui.label(format!("instances drawn: {instances_drawn}"));
}

/// Draws the rolling frame-time history as a polyline via [`egui::Painter`]
/// (hand-rolled in place of `egui_plot`; see the module docs).
///
/// Reads the fixed [`FrameStats`] ring directly — no allocation. Each sample maps
/// to an x position across the plot rect (oldest at the left) and a y position
/// scaled by [`PLOT_MAX_MS`] (clamped at the top for spikes). A guide line marks
/// the 60 FPS budget.
fn plot(ui: &mut egui::Ui, stats: &FrameStats) {
    let width = ui.available_width().max(1.0);
    let (rect, _response) =
        ui.allocate_exact_size(egui::vec2(width, PLOT_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();

    // Backdrop so the plot reads as a panel even when empty. The window's own
    // frame already delineates the area, so no extra border is drawn. Sharp
    // corners (`CornerRadius::ZERO`, a const) are fine for an inner backdrop and
    // sidestep the inference/clippy friction of an inline `same(2)` literal.
    painter.rect_filled(rect, egui::CornerRadius::ZERO, visuals.extreme_bg_color);

    // Maps a frame time in ms to a y coordinate in the rect (top = PLOT_MAX_MS,
    // bottom = 0 ms). Clamps so a spike past the ceiling pins to the top edge.
    let y_for_ms = |ms: f32| {
        let frac = (ms / PLOT_MAX_MS).clamp(0.0, 1.0);
        rect.bottom() - frac * rect.height()
    };

    // 60 FPS budget guide line.
    let target_y = y_for_ms(TARGET_FRAME_MS);
    painter.line_segment(
        [
            egui::pos2(rect.left(), target_y),
            egui::pos2(rect.right(), target_y),
        ],
        egui::Stroke::new(1.0, visuals.weak_text_color()),
    );

    // Need at least two samples to draw a segment.
    let count = stats.len();
    if count < 2 {
        return;
    }

    // X step between consecutive samples across the full width; oldest at left.
    let dx = rect.width() / (count - 1) as f32;
    let line_color = visuals.hyperlink_color;

    // Walk the ring oldest-first, drawing a segment between each adjacent pair.
    // No allocation: we keep only the previous point and step the iterator.
    let mut prev: Option<egui::Pos2> = None;
    for (i, ms) in stats.iter_chronological().enumerate() {
        let x = rect.left() + dx * i as f32;
        let p = egui::pos2(x, y_for_ms(ms));
        if let Some(prev_p) = prev {
            painter.line_segment([prev_p, p], egui::Stroke::new(1.5, line_color));
        }
        prev = Some(p);
    }
}

/// The Particles-mode slider/toggle controls that mutate [`SimParams`]
/// (plan §7 control table).
fn particle_controls(ui: &mut egui::Ui, params: &mut SimParams, at_capacity: bool) {
    // Pause is drawn above the per-mode split (it is global); here only the
    // particle-specific gravity-well toggle.
    ui.checkbox(&mut params.gravity_enabled, "mouse gravity well");

    ui.add_space(4.0);

    // Sliders write SimParams in place; ranges are tuned for the ±100 world box.
    ui.add(egui::Slider::new(&mut params.gravity, 0.0..=5_000.0).text("well strength"));
    ui.add(egui::Slider::new(&mut params.max_speed, 10.0..=600.0).text("max speed"));
    // Damping is a per-second retention factor in (0, 1]; 1.0 = frictionless.
    ui.add(egui::Slider::new(&mut params.damping, 0.80..=1.0).text("damping"));
    ui.add(egui::Slider::new(&mut params.particle_size, 0.1..=3.0).text("particle size"));

    ui.add_space(4.0);

    ui.add(
        egui::Slider::new(&mut params.target_count, TARGET_COUNT_MIN..=TARGET_COUNT_MAX)
            .text("target count")
            .logarithmic(true),
    );
    ui.add(egui::Slider::new(&mut params.spawn_burst, 1..=20_000).text("click spawns N"));

    ui.add_space(4.0);

    if at_capacity {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "at capacity — click spawn disabled",
        );
    } else {
        ui.label(egui::RichText::new("click the field to spawn a burst").weak());
    }
    ui.label(egui::RichText::new("hold left mouse to pull particles").weak());
}

/// The Boids-mode slider controls that mutate [`BoidParams`] (plan §7 / Wave 5:
/// "boid weights: separation/alignment/cohesion/radius").
fn boid_controls(ui: &mut egui::Ui, params: &mut BoidParams) {
    // `pause` lives on SimParams (shared across modes); the Boids panel exposes
    // only the flocking weights + radius + speed.
    ui.add(egui::Slider::new(&mut params.radius, 1.0..=20.0).text("neighbor radius"));
    ui.add(egui::Slider::new(&mut params.separation, 0.0..=80.0).text("separation"));
    ui.add(egui::Slider::new(&mut params.alignment, 0.0..=40.0).text("alignment"));
    ui.add(egui::Slider::new(&mut params.cohesion, 0.0..=40.0).text("cohesion"));
    ui.add(egui::Slider::new(&mut params.max_speed, 10.0..=200.0).text("max speed"));

    ui.add_space(4.0);
    ui.label(egui::RichText::new("boids flock via separation / alignment / cohesion").weak());
    ui.label(egui::RichText::new("larger radius = denser neighborhoods, slower").weak());
}
