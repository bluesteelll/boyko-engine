//! The `eframe::App` implementation: the single seam between the ECS sim and the
//! wgpu renderer (plan §4 / D8 / D14).
//!
//! `DemoApp` owns the [`EcsMaster`] world, the native [`SimRunner`] (which bundles
//! the thread pool, schedule, and fixed-timestep accumulator), and the GPU handles
//! needed for the per-frame upload. Each frame `ui` runs the pipeline in order:
//!
//! 1. maps the egui pointer into world space and writes [`InputState`];
//! 2. steps the simulation (`runner.step`) — the real multi-threaded schedule;
//! 3. uploads the `GpuInstance` column zero-copy via `for_each_chunk` into the
//!    shared instance buffer (plan D2/H4 — done here because the `'static` paint
//!    callback cannot borrow the world);
//! 4. registers the paint callback (which only draws `0..count`); and
//! 5. draws the egui control/stats panel.

use std::sync::Arc;

use eframe::CreationContext;
use eframe::egui;
use eframe::egui_wgpu::wgpu;
use rand::Rng;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use crate::render::camera::CameraUniform;
use crate::render::instance::GpuInstance;
use crate::render::{MAX_INSTANCES, RenderCallback, RenderResources, WORLD_HALF_EXTENT};
use crate::sim::bundles::ParticleBundle;
use crate::sim::components::{ParticleTag, Position, Velocity};
use crate::sim::resources::{DeltaTime, InputState, SimParams};
use crate::sim::runner::SimRunner;

/// Number of particles spawned at startup (plan §1.2 MVP: 100k+). Spawned via
/// the direct single-entity `spawn` path (C1 — NOT `spawn_batch`, whose 8192/call
/// cap fails at this size), so the full population exists immediately with no
/// per-frame apply delay.
const STARTUP_PARTICLE_COUNT: usize = 100_000;

/// Initial speed range for spawned particles, in world units per second.
const INITIAL_SPEED: f32 = 40.0;

/// The demo application state (plan §4).
///
/// Holds the ECS world and the native runner. GPU handles are limited to what
/// the per-frame upload needs: the `queue` (a cheap refcounted handle) and the
/// shared `instance_buffer`. The pipeline and bind groups live in egui's
/// `callback_resources`, not here (plan D8).
pub struct DemoApp {
    /// The ECS world: entities, components, resources.
    world: EcsMaster,
    /// Native scheduler + fixed-timestep driver.
    runner: SimRunner,
    /// The work-stealing pool the schedule fans `par_iter` across. Held to keep
    /// it alive for the schedule's lifetime and as the canonical owner.
    _pool: Arc<ThreadPool>,
    /// wgpu queue for the per-frame instance upload. `wgpu::Queue` is `Clone`
    /// (refcounted internally), so cloning it out of eframe's render state is
    /// cheap and the clone shares the same underlying queue.
    queue: wgpu::Queue,
    /// Instance buffer shared with [`RenderResources`]; the upload target.
    instance_buffer: Arc<wgpu::Buffer>,
    /// Smoothed frames-per-second estimate for the stats label.
    fps: f32,
}

impl DemoApp {
    /// Builds the app: creates the wgpu render resources, the ECS world (with its
    /// resources and the startup particle population), and the native runner.
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

        let (resources, instance_buffer) =
            RenderResources::new(&render_state.device, render_state.target_format);

        // Store GPU pipeline/bind-group resources in egui's per-renderer type-map
        // so the `'static` paint callback can reach them (plan D8).
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(resources);

        // `wgpu::Queue` is refcounted internally and `Clone`; cloning eframe's
        // queue is a cheap handle copy sharing the same underlying queue, which
        // the app uses for the per-frame instance upload.
        let queue = render_state.queue.clone();

        // Build the world: resources first, then the startup population. One
        // archetype (the particle bundle), so an archetype capacity of 1 is
        // enough; the entity capacity is sized to the startup population.
        let mut world = EcsMaster::with_capacity(STARTUP_PARTICLE_COUNT, 1);
        world.insert_resource(DeltaTime(crate::sim::runner::FIXED_DT));
        world.insert_resource(InputState::default());
        world.insert_resource(SimParams::default());
        spawn_particles(&mut world, STARTUP_PARTICLE_COUNT);

        // Native: a real multi-threaded pool sized to the machine (plan D10).
        let pool = ThreadPoolBuilder::new().build();
        let runner = SimRunner::new(Arc::clone(&pool), &mut world);

        Self {
            world,
            runner,
            _pool: pool,
            queue,
            instance_buffer,
            fps: 0.0,
        }
    }

    /// Maps the egui pointer into world space and writes [`InputState`] for the
    /// upcoming sim step (plan §7).
    ///
    /// `rect` is the scene rect in logical points; `ppp` is points-per-pixel. The
    /// well is suppressed when egui is using the pointer (e.g. over the panel) so
    /// dragging a slider does not also fling particles.
    fn update_input(&mut self, ctx: &egui::Context, rect: egui::Rect, ppp: f32) {
        let wants_pointer = ctx.egui_wants_pointer_input();
        let (pointer_pos, primary_held) =
            ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));

        // The well engages only when the pointer is over the scene and egui is
        // not consuming it (so dragging a slider does not also fling particles).
        let over_scene = pointer_pos.is_some_and(|pos| !wants_pointer && rect.contains(pos));

        let cursor_world = if over_scene {
            // `over_scene` guarantees `Some`; the pointer is inside the rect.
            let pos = pointer_pos.expect("invariant: over_scene implies a pointer position");
            // Logical point within the rect -> physical pixels within the
            // viewport, then invert the camera projection.
            let px = (pos.x - rect.min.x) * ppp;
            let py = (pos.y - rect.min.y) * ppp;
            CameraUniform::screen_to_world(
                px,
                py,
                rect.width() * ppp,
                rect.height() * ppp,
                WORLD_HALF_EXTENT,
                WORLD_HALF_EXTENT,
            )
        } else {
            None
        };

        let input = self.world.resource_mut::<InputState>();
        input.cursor_world = cursor_world;
        input.primary_down = over_scene && primary_held;
    }

    /// Uploads the live `GpuInstance` column into the instance buffer with no
    /// intermediate AoS copy (the headline zero-copy path, plan D2/D5/H4) and
    /// returns the total instance count drawn.
    ///
    /// `&GpuInstance` needs no change detection, so the direct `query()` API is
    /// valid here (plan §9 G2). `for_each_chunk` yields one contiguous column
    /// slice per archetype; each is `cast_slice`d straight into the GPU buffer.
    fn upload_instances(&mut self) -> u32 {
        let queue = &self.queue;
        let buffer = &self.instance_buffer;
        let stride = size_of::<GpuInstance>() as u64;
        let capacity_bytes = MAX_INSTANCES * stride;

        let mut byte_offset: u64 = 0;
        self.world
            .query::<&GpuInstance, ()>()
            .for_each_chunk(|chunk: &[GpuInstance]| {
                if chunk.is_empty() {
                    return;
                }
                let bytes: &[u8] = bytemuck::cast_slice(chunk);
                let len = bytes.len() as u64;
                // The buffer is sized at MAX_INSTANCES; never write past it
                // (plan §11.4 upload invariant). The runtime guard is strictly
                // stronger than a `debug_assert!` — it protects release builds
                // from a GPU buffer overrun if the entity count ever exceeds the
                // cap (the paint callback clamps the draw count to match).
                if byte_offset + len > capacity_bytes {
                    return;
                }
                queue.write_buffer(buffer, byte_offset, bytes);
                byte_offset += len;
            });

        (byte_offset / stride) as u32
    }

    /// Draws the floating control/stats window (plan D14). The Wave-3 MVP shows
    /// the live entity count and FPS; sliders and the frame-time plot are
    /// additive in Wave 4.
    fn draw_controls(&self, ctx: &egui::Context, instance_count: u32) {
        egui::Window::new("boyko_demo")
            .default_pos([16.0, 16.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("boyko_demo");
                ui.separator();
                ui.label(format!("FPS: {:.1}", self.fps));
                ui.label(format!("Entities: {}", self.world.entity_count()));
                ui.label(format!("Instances drawn: {instance_count}"));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Hold the left mouse button to pull particles").weak(),
                );
            });
    }
}

impl eframe::App for DemoApp {
    // eframe 0.34 made `App::ui` the primary entry point. `ui` hands us the whole
    // app area; side panels and windows go on top via `ui.ctx()`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Smooth the FPS readout from egui's stable frame delta (wasm-safe; no
        // `std::time::Instant`).
        let dt = ctx.input(|i| i.stable_dt).max(f32::EPSILON);
        let instant_fps = 1.0 / dt;
        self.fps = if self.fps == 0.0 {
            instant_fps
        } else {
            self.fps * 0.9 + instant_fps * 0.1
        };

        let rect = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();

        // 1. Pointer -> InputState for this step.
        self.update_input(&ctx, rect, ppp);

        // 2. Advance the simulation (real multi-threaded schedule, fixed dt).
        self.runner.step(&mut self.world, dt);

        // 3. Zero-copy upload of the GpuInstance column (plan D2/H4).
        let instance_count = self.upload_instances();

        // 4. Register the paint callback for this rect. It is `'static`: it owns
        // only the viewport size and the instance count — never a borrow of the
        // world (D6 / D8 / plan H4).
        let viewport_px = [rect.width() * ppp, rect.height() * ppp];
        let callback = RenderCallback {
            viewport_px,
            instance_count,
        };
        let paint_callback = eframe::egui_wgpu::Callback::new_paint_callback(rect, callback);
        ui.painter().add(paint_callback);

        // 5. Controls on top of the scene.
        self.draw_controls(&ctx, instance_count);

        // Keep animating so the sim runs every frame and the FPS label stays live.
        ctx.request_repaint();
    }
}

/// Spawns `count` particles scattered across the world box with random initial
/// velocities, using the direct `create_entity` path (C1 / plan §9 G5).
///
/// There is no `world.spawn(bundle)` on `EcsMaster` — the bundle spawn path is
/// `Commands::spawn` (deferred, system-only) which is unavailable at setup, and
/// `spawn_batch` caps at 8192/call. The direct path is: resolve the bundle's
/// archetype once via `bundle_archetype_id_for` (which registers it on the first
/// call), then `create_entity(archetype, &[(ComponentId, &[u8])])` per entity —
/// no batch cap, no one-frame apply delay. Component bytes come from
/// `bytemuck::bytes_of` (every component here is `Pod`), so the spawn is
/// `unsafe`-free.
fn spawn_particles(world: &mut EcsMaster, count: usize) {
    // Resolve (and register on first call) the particle archetype once.
    let archetype = world.bundle_archetype_id_for::<ParticleBundle>();

    // Component ids are stable for the process; resolve them once outside the
    // loop so each spawn skips the `OnceLock` load.
    let pos_id = Position::component_id();
    let vel_id = Velocity::component_id();
    let gpu_id = GpuInstance::component_id();
    let tag_id = ParticleTag::component_id();

    let mut rng = rand::rng();
    for _ in 0..count {
        let x = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let y = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let vx = rng.random_range(-INITIAL_SPEED..INITIAL_SPEED);
        let vy = rng.random_range(-INITIAL_SPEED..INITIAL_SPEED);

        let pos = Position { x, y };
        let vel = Velocity { x: vx, y: vy };
        // Filled by `sync_gpu_instance` on the first step; a sane initial value
        // avoids a one-frame flash of zeroed instances.
        let gpu = GpuInstance::new([x, y], 0.6, [80, 160, 255, 255]);
        let tag = ParticleTag(0);

        world
            .create_entity(
                archetype,
                &[
                    (pos_id, bytemuck::bytes_of(&pos)),
                    (vel_id, bytemuck::bytes_of(&vel)),
                    (gpu_id, bytemuck::bytes_of(&gpu)),
                    (tag_id, bytemuck::bytes_of(&tag)),
                ],
            )
            .expect("invariant: create_entity with the resolved archetype's full component set");
    }
}
