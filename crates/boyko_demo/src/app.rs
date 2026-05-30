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
use std::time::Instant;

use eframe::CreationContext;
use eframe::egui;
use eframe::egui_wgpu::wgpu;
use rand::Rng;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use crate::render::camera::CameraUniform;
use crate::render::instance::GpuInstance;
use crate::render::{MAX_INSTANCES, RenderCallback, RenderResources, WORLD_HALF_EXTENT};
use crate::sim::bundles::ParticleBundle;
use crate::sim::components::{ParticleTag, Position, Velocity};
use crate::sim::resources::{DeltaTime, FrameStats, InputState, SimParams};
use crate::sim::runner::SimRunner;
use crate::ui;

/// Number of particles spawned at startup (plan §1.2 MVP: 100k+). Spawned via
/// the direct single-entity `spawn` path (C1 — NOT `spawn_batch`, whose 8192/call
/// cap fails at this size), so the full population exists immediately with no
/// per-frame apply delay.
const STARTUP_PARTICLE_COUNT: usize = 100_000;

/// Initial speed range for startup particles, in world units per second.
const INITIAL_SPEED: f32 = 40.0;

/// Speed range for particles spawned by a scene click, in world units per
/// second. Wider than [`INITIAL_SPEED`] so a click reads as an outward burst.
const CLICK_BURST_SPEED: f32 = 90.0;

/// Pre-resolved identifiers for the particle archetype, so a spawn (startup or a
/// runtime click burst) skips the per-entity registry lookups.
///
/// `bundle_archetype_id_for` registers the archetype on first call; the
/// `ComponentId`s are process-stable. Resolving them once and reusing the handle
/// keeps the direct `create_entity` spawn path (C1 / plan §9 G5) allocation- and
/// lookup-light in the click-spawn hot path.
#[derive(Clone, Copy)]
struct ParticleSpawner {
    archetype: ArchetypeId,
    pos_id: ComponentId,
    vel_id: ComponentId,
    gpu_id: ComponentId,
    tag_id: ComponentId,
}

impl ParticleSpawner {
    /// Resolves (and, for the archetype, registers on first call) every id the
    /// particle spawn path needs.
    fn resolve(world: &mut EcsMaster) -> Self {
        Self {
            archetype: world.bundle_archetype_id_for::<ParticleBundle>(),
            pos_id: Position::component_id(),
            vel_id: Velocity::component_id(),
            gpu_id: GpuInstance::component_id(),
            tag_id: ParticleTag::component_id(),
        }
    }

    /// Spawns one particle with the given state via the direct `create_entity`
    /// path. Returns whether the spawn succeeded; a `false` result means the
    /// underlying pool is full (the world hit capacity).
    fn spawn_one(&self, world: &mut EcsMaster, pos: Position, vel: Velocity) -> bool {
        // Filled by `sync_gpu_instance` on the next step; a sane initial value
        // avoids a one-frame flash of zeroed instances.
        let gpu = GpuInstance::new([pos.x, pos.y], 0.6, [80, 160, 255, 255]);
        let tag = ParticleTag(0);
        world
            .create_entity(
                self.archetype,
                &[
                    (self.pos_id, bytemuck::bytes_of(&pos)),
                    (self.vel_id, bytemuck::bytes_of(&vel)),
                    (self.gpu_id, bytemuck::bytes_of(&gpu)),
                    (self.tag_id, bytemuck::bytes_of(&tag)),
                ],
            )
            .is_ok()
    }
}

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
    /// Pre-resolved particle ids/archetype for runtime click-spawning.
    spawner: ParticleSpawner,
    /// Rolling frame/sim timing + entity-count history for the panel readouts and
    /// FPS plot (plan §7 / §11.2). A fixed-size ring — no per-frame allocation.
    stats: FrameStats,
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

        // Resolve the particle spawn ids once (registers the archetype), then
        // populate the startup cloud through the same direct path runtime
        // click-spawn reuses (C1 / plan §9 G5).
        let spawner = ParticleSpawner::resolve(&mut world);
        spawn_initial_particles(&mut world, &spawner, STARTUP_PARTICLE_COUNT);

        // Native: a real multi-threaded pool sized to the machine (plan D10).
        let pool = ThreadPoolBuilder::new().build();
        let runner = SimRunner::new(Arc::clone(&pool), &mut world);

        Self {
            world,
            runner,
            _pool: pool,
            queue,
            instance_buffer,
            spawner,
            stats: FrameStats::default(),
        }
    }

    /// Maps the egui pointer into world space and writes [`InputState`] for the
    /// upcoming sim step (plan §7).
    ///
    /// `rect` is the scene rect in logical points; `ppp` is points-per-pixel. The
    /// well is suppressed when egui is using the pointer (e.g. over the panel) so
    /// dragging a slider does not also fling particles.
    ///
    /// Returns the world position of a primary click made over the scene this
    /// frame, if any (plan §7 click-to-spawn). It is `Some` only when the click
    /// landed on the scene and egui did not want the pointer, so clicking a
    /// widget never spawns.
    fn update_input(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        ppp: f32,
    ) -> Option<[f32; 2]> {
        // egui 0.34 renamed the pointer-capture query to `egui_wants_pointer_input`
        // (the bare `wants_pointer_input` is deprecated). It is `true` when egui is
        // consuming the pointer (e.g. over a panel widget), which suppresses both
        // the gravity well and click-to-spawn.
        let wants_pointer = ctx.egui_wants_pointer_input();
        let (pointer_pos, primary_held, primary_clicked) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_down(),
                i.pointer.primary_clicked(),
            )
        });

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

        // A click that both started and ended over the scene spawns a burst
        // there. `cursor_world` already encodes the over-scene gate.
        if primary_clicked { cursor_world } else { None }
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

    /// Spawns a burst of particles at `world_pos`, clamped to remaining capacity
    /// (plan §7 click-to-spawn / D6 / M5).
    ///
    /// The burst size is `min(spawn_burst, MAX_INSTANCES - current_count)` so it
    /// can never exceed the instance buffer the renderer draws from. Each spawn
    /// uses the direct `create_entity` path (C1); if the underlying pool reports
    /// full mid-burst (`create_entity` errors), the loop stops early — the next
    /// frame's panel then shows "at capacity". Particles fan out from the click
    /// with small random velocities so the burst reads as an explosion.
    fn spawn_click_burst(&mut self, world_pos: [f32; 2]) {
        let burst = self.world.resource::<SimParams>().spawn_burst as u64;
        let live = self.world.entity_count() as u64;
        // Clamp against the instance-buffer cap (M5). `saturating_sub` yields 0
        // once the world is at or above the cap, making the burst a no-op.
        let room = MAX_INSTANCES.saturating_sub(live);
        let to_spawn = burst.min(room);
        if to_spawn == 0 {
            return;
        }

        let [cx, cy] = world_pos;
        let mut rng = rand::rng();
        for _ in 0..to_spawn {
            let vx = rng.random_range(-CLICK_BURST_SPEED..CLICK_BURST_SPEED);
            let vy = rng.random_range(-CLICK_BURST_SPEED..CLICK_BURST_SPEED);
            let pos = Position { x: cx, y: cy };
            let vel = Velocity { x: vx, y: vy };
            // Stop early if the pool fills mid-burst (capacity reached); the
            // panel surfaces it next frame via `at_capacity`.
            if !self.spawner.spawn_one(&mut self.world, pos, vel) {
                break;
            }
        }
    }

    /// Whether the world has reached the instance cap, so click-to-spawn is a
    /// no-op (drives the panel's "at capacity" note, plan D6/M5).
    fn at_capacity(&self) -> bool {
        self.world.entity_count() as u64 >= MAX_INSTANCES
    }
}

impl eframe::App for DemoApp {
    // eframe 0.34 made `App::ui` the primary entry point. `ui` hands us the whole
    // app area; side panels and windows go on top via `ui.ctx()`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Wall clock for the per-frame stats. `Instant` is the native shell's
        // timer; the wasm entry (Wave 7) will substitute a JS clock.
        let frame_start = Instant::now();

        let ctx = ui.ctx().clone();

        // Display delta drives the fixed-timestep accumulator (plan §6.7). egui's
        // `stable_dt` is wasm-safe (no `std::time::Instant`) and smoothed against
        // hitches.
        let dt = ctx.input(|i| i.stable_dt).max(f32::EPSILON);

        let rect = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();

        // 1. Pointer -> InputState for this step; capture a scene click position.
        let click_world = self.update_input(&ctx, rect, ppp);

        // 2. Click-to-spawn a burst at the cursor before stepping, so the new
        // particles integrate this frame (plan §7). Capacity-clamped (M5).
        if let Some(world_pos) = click_world {
            self.spawn_click_burst(world_pos);
        }

        // 3. Advance the simulation (real multi-threaded schedule, fixed dt),
        // timing just the sim so the panel can show sim ms vs total frame ms.
        let sim_start = Instant::now();
        self.runner.step(&mut self.world, dt);
        let sim_ms = sim_start.elapsed().as_secs_f32() * 1000.0;

        // 4. Zero-copy upload of the GpuInstance column (plan D2/H4).
        let instance_count = self.upload_instances();

        // 5. Register the paint callback for this rect. It is `'static`: it owns
        // only the viewport size and the instance count — never a borrow of the
        // world (D6 / D8 / plan H4).
        let viewport_px = [rect.width() * ppp, rect.height() * ppp];
        let callback = RenderCallback {
            viewport_px,
            instance_count,
        };
        let paint_callback = eframe::egui_wgpu::Callback::new_paint_callback(rect, callback);
        ui.painter().add(paint_callback);

        // 6. Record this frame's stats into the fixed ring (no allocation), then
        // draw the control panel. The panel mutates `SimParams` in place via the
        // borrow out of the world, so slider edits hit the next step directly
        // (plan §7). `frame_ms` uses the whole-`ui` span up to this point — the
        // dominant cost (sim + upload + draw record); the remaining egui paint is
        // negligible and not double-counted.
        let frame_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let entity_count = self.world.entity_count() as u32;
        self.stats.push(frame_ms, sim_ms, entity_count);

        let at_capacity = self.at_capacity();
        let params = self.world.resource_mut::<SimParams>();
        ui::panel::draw(&ctx, params, &self.stats, instance_count, at_capacity);

        // Keep animating so the sim runs every frame and the readouts stay live.
        ctx.request_repaint();
    }
}

/// Spawns the startup cloud: `count` particles scattered across the world box
/// with random initial velocities, via the pre-resolved [`ParticleSpawner`]
/// (the direct `create_entity` path, C1 / plan §9 G5).
///
/// There is no `world.spawn(bundle)` on `EcsMaster` — the bundle spawn path is
/// `Commands::spawn` (deferred, system-only) which is unavailable at setup, and
/// `spawn_batch` caps at 8192/call. The direct path resolves the archetype +
/// component ids once (in [`ParticleSpawner::resolve`]) then writes each entity
/// with no batch cap and no one-frame apply delay. Component bytes come from
/// `bytemuck::bytes_of` (every component here is `Pod`), so the spawn is
/// `unsafe`-free. The startup count is well under the cap, so a failed spawn
/// would be a setup bug rather than a capacity condition — hence the `expect`.
fn spawn_initial_particles(world: &mut EcsMaster, spawner: &ParticleSpawner, count: usize) {
    let mut rng = rand::rng();
    for _ in 0..count {
        let x = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let y = rng.random_range(-WORLD_HALF_EXTENT..WORLD_HALF_EXTENT);
        let vx = rng.random_range(-INITIAL_SPEED..INITIAL_SPEED);
        let vy = rng.random_range(-INITIAL_SPEED..INITIAL_SPEED);

        let ok = spawner.spawn_one(world, Position { x, y }, Velocity { x: vx, y: vy });
        assert!(
            ok,
            "invariant: startup spawn is well under capacity and must succeed"
        );
    }
}
