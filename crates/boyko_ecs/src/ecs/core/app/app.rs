//! The [`App`] builder facade — a thin, additive composition layer over the
//! shipped [`EcsMaster`] + [`ScheduleBuilder`] + [`Schedule`] + [`ThreadPool`].
//!
//! `App` owns the world, stages one schedule, owns the worker pool, and drives
//! the per-frame loop. It adds **no** per-frame allocation, `dyn` dispatch,
//! atomic, or branch beyond `Schedule::run` itself: all of the
//! `Box<dyn Plugin>` / `Vec` / `TypeId` / tuple machinery is cold, setup-only
//! code that runs once during configuration.

use std::any::TypeId;
use std::sync::Arc;

use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use crate::ecs::core::app::app_exit::AppExit;
use crate::ecs::core::app::plugin::Plugin;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::schedule::schedule::Schedule;
use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;
use crate::ecs::core::state::states::States;
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::system::System;

/// A type-erased one-shot startup runnable, drained once in [`App::finish`].
/// Boxed because each startup system is a distinct monomorphized closure type.
type StartupSystem = Box<dyn FnOnce(&mut EcsMaster)>;

/// The application builder + runner.
///
/// Construct with [`App::new`] / [`App::with_threads`] / [`App::with_pool`],
/// configure it during the **config phase** (`add_systems`, `insert_resource`,
/// `init_state`, `add_plugins`, …), then drive it during the **run phase**
/// (`update`, `run_n`, `run`). The transition is performed once by
/// [`finish`](App::finish), which the runners call automatically.
///
/// # Threading
///
/// `App` is `!Send + !Sync` (inherited from [`EcsMaster`]): it is built and run
/// on a single dispatcher thread and never crosses a thread boundary. The only
/// other threads are the pool's workers, touched exclusively inside
/// [`Schedule::run`].
pub struct App {
    // Field order is for readability; no field's Drop observes another. App is
    // always dropped with no frame in flight (every `run*` takes &mut self ⇒
    // workers are idle and hold no borrow of world/schedule), so drop order is
    // not soundness-load-bearing.
    /// The world (arena + resources + entities). Lazy per Phase 12.6 — not
    /// eagerly memset on construction.
    world: EcsMaster,

    /// Staged schedule builder: `Some` during config, `None` after `finish`
    /// (which consumes it by value to produce the [`Schedule`]).
    builder: Option<ScheduleBuilder>,

    /// Finalized schedule: `None` until `finish`, `Some` after. Drives `update`.
    schedule: Option<Schedule>,

    /// One-shot startup systems, drained ONCE before the frame loop in
    /// `finish`. Cold, setup-only.
    startup: Vec<StartupSystem>,

    /// Duplicate-plugin detection (cold, setup-only). Linear `TypeId` scan; the
    /// plugin count is small (dozens at most) so a `Vec` beats a `HashSet`
    /// here and avoids the allocation.
    plugin_type_ids: Vec<TypeId>,

    /// The thread pool, owned by the `App`. Cloned into the [`ScheduleBuilder`]
    /// at construction and into the [`Schedule`] at `finish`.
    pool: Arc<ThreadPool>,

    /// Whether `finish` has run, so `update` / `run` / `run_n` auto-finish
    /// exactly once.
    finished: bool,
}

impl App {
    /// Creates an `App` with a fresh worker pool sized to the platform's
    /// available parallelism (the [`ThreadPoolBuilder`] default).
    pub fn new() -> Self {
        Self::with_pool(ThreadPoolBuilder::new().build())
    }

    /// Creates an `App` with a fresh worker pool of `n` threads.
    ///
    /// `n` is clamped to `[1, 64]` by the [`ThreadPoolBuilder`].
    pub fn with_threads(n: usize) -> Self {
        Self::with_pool(ThreadPoolBuilder::new().num_threads(n).build())
    }

    /// Creates an `App` reusing an external pool — for sharing one
    /// [`ThreadPool`] across several `App`s.
    pub fn with_pool(pool: Arc<ThreadPool>) -> Self {
        let builder = ScheduleBuilder::new(Arc::clone(&pool));
        Self {
            world: EcsMaster::new(),
            builder: Some(builder),
            schedule: None,
            startup: Vec::new(),
            plugin_type_ids: Vec::new(),
            pool,
            finished: false,
        }
    }

    /// Returns the worker pool owned by this `App` (escape hatch for code that
    /// needs the raw pool, e.g. a manual `par_iter` outside a system).
    #[inline]
    pub fn pool(&self) -> &Arc<ThreadPool> {
        &self.pool
    }

    // ── Config phase ─────────────────────────────────────────────────────────

    /// Inserts a resource into the world. Overwrites any existing value of the
    /// same type.
    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.world.insert_resource(resource);
        self
    }

    /// Registers state type `S` using `S::default()` as the initial value.
    pub fn init_state<S: States + Default>(&mut self) -> &mut Self {
        debug_assert!(
            self.builder.is_some(),
            "App::init_state called after finish() — App is in the run phase"
        );
        self.builder
            .as_mut()
            .expect("invariant: config method requires the staged builder")
            .init_state::<S>();
        self
    }

    /// Registers state type `S` with the given initial value.
    pub fn insert_state<S: States>(&mut self, state: S) -> &mut Self {
        debug_assert!(
            self.builder.is_some(),
            "App::insert_state called after finish() — App is in the run phase"
        );
        self.builder
            .as_mut()
            .expect("invariant: config method requires the staged builder")
            .insert_state(state);
        self
    }

    /// Registers systems with full ordering control. The closure receives the
    /// raw `&mut ScheduleBuilder`, so the Phase-15/16/17 chaining API
    /// (`.add_system(x).after(k).run_if(c)`, `.configure_set`, etc.) is
    /// available verbatim. THIS is the primary path for any non-trivial
    /// schedule.
    ///
    /// # Example
    ///
    /// ```ignore
    /// app.add_systems_cfg(|b| {
    ///     let setup = b.add_system(setup_system).id();
    ///     b.add_system(physics_system).after(setup);
    /// });
    /// ```
    pub fn add_systems_cfg(&mut self, f: impl FnOnce(&mut ScheduleBuilder)) -> &mut Self {
        debug_assert!(
            self.builder.is_some(),
            "App::add_systems_cfg called after finish() — App is in the run phase"
        );
        f(self
            .builder
            .as_mut()
            .expect("invariant: config method requires the staged builder"));
        self
    }

    /// Convenience for registering a single unordered system. Equivalent to
    /// `add_systems_cfg(|b| { b.add_system(system); })`.
    ///
    /// For ordered registration (`.before` / `.after` / `.run_if` / sets) use
    /// [`add_systems_cfg`](App::add_systems_cfg), which exposes the full
    /// builder chaining API.
    pub fn add_systems<F, M>(&mut self, system: F) -> &mut Self
    where
        F: IntoSystem<(), (), M>,
        F::System: System<Out = ()> + 'static,
    {
        // The `SystemConfig` returned by `add_system` is discarded (the
        // temporary drops at the end of the closure), since this convenience
        // path registers an unordered system.
        self.add_systems_cfg(|b| {
            b.add_system(system);
        })
    }

    /// Registers a system to run ONCE, before the frame loop (drained in
    /// [`finish`](App::finish)).
    ///
    /// Startup systems run single-threaded via [`EcsMaster::run_system`] — no
    /// pool / `par_iter` participation. For ordered or parallel setup, prefer
    /// an `on_enter`-state system (Phase 17).
    pub fn add_startup_system<F, M, Out>(&mut self, system: F) -> &mut Self
    where
        F: IntoSystem<(), Out, M> + 'static,
        F::System: System<Out = Out>,
    {
        debug_assert!(
            self.builder.is_some(),
            "App::add_startup_system called after finish() — App is in the run phase"
        );
        self.startup.push(Box::new(move |world: &mut EcsMaster| {
            world.run_system(system);
        }));
        self
    }

    // ── Plugins ──────────────────────────────────────────────────────────────

    /// Adds a single plugin: detects duplicates, then calls
    /// [`Plugin::build`] immediately and drops the plugin value.
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1801`) if a plugin of the same type was already added.
    /// Re-adding a plugin is virtually always a bug (double-registered systems
    /// / states), so it is rejected loudly rather than silently skipped.
    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        let tid = TypeId::of::<P>();
        if self.plugin_type_ids.contains(&tid) {
            duplicate_plugin_panic(plugin.name());
        }
        self.plugin_type_ids.push(tid);
        plugin.build(self);
        self
    }

    // ── Direct world access ───────────────────────────────────────────────────

    /// Returns a shared reference to the world.
    #[inline]
    pub fn world(&self) -> &EcsMaster {
        &self.world
    }

    /// Returns an exclusive reference to the world.
    #[inline]
    pub fn world_mut(&mut self) -> &mut EcsMaster {
        &mut self.world
    }

    // ── Finalize + run phase ──────────────────────────────────────────────────

    /// Finalizes the config phase into the run phase. Idempotent — a second
    /// call is a no-op.
    ///
    /// Builds the schedule (consuming the staged builder while borrowing
    /// `&mut world`) and then drains the startup systems once, after the world
    /// is fully initialized and before any frame runs.
    pub fn finish(&mut self) -> &mut Self {
        if self.finished {
            return self;
        }
        let builder = self
            .builder
            .take()
            .expect("invariant: builder is Some until the first finish()");
        // `take()` released the `builder` field borrow, so `&mut self.world` is
        // a disjoint field borrow here — this is the resolution of the
        // "build consumes the builder AND needs &mut world" wrinkle.
        let schedule = builder.build(&mut self.world);
        self.schedule = Some(schedule);
        self.finished = true;

        // Startup runs after build (world fully initialized), before any frame.
        let startup = std::mem::take(&mut self.startup);
        for s in startup {
            s(&mut self.world);
        }
        self
    }

    /// `true` once [`finish`](App::finish) has run.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Runs exactly one frame. Auto-finishes on the first call; the
    /// finish branch is cold after frame 1.
    pub fn update(&mut self) {
        if !self.finished {
            self.finish();
        }
        debug_assert!(
            self.schedule.is_some(),
            "invariant: schedule must be built before update()"
        );
        self.schedule
            .as_mut()
            .expect("invariant: schedule is Some after finish()")
            .run(&mut self.world);
    }

    /// Finishes once, then runs `frames` frames. The finish is hoisted out of
    /// the loop, so the loop body is a direct `Schedule::run`.
    pub fn run_n(&mut self, frames: u64) {
        self.finish();
        debug_assert!(
            self.schedule.is_some(),
            "invariant: schedule must be built before run_n()"
        );
        // Bind `schedule` + `world` to disjoint field borrows ONCE so the loop
        // body is provably `schedule.run(world)` with no per-frame branch.
        let schedule = self
            .schedule
            .as_mut()
            .expect("invariant: schedule is Some after finish()");
        let world = &mut self.world;
        for _ in 0..frames {
            schedule.run(world);
        }
    }

    /// Finishes once, then loops until a system sets `AppExit(true)`.
    ///
    /// Inserts an `AppExit(false)` resource before the loop so the per-frame
    /// read never panics on a missing resource. A system requests exit via
    /// `ResMut<AppExit>`; the flag is checked once per frame, after the frame
    /// completes. Note: this resets `AppExit` to `false` at the start of `run`,
    /// so a pre-loop exit request (e.g. set by a startup system) is cleared and
    /// at least one frame always executes — request exit from a frame system.
    pub fn run(&mut self) {
        self.finish();
        // Ensure the exit flag is present so the per-frame read below cannot
        // panic on an absent resource.
        self.world.insert_resource(AppExit(false));
        debug_assert!(
            self.schedule.is_some(),
            "invariant: schedule must be built before run()"
        );
        // Bind `schedule` + `world` to disjoint field borrows ONCE; the loop
        // body is `schedule.run(world)` plus one exit-flag read.
        let schedule = self
            .schedule
            .as_mut()
            .expect("invariant: schedule is Some after finish()");
        let world = &mut self.world;
        loop {
            schedule.run(world);
            if world.resource::<AppExit>().0 {
                break;
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Cold duplicate-plugin panic, kept out of the `add_plugin` body so the hot
/// (no-duplicate) path stays compact.
#[cold]
#[inline(never)]
fn duplicate_plugin_panic(name: &'static str) -> ! {
    panic!("boyko-B1801: plugin '{name}' added more than once");
}
