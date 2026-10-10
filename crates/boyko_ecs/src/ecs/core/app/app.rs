//! The [`App`] builder facade — a thin, additive composition layer over the
//! shipped [`EcsMaster`] + [`ScheduleBuilder`] + [`Schedule`] + [`ThreadPool`].
//!
//! `App` owns the world, stages the [`CoreSchedule`]s (Main + an optional
//! Fixed), owns the worker pool, and drives the per-frame loop (Phase 20
//! plan D1): ① [`Time`] advance → ② margin-aware check-ticks pass → ③ gated
//! event swap → ④ fixed catch-up loop → ⑤ Main run. It adds **no** per-frame
//! allocation, `dyn` dispatch, or atomic beyond `Schedule::run` itself: all
//! of the `Box<dyn Plugin>` / `Vec` / `TypeId` / tuple machinery is cold,
//! setup-only code, and the frame driver adds only a handful of predictable
//! branches around the runs.
//!
//! # Event-swap contract (plan D6)
//!
//! A world driven by an `App` must NOT also call
//! [`EcsMaster::update_events`] manually — the driver owns the once-per-frame
//! swap (gated under [`EventUpdatePolicy::WaitForFixed`]); a second manual
//! flip would halve every reader's visibility window.

use std::any::TypeId;
use std::sync::Arc;
use std::time::{Duration, Instant};

use boyko_log::codes::{B1801, B1802};
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

use crate::ecs::core::app::app_exit::AppExit;
use crate::ecs::core::app::plugin::Plugin;
use crate::ecs::core::change_detection::{
    CHECK_TICK_PREEMPT_MARGIN, CHECK_TICK_THRESHOLD, MAX_CHANGE_AGE, run_check_ticks_scan,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
// The frame driver's four zone sites. `zone!` matches a bare `ident` -- deliberately, so a site
// cannot name a handle it did not import -- and one `use` brings in both items a `declare_zone!`
// emits: the `static` in the value namespace and the `mod` companion in the type namespace.
use crate::ecs::core::profiling::zones::{EVENTS, FIXED_STEP, FRAME, MAIN_RUN};
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::schedule::schedule::Schedule;
use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;
use crate::ecs::core::state::states::States;
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::system::System;
use crate::ecs::core::time::fixed_time::DEFAULT_FIXED_TIMESTEP;
use crate::ecs::core::time::{FixedTime, Time, fixed_advance};

/// A type-erased one-shot startup runnable, drained once in [`App::finish`].
/// Boxed because each startup system is a distinct monomorphized closure type.
type StartupSystem = Box<dyn FnOnce(&mut EcsMaster)>;

/// A type-erased run-loop owner, installed via [`App::set_runner`] and
/// `take()`n by [`App::run`] (host plan D6 / rung R1 — the Bevy `RunnerFn`
/// precedent adapted to `&mut App`, no App extraction). One setup-stage box,
/// called once; never on the frame path.
type RunnerFn = Box<dyn FnOnce(&mut App) -> AppExit>;

/// The closed set of top-level schedules an [`App`] drives (Phase 20 plan D5).
///
/// Matched ONLY inside config-time routing methods (`add_systems_in`,
/// `add_systems_cfg_in`, `init_state_in`, `insert_state_in`); the frame
/// driver accesses the schedules through direct named fields — zero dispatch,
/// no label map. New top-level slots are an engine change by design;
/// finer-grained structure WITHIN a schedule is what Phase-15 sets are for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoreSchedule {
    /// The per-frame schedule (the existing one-arg `add_systems` target).
    /// Runs exactly once per [`App::update_with_delta`], after the fixed loop.
    Main,
    /// The fixed-timestep schedule: runs 0..N times per frame under the
    /// [`fixed_advance`] catch-up loop (64 Hz by default; see
    /// [`App::set_fixed_timestep`]). Created lazily on first registration.
    Fixed,
}

/// When the frame driver swaps the event double-buffer (Phase 20 plan D6).
///
/// Resolved at [`App::finish`]: a user-set value wins; otherwise
/// `WaitForFixed` iff a Fixed schedule was configured, else `EveryFrame`.
///
/// # The pause hazard (plan ★M1)
///
/// Under `WaitForFixed`, a paused [`Time`] yields 0 substeps every frame, so
/// the swap is held INDEFINITELY — starving ALL readers, including Main-only
/// readers unrelated to the fixed schedule (a paused menu sending UI events
/// is the canonical case). Held sends keep accumulating until the per-lane
/// capacity is hit, after which `send` returns `Err(EventBufferFull)` — check
/// that `Result` in pause-capable apps, or select `EveryFrame` when no
/// fixed-schedule reader exists. On unpause the held backlog arrives in ONE
/// generation (bounded, nothing lost — a stale-event burst, not a leak).
///
/// At cold start (plan ★m4), startup-sent events become visible at the first
/// post-substep swap — a bounded ≈ 2-frame delay at 60 FPS / 64 Hz, never a
/// loss.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventUpdatePolicy {
    /// Swap only after the fixed schedule has run ≥ 1 substep since the last
    /// swap, so a fixed-schedule `EventReader` never loses a buffer
    /// generation on a 0-substep frame. With NO fixed schedule this silently
    /// degrades to `EveryFrame` (the gate's `fixed.is_none()` arm).
    WaitForFixed,
    /// Swap once at the start of every frame (the single-schedule default).
    EveryFrame,
}

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
/// `App` is `!Send + !Sync`. NOT because of the world — [`EcsMaster`] has been
/// `Send + Sync` since Phase 9 (the old wording here was stale; Phase 21 audit) —
/// but because of the type-erased one-shot closures the `App` stages:
/// `StartupSystem` (`Box<dyn FnOnce(&mut EcsMaster)>` without `+ Send`) and
/// the schedules' `StateEntry::insert` closures of the same shape (the R1
/// [`RunnerFn`] box is another). Pinned at
/// compile time by `assert_not_impl_any!(App: Send, Sync)` in
/// `tests/multi_world.rs`. Practically: an `App` is built and run on a single
/// dispatcher thread and never crosses a thread boundary; the only other
/// threads are the pool's workers, touched exclusively inside
/// [`Schedule::run`].
pub struct App {
    // Field order is for readability; no field's Drop observes another. App is
    // always dropped with no frame in flight (every `run*` takes &mut self ⇒
    // workers are idle and hold no borrow of world/schedule), so drop order is
    // not soundness-load-bearing.
    /// The world (storage + resources + entities). Lazy per Phase 12.6 — not
    /// eagerly memset on construction.
    world: EcsMaster,

    /// Staged Main schedule builder: `Some` during config, `None` after
    /// `finish` (which consumes it by value to produce the [`Schedule`]).
    builder: Option<ScheduleBuilder>,

    /// Finalized Main schedule: `None` until `finish`, `Some` after. Runs
    /// once per frame (driver step ⑤).
    schedule: Option<Schedule>,

    /// Staged Fixed schedule builder (Phase 20 D5): created lazily on the
    /// first `*_in(CoreSchedule::Fixed, …)` registration, `None` otherwise
    /// and after `finish`.
    fixed_builder: Option<ScheduleBuilder>,

    /// Finalized Fixed schedule: `Some` after `finish` iff `fixed_builder`
    /// existed. `None` ⇒ the frame driver's fixed branch is one
    /// predicted-not-taken check.
    fixed: Option<Schedule>,

    /// Config staging for the fixed timestep (default exactly 64 Hz); applied
    /// as `FixedTime::new(fixed_timestep)` at `finish` UNLESS the user
    /// inserted a `FixedTime` resource during config (insert-if-absent).
    fixed_timestep: Duration,

    /// User-set event policy override; `None` ⇒ auto-resolve at `finish`
    /// (`WaitForFixed` iff a Fixed schedule was configured — plan D6).
    event_policy_cfg: Option<EventUpdatePolicy>,

    /// The resolved event policy (valid after `finish`).
    event_policy: EventUpdatePolicy,

    /// D6 gate counter: substeps run since the last event swap
    /// (`saturating_add`; zeroed on swap). Holds the swap across consecutive
    /// 0-substep frames under `WaitForFixed`.
    fixed_steps_since_swap: u32,

    /// D11 self-clock anchor for [`App::update`]: `None` until the first
    /// frame (whose raw delta is therefore ZERO — Bevy parity).
    last_instant: Option<Instant>,

    /// One-shot startup systems, drained ONCE before the frame loop in
    /// `finish`. Cold, setup-only.
    startup: Vec<StartupSystem>,

    /// The installed run-loop owner (host plan D6 / rung R1): `Some` after
    /// [`set_runner`](App::set_runner), `take()`n by [`run`](App::run) BEFORE
    /// it calls `finish()` — when installed, the runner owns the app
    /// lifecycle (its own `finish()` call, `AppExit` policy, and teardown).
    /// Cold, setup-only — the one `dyn` dispatch in the host design.
    runner: Option<RunnerFn>,

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
            fixed_builder: None,
            fixed: None,
            fixed_timestep: DEFAULT_FIXED_TIMESTEP,
            event_policy_cfg: None,
            // Placeholder until `finish` resolves the policy (plan D6); an
            // App with no Fixed schedule keeps this value.
            event_policy: EventUpdatePolicy::EveryFrame,
            fixed_steps_since_swap: 0,
            last_instant: None,
            startup: Vec::new(),
            runner: None,
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

    /// Run-phase guard shared by EVERY config method: once
    /// [`finish`](App::finish) has consumed the staged builders, a late config
    /// call could never take effect (a fresh fixed builder, a staged setter
    /// value, a startup push — none would ever be built or drained), so it
    /// fails LOUDLY in both debug and release builds — one API, one failure
    /// mode, never a silent drop.
    #[inline]
    fn assert_config_phase(&self, method: &'static str) {
        if self.finished {
            config_after_finish_panic(method);
        }
    }

    /// Inserts a resource into the world. Overwrites any existing value of the
    /// same type.
    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.world.insert_resource(resource);
        self
    }

    /// Registers state type `S` using `S::default()` as the initial value,
    /// into the **Main** schedule (the [`CoreSchedule::Main`] routing of
    /// [`init_state_in`](App::init_state_in)).
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
    pub fn init_state<S: States + Default>(&mut self) -> &mut Self {
        self.assert_config_phase("init_state");
        self.builder
            .as_mut()
            .expect("invariant: config method requires the staged builder")
            .init_state::<S>();
        self
    }

    /// Registers state type `S` with the given initial value, into the
    /// **Main** schedule (the [`CoreSchedule::Main`] routing of
    /// [`insert_state_in`](App::insert_state_in)).
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
    pub fn insert_state<S: States>(&mut self, state: S) -> &mut Self {
        self.assert_config_phase("insert_state");
        self.builder
            .as_mut()
            .expect("invariant: config method requires the staged builder")
            .insert_state(state);
        self
    }

    /// Registers systems with full ordering control, into the **Main**
    /// schedule (the [`CoreSchedule::Main`] routing of
    /// [`add_systems_cfg_in`](App::add_systems_cfg_in)). The closure receives
    /// the raw `&mut ScheduleBuilder`, so the Phase-15/16/17 chaining API
    /// (`.add_system(x).after(k).run_if(c)`, `.configure_set`, etc.) is
    /// available verbatim. THIS is the primary path for any non-trivial
    /// schedule.
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
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
        self.assert_config_phase("add_systems_cfg");
        f(self
            .builder
            .as_mut()
            .expect("invariant: config method requires the staged builder"));
        self
    }

    /// Convenience for registering a single unordered system into the
    /// **Main** schedule (the [`CoreSchedule::Main`] routing of
    /// [`add_systems_in`](App::add_systems_in)). Equivalent to
    /// `add_systems_cfg(|b| { b.add_system(system); })`.
    ///
    /// For ordered registration (`.before` / `.after` / `.run_if` / sets) use
    /// [`add_systems_cfg`](App::add_systems_cfg), which exposes the full
    /// builder chaining API.
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
    pub fn add_systems<F, M>(&mut self, system: F) -> &mut Self
    where
        F: IntoSystem<(), (), M>,
        F::System: System<Out = ()> + 'static,
    {
        self.assert_config_phase("add_systems");
        // The `SystemConfig` returned by `add_system` is discarded (the
        // temporary drops at the end of the closure), since this convenience
        // path registers an unordered system.
        self.add_systems_cfg(|b| {
            b.add_system(system);
        })
    }

    // ── Multi-schedule routing (Phase 20 D5) ─────────────────────────────────

    /// Returns the staged Fixed builder, creating it lazily on the first
    /// Fixed registration with a clone of the App's own pool (plan D5: the
    /// pool exists from construction, so the lazy creation has no ordering
    /// hole). Cold, config-only.
    fn fixed_builder_mut(&mut self) -> &mut ScheduleBuilder {
        self.fixed_builder
            .get_or_insert_with(|| ScheduleBuilder::new(Arc::clone(&self.pool)))
    }

    /// [`add_systems_cfg`](App::add_systems_cfg) with an explicit
    /// [`CoreSchedule`] target. `Main` routes to the existing builder;
    /// `Fixed` lazily creates the fixed builder on first use.
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish) — a
    /// post-finish registration could never be built into a running schedule.
    pub fn add_systems_cfg_in(
        &mut self,
        schedule: CoreSchedule,
        f: impl FnOnce(&mut ScheduleBuilder),
    ) -> &mut Self {
        self.assert_config_phase("add_systems_cfg_in");
        match schedule {
            CoreSchedule::Main => self.add_systems_cfg(f),
            CoreSchedule::Fixed => {
                f(self.fixed_builder_mut());
                self
            }
        }
    }

    /// [`add_systems`](App::add_systems) with an explicit [`CoreSchedule`]
    /// target: registers a single unordered system into `schedule`.
    ///
    /// A `Fixed` system runs once per fixed substep and should read
    /// `Res<FixedTime>` for its delta; a `Main` system runs once per frame
    /// and reads `Res<Time>` (plan D2 — the parameter type IS the clock
    /// documentation).
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
    pub fn add_systems_in<F, M>(&mut self, schedule: CoreSchedule, system: F) -> &mut Self
    where
        F: IntoSystem<(), (), M>,
        F::System: System<Out = ()> + 'static,
    {
        self.assert_config_phase("add_systems_in");
        self.add_systems_cfg_in(schedule, |b| {
            b.add_system(system);
        })
    }

    /// [`init_state`](App::init_state) with an explicit [`CoreSchedule`]
    /// target.
    ///
    /// Phase 20 D7 (binding contract): `on_enter` / `on_exit` /
    /// `on_transition` conditions are valid only on systems in the SAME
    /// schedule where the state is registered; `in_state` (a plain value
    /// read) is valid anywhere at frame granularity. Do not register one
    /// state type into both schedules (double pass + double initial).
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
    pub fn init_state_in<S: States + Default>(&mut self, schedule: CoreSchedule) -> &mut Self {
        self.assert_config_phase("init_state_in");
        self.insert_state_in(schedule, S::default())
    }

    /// [`insert_state`](App::insert_state) with an explicit [`CoreSchedule`]
    /// target. See [`init_state_in`](App::init_state_in) for the
    /// same-schedule contract on edge conditions (plan D7).
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish).
    pub fn insert_state_in<S: States>(&mut self, schedule: CoreSchedule, state: S) -> &mut Self {
        self.assert_config_phase("insert_state_in");
        match schedule {
            CoreSchedule::Main => self.insert_state(state),
            CoreSchedule::Fixed => {
                self.fixed_builder_mut().insert_state(state);
                self
            }
        }
    }

    /// Sets the fixed timestep applied at [`finish`](App::finish) as
    /// `FixedTime::new(d)` (insert-if-absent: a user-inserted `FixedTime`
    /// resource wins). Default: exactly 64 Hz = 15 625 000 ns.
    ///
    /// Lowering the timestep raises the worst-case substep count per frame
    /// proportionally (plan D4: the bound is
    /// `⌈(max_delta × speed + timestep) / timestep⌉`).
    ///
    /// # Panics
    ///
    /// Panics if `d` is zero (via the same validation as
    /// [`FixedTime::new`]); panics (`boyko-B1802`) if called after
    /// [`finish`](App::finish) — the staged value would never be applied.
    pub fn set_fixed_timestep(&mut self, d: Duration) -> &mut Self {
        self.assert_config_phase("set_fixed_timestep");
        // Route through the FixedTime constructor so the zero-timestep
        // validation lives in exactly one place.
        self.fixed_timestep = FixedTime::new(d).timestep();
        self
    }

    /// Convenience for [`set_fixed_timestep`](App::set_fixed_timestep) in
    /// frequency form: `set_fixed_hz(60.0)` preserves a legacy 60 Hz loop.
    ///
    /// # Panics
    ///
    /// Panics if `hz` is not finite, not strictly positive, or so large that
    /// the timestep rounds below `Duration`'s 1 ns resolution (see
    /// [`FixedTime::from_hz`]); panics (`boyko-B1802`) if called after
    /// [`finish`](App::finish).
    pub fn set_fixed_hz(&mut self, hz: f64) -> &mut Self {
        self.assert_config_phase("set_fixed_hz");
        self.fixed_timestep = FixedTime::from_hz(hz).timestep();
        self
    }

    /// Overrides the auto-resolved [`EventUpdatePolicy`] (plan D6: the
    /// default is `WaitForFixed` iff a Fixed schedule was configured, else
    /// `EveryFrame`). See the [`EventUpdatePolicy`] docs for the
    /// `WaitForFixed` pause hazard before forcing it.
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish) — the
    /// policy is resolved at `finish`, so a later override would be silently
    /// ineffective.
    pub fn set_event_update_policy(&mut self, p: EventUpdatePolicy) -> &mut Self {
        self.assert_config_phase("set_event_update_policy");
        self.event_policy_cfg = Some(p);
        self
    }

    /// Registers a system to run ONCE, before the frame loop (drained in
    /// [`finish`](App::finish)).
    ///
    /// Startup systems run single-threaded via [`EcsMaster::run_system`] — no
    /// pool / `par_iter` participation. For ordered or parallel setup, prefer
    /// an `on_enter`-state system (Phase 17).
    ///
    /// # Panics
    ///
    /// Panics (`boyko-B1802`) if called after [`finish`](App::finish) — the
    /// startup queue is drained exactly once there, so a later push would
    /// never run.
    pub fn add_startup_system<F, M, Out>(&mut self, system: F) -> &mut Self
    where
        F: IntoSystem<(), Out, M> + 'static,
        F::System: System<Out = Out>,
    {
        self.assert_config_phase("add_startup_system");
        self.startup.push(Box::new(move |world: &mut EcsMaster| {
            world.run_system(system);
        }));
        self
    }

    /// Installs a run-loop owner: [`run`](App::run) hands control to `runner`
    /// as its FIRST action — BEFORE [`finish`](App::finish) — and returns the
    /// runner's [`AppExit`] verbatim (host plan D6 / rung R1; the Bevy
    /// `RunnerFn` precedent adapted to `&mut App`, no App extraction).
    ///
    /// When installed, the runner OWNS the app lifecycle: it is responsible
    /// for calling `app.finish()` itself (typically after inserting its
    /// platform resources, so the startup one-shots see them), for its own
    /// `AppExit` policy (e.g. a windowed host's insert-if-absent vs the
    /// headless path's unconditional insert), and for its own teardown before
    /// returning.
    ///
    /// One setup-stage box, called once. Installing a second runner REPLACES
    /// the first. Unlike the config methods, `set_runner` is NOT subject to
    /// the post-`finish` panic guard (`boyko-B1802`) — it may be called any
    /// time before [`run`](App::run).
    pub fn set_runner(&mut self, runner: Box<dyn FnOnce(&mut App) -> AppExit>) {
        self.runner = Some(runner);
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
    /// Resolves the event policy, inserts the clock resources if absent,
    /// builds the schedules (consuming the staged builders while borrowing
    /// `&mut world`), and then drains the startup systems once, after the
    /// world is fully initialized and before any frame runs.
    pub fn finish(&mut self) -> &mut Self {
        if self.finished {
            return self;
        }

        // Phase 20 D6: resolve the event policy BEFORE the builders are
        // consumed — the auto default reads whether a Fixed schedule was
        // configured. A user override always wins.
        self.event_policy = match self.event_policy_cfg {
            Some(p) => p,
            None if self.fixed_builder.is_some() => EventUpdatePolicy::WaitForFixed,
            None => EventUpdatePolicy::EveryFrame,
        };

        // Phase 20 D2: insert the clock resources IF ABSENT — a user-inserted
        // value during config wins (e.g. a custom `Time` with a different
        // `max_delta`, or a pre-seeded `FixedTime`).
        if !self.world.contains_resource::<Time>() {
            self.world.insert_resource(Time::default());
        }
        if !self.world.contains_resource::<FixedTime>() {
            self.world.insert_resource(FixedTime::new(self.fixed_timestep));
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

        // The Fixed schedule, when configured, builds through the identical
        // take-then-disjoint-borrow dance.
        if let Some(fixed_builder) = self.fixed_builder.take() {
            self.fixed = Some(fixed_builder.build(&mut self.world));
        }

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

    /// Runs exactly one frame with an externally supplied raw delta — THE
    /// frame function (Phase 20 plan D1/D11); [`update`](App::update) is its
    /// self-clocked shell. Auto-finishes on the first call; the finish branch
    /// is cold after frame 1.
    ///
    /// # Frame order (plan D1, binding)
    ///
    /// 1. [`Time::advance_with`]`(raw)` — clamp / scale / pause once.
    /// 2. Margin-aware check-ticks pass (plan ★C1/D8): a single u32 compare,
    ///    predicted-not-taken; the cold all-schedule clamp fires
    ///    [`CHECK_TICK_PREEMPT_MARGIN`] ticks before any schedule's internal
    ///    block could, so dormant siblings are never starved of their clamp.
    /// 3. Gated event swap (plan D6): swap iff the policy is `EveryFrame`,
    ///    no Fixed schedule exists, or the fixed loop ran ≥ 1 substep since
    ///    the last swap.
    /// 4. Fixed catch-up loop: [`fixed_advance`] with `|w| fixed.run(w)` —
    ///    0..N opaque `Schedule::run`s (at most 16 at the defaults).
    /// 5. Main `Schedule::run`.
    ///
    /// All inter-run work holds the dispatcher's own `&mut EcsMaster` with
    /// zero workers in flight — the runs are opaque units (plan D1).
    pub fn update_with_delta(&mut self, raw: Duration) {
        if !self.finished {
            self.finish();
        }
        debug_assert!(
            self.finished,
            "invariant: App must be finished before the frame driver runs"
        );
        debug_assert!(
            self.schedule.is_some(),
            "invariant: schedule must be built before update_with_delta()"
        );

        // ★C1 invariant witness: one frame must consume fewer than
        // CHECK_TICK_PREEMPT_MARGIN ticks (2 bumps × (1 + substeps) ≤ 34 at
        // the defaults), or the margin no longer guarantees the App pass
        // preempts the internal blocks.
        #[cfg(debug_assertions)]
        let frame_start_tick = self.world.current_tick().get();

        // ⓪ Profiling fold (profiling A2/D16), BEFORE step ①.
        //
        // This is the single funnel both frame entry points share: the windowed host calls
        // `update_with_delta` directly and never touches `update`, so a fold placed there would
        // never run in the one configuration that has a GPU channel. It is also what puts the
        // instrument outside its own primary number — the frame time the profiler reports does
        // not include the cost of reporting it.
        //
        // With profiling off this is one `.bss` load and one predicted branch; the resource is not
        // touched, and a world without a `Profiler` is a supported state.
        crate::ecs::core::profiling::fold_frame(&mut self.world);

        // `__frame` opens HERE, after the fold has returned — which is what puts the instrument
        // outside its own primary number. Its guard lives to the end of this function, so the
        // bracket is the frame minus the fold, by construction rather than by subtraction.
        let _z_frame = boyko_diag::zone!(FRAME);

        // ① Advance the virtual clock (clamp → scale → pause, plan D4/★m5).
        self.world.resource_mut::<Time>().advance_with(raw);

        // ② Margin-aware all-schedule check-ticks pass (plan ★C1/D8). One
        // u32 compare per frame; the cold body clamps EVERY schedule.
        if self
            .world
            .should_run_check_ticks_with_margin(CHECK_TICK_PREEMPT_MARGIN)
        {
            self.check_ticks_all_schedules();
        }

        // ③ Gated event swap (plan D6). The gate reads the substep counter
        // accumulated by PREVIOUS frames, so the hold composes across
        // consecutive 0-substep frames; with no Fixed schedule the
        // `fixed.is_none()` arm degrades `WaitForFixed` to every-frame.
        if self.event_policy == EventUpdatePolicy::EveryFrame
            || self.fixed.is_none()
            || self.fixed_steps_since_swap > 0
        {
            let _z = boyko_diag::zone!(EVENTS);
            self.world.update_events();
            self.fixed_steps_since_swap = 0;
        }

        // ④ Fixed catch-up loop (plan D3). Disjoint field borrows: `fixed`
        // borrows `self.fixed`, the driver passes `self.world` — the same
        // dance `finish` uses.
        if let Some(fixed) = self.fixed.as_mut() {
            // The bracket is around ONE substep, inside the catch-up loop, so a frame that runs N
            // of them produces N samples. A bracket around the loop would produce one span whose
            // `count` could never report N — and N is the number a reader needs to tell a slow
            // substep from a frame that ran three of them.
            let steps = fixed_advance(&mut self.world, |w| {
                let _z = boyko_diag::zone!(FIXED_STEP);
                fixed.run(w);
            });
            self.fixed_steps_since_swap = self.fixed_steps_since_swap.saturating_add(steps);
        }

        // ⑤ Main run (the pre-Phase-20 frame body, unchanged).
        {
            let _z = boyko_diag::zone!(MAIN_RUN);
            self.schedule
                .as_mut()
                .expect("invariant: schedule is Some after finish()")
                .run(&mut self.world);
        }

        #[cfg(debug_assertions)]
        {
            let consumed = self
                .world
                .current_tick()
                .get()
                .wrapping_sub(frame_start_tick);
            debug_assert!(
                consumed < CHECK_TICK_PREEMPT_MARGIN,
                "★C1 invariant: one frame consumed {consumed} ticks, >= the preempt margin \
                 ({CHECK_TICK_PREEMPT_MARGIN}) — the App check-ticks pass can no longer \
                 preempt the per-schedule internal blocks (substep count too high?)"
            );
        }
    }

    /// Runs exactly one frame, self-clocked via [`Instant`] (plan D11): the
    /// raw delta is the wall time since the previous `update` call, ZERO on
    /// the first frame (Bevy parity). Delegates to
    /// [`update_with_delta`](App::update_with_delta).
    ///
    /// Embedders that own the clock (eframe, a wasm host, deterministic
    /// tests) call `update_with_delta` directly instead.
    pub fn update(&mut self) {
        let now = Instant::now();
        let raw = match self.last_instant {
            Some(last) => now.duration_since(last),
            None => Duration::ZERO,
        };
        self.last_instant = Some(now);
        self.update_with_delta(raw);
    }

    /// Finishes once, then runs `frames` self-clocked frames (a
    /// [`update`](App::update) loop).
    pub fn run_n(&mut self, frames: u64) {
        self.finish();
        for _ in 0..frames {
            self.update();
        }
    }

    /// Finishes once, then runs `frames` frames with the SAME externally
    /// supplied raw delta each frame — the deterministic loop for tests and
    /// benches (plan D11/Q7: every TIMED artifact routes through this, so
    /// `Instant::now` jitter stays out of measured loops; Miri suites use it
    /// because `Instant` requires isolation to be disabled).
    pub fn run_n_with_delta(&mut self, frames: u64, delta: Duration) {
        self.finish();
        for _ in 0..frames {
            self.update_with_delta(delta);
        }
    }

    /// Hands control to the installed runner, or (headless default) finishes
    /// once and loops self-clocked frames until a system sets `AppExit(true)`.
    /// Returns the exit value in both modes.
    ///
    /// # Runner dispatch (host plan D6 / rung R1)
    ///
    /// If a runner was installed via [`set_runner`](App::set_runner), `run()`
    /// hands it control as its FIRST action and returns its [`AppExit`]
    /// verbatim: in runner mode `run()` calls NEITHER `finish()` nor inserts
    /// `AppExit` — the runner owns both (see the `set_runner` contract). The
    /// runner is `take()`n out of the `App`, so a hypothetical second `run()`
    /// call falls through to the headless path below — a documented edge, not
    /// a guarded one.
    ///
    /// # Headless path (no runner) — pre-R1 behavior, unchanged
    ///
    /// Inserts an `AppExit(false)` resource before the loop so the per-frame
    /// read never panics on a missing resource. A system requests exit via
    /// `ResMut<AppExit>`; the flag is checked once per frame, after the frame
    /// completes (after the Main run — a Fixed-schedule exit request is
    /// observed at the end of the same frame). Note: this resets `AppExit` to
    /// `false` at the start of `run`, so a pre-loop exit request (e.g. set by
    /// a startup system) is cleared and at least one frame always executes —
    /// request exit from a frame system. The loop exits only on a `true`
    /// flag, so this path always returns `AppExit(true)`.
    pub fn run(&mut self) -> AppExit {
        if let Some(runner) = self.runner.take() {
            return runner(self);
        }
        self.finish();
        // Ensure the exit flag is present so the per-frame read below cannot
        // panic on an absent resource.
        self.world.insert_resource(AppExit(false));
        loop {
            self.update();
            if self.world.resource::<AppExit>().0 {
                break;
            }
        }
        AppExit(true)
    }

    /// Phase 20 ★C1/D8 — the cold all-schedule check-ticks pass: the
    /// world-level per-row pool scan plus BOTH schedules' system/condition
    /// clamps, under one `current_tick` snapshot, then the shared counter
    /// reset.
    ///
    /// The App is the only owner that can enumerate all schedules, and
    /// clamping at frame start with the un-bumped `current` is sound:
    /// `check_tick` only pulls old values forward, and the ≤ 34 ticks of
    /// staleness vs the internal blocks is noise against the ~518 M slack.
    #[cold]
    #[inline(never)]
    fn check_ticks_all_schedules(&mut self) {
        let t = self.world.current_tick();
        // Plan §13.1: current_tick monotone vs last_check_tick — the clamp
        // math is faithful only while the elapsed distance stays within the
        // §9.3 wraparound headroom (guards future call-site drift past the
        // `should_run_check_ticks_with_margin` gate).
        debug_assert!(
            t.get().wrapping_sub(self.world.last_check_tick.get())
                <= MAX_CHANGE_AGE.wrapping_add(CHECK_TICK_THRESHOLD),
            "invariant: the elapsed tick distance since the last clamp must stay within \
             MAX_CHANGE_AGE + CHECK_TICK_THRESHOLD when the App check-ticks pass fires"
        );
        run_check_ticks_scan(&mut self.world);
        self.schedule
            .as_mut()
            .expect("invariant: schedule is Some after finish()")
            .check_change_ticks(t);
        if let Some(fixed) = self.fixed.as_mut() {
            fixed.check_change_ticks(t);
        }
        self.world.set_last_check_tick(t);
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
    // L8b: the code is the IDENTIFIER, positionally. The rendered text is byte-identical to the
    // literal it replaces (`PanicCode`'s `Display` prints `boyko-B1801`), which is what keeps
    // every `#[should_panic(expected = ..)]` matching; what changed is that the registry's orphan
    // check can now SEE the row, because it scans identifiers and a literal is invisible to it.
    // Positional and never inline (`{B1801}`) -- an inline argument lives inside the string
    // literal, so the walker's LIT stream would see it and its CODE stream would not.
    panic!("{}: plugin '{}' added more than once", B1801, name);
}

/// Cold run-phase config panic: [`App::finish`] consumes the staged builders,
/// drains the startup queue, and resolves the config staging, so a
/// post-`finish` config call could never take effect. Every config method
/// routes here via [`App::assert_config_phase`], so debug AND release builds
/// fail identically loudly instead of silently dropping the registration.
#[cold]
#[inline(never)]
fn config_after_finish_panic(method: &'static str) -> ! {
    // As `duplicate_plugin_panic` above: the identifier, positionally, and the rendered bytes are
    // unchanged -- five `#[should_panic(expected = "boyko-B1802: App::…")]` cases below depend on
    // exactly that.
    panic!(
        "{}: App::{} called after finish() — the App is in the run phase; \
         perform all configuration before the first finish()/update()/run() call",
        B1802, method
    );
}

#[cfg(test)]
#[cfg(not(miri))] // constructs a real ThreadPool — same gate as tests/app_plugin.rs
mod tests {
    // Test-only harness: `Rc`/`RefCell` are the single-threaded observation
    // channel the assertions read plugin/schedule side effects through — the
    // reference model, never engine data. Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

    use super::*;

    /// One-variant state type for the post-finish routing panic test.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum GuardState {
        A,
    }
    impl States for GuardState {}

    fn serial_app() -> App {
        App::with_pool(ThreadPoolBuilder::new().num_threads(1).build())
    }

    // ── M1 — post-finish config fails loudly, uniformly (debug AND release) ──

    /// A Fixed-arm registration after `finish()` panics (`boyko-B1802`)
    /// instead of silently staging into a fixed builder that is never built.
    #[test]
    #[should_panic(expected = "boyko-B1802: App::add_systems_in")]
    fn add_systems_in_fixed_after_finish_panics() {
        let mut app = serial_app();
        app.finish();
        app.add_systems_in(CoreSchedule::Fixed, || {});
    }

    /// A Fixed-arm state registration after `finish()` panics (`boyko-B1802`).
    #[test]
    #[should_panic(expected = "boyko-B1802: App::insert_state_in")]
    fn insert_state_in_fixed_after_finish_panics() {
        let mut app = serial_app();
        app.finish();
        app.insert_state_in(CoreSchedule::Fixed, GuardState::A);
    }

    /// A startup registration after `finish()` panics (`boyko-B1802`) — the
    /// startup queue was already drained, so the system could never run.
    #[test]
    #[should_panic(expected = "boyko-B1802: App::add_startup_system")]
    fn add_startup_system_after_finish_panics() {
        let mut app = serial_app();
        app.finish();
        app.add_startup_system(|| {});
    }

    // ── m1 — post-finish setters fail loudly too ─────────────────────────────

    /// `set_fixed_timestep` after `finish()` panics (`boyko-B1802`) instead of
    /// staging a value that would never be applied.
    #[test]
    #[should_panic(expected = "boyko-B1802: App::set_fixed_timestep")]
    fn set_fixed_timestep_after_finish_panics() {
        let mut app = serial_app();
        app.finish();
        app.set_fixed_timestep(Duration::from_millis(10));
    }

    /// `set_event_update_policy` after `finish()` panics (`boyko-B1802`) — the
    /// policy was already resolved at `finish`.
    #[test]
    #[should_panic(expected = "boyko-B1802: App::set_event_update_policy")]
    fn set_event_update_policy_after_finish_panics() {
        let mut app = serial_app();
        app.finish();
        app.set_event_update_policy(EventUpdatePolicy::EveryFrame);
    }

    // ── R1 — set_runner + the run() dispatch contract (host plan D6) ─────────

    use std::cell::Cell;
    use std::rc::Rc;

    use crate::ecs::core::system::params::ResMut;

    /// The installed runner receives control from `run()` and its `AppExit`
    /// propagates verbatim; the captured marker proves the body executed.
    #[test]
    fn runner_receives_control_and_return_propagates() {
        let ran = Rc::new(Cell::new(false));
        let marker = Rc::clone(&ran);

        let mut app = serial_app();
        app.set_runner(Box::new(move |_app| {
            marker.set(true);
            AppExit(true)
        }));

        let exit = app.run();
        assert!(ran.get(), "run() must invoke the installed runner body");
        assert!(exit.0, "run() must return the runner's AppExit verbatim");
    }

    /// `run()` hands control to the runner BEFORE `finish()`: the startup
    /// one-shot has NOT been drained when the runner starts (so a runner can
    /// insert its platform resources first), and the runner's own `finish()`
    /// call is what drains it.
    #[test]
    fn runner_mode_does_not_prefinish() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let started = Arc::new(AtomicBool::new(false));
        let s = Arc::clone(&started);

        let mut app = serial_app();
        app.add_startup_system(move || {
            s.store(true, Ordering::Relaxed);
        });
        let probe = Arc::clone(&started);
        app.set_runner(Box::new(move |app| {
            assert!(
                !app.is_finished(),
                "run() must NOT call finish() before handing control to the runner"
            );
            assert!(
                !probe.load(Ordering::Relaxed),
                "the startup one-shot must not run before the runner's finish()"
            );
            app.finish();
            assert!(
                probe.load(Ordering::Relaxed),
                "the runner's own finish() call must drain the startup queue"
            );
            AppExit(true)
        }));
        assert!(app.run().0, "the runner's exit value propagates");
    }

    /// Headless default unchanged: with NO runner installed, `run()` finishes,
    /// inserts `AppExit(false)`, and loops until a frame system requests exit
    /// — returning the observed flag, which on this path is always
    /// `AppExit(true)` (R1 signature). Frame-count termination stays covered
    /// by `tests/app_plugin.rs::run_exits_on_appexit`.
    #[test]
    fn headless_default_unchanged() {
        let mut app = serial_app();
        // In-crate, `.0` would hit ResMut's own `pub(crate)` field — deref
        // explicitly to reach the AppExit flag (external code says `exit.0`).
        app.add_systems(|mut exit: ResMut<AppExit>| {
            (*exit).0 = true;
        });
        let exit = app.run();
        assert!(exit.0, "the headless path returns AppExit(true) after the loop exits");
    }

    /// The runner is `take()`n by `run()`: a second `run()` falls through to
    /// the legacy headless path (terminating via the AppExit system) instead
    /// of invoking the runner again — the call counter must not bump twice.
    #[test]
    fn runner_take_semantics() {
        let calls = Rc::new(Cell::new(0u32));
        let counter = Rc::clone(&calls);

        let mut app = serial_app();
        // Registered during config so the SECOND run() — the legacy path,
        // which is the one that calls finish() — can terminate its loop.
        // In-crate, `.0` would hit ResMut's own `pub(crate)` field — deref
        // explicitly to reach the AppExit flag (external code says `exit.0`).
        app.add_systems(|mut exit: ResMut<AppExit>| {
            (*exit).0 = true;
        });
        app.set_runner(Box::new(move |_app| {
            counter.set(counter.get() + 1);
            AppExit(true)
        }));

        assert!(app.run().0, "first run(): the runner's exit value propagates");
        assert_eq!(calls.get(), 1, "the runner ran exactly once");

        assert!(
            app.run().0,
            "second run(): falls through to the legacy headless path and terminates"
        );
        assert_eq!(calls.get(), 1, "the take()n runner must NOT be invoked again");
    }

    // ── ★C1 — the margin-aware clamp pass strictly preempts the schedules'
    //    internal threshold blocks (plan D8 race shape; a naive near-threshold
    //    test would pass with the race present and is NOT sufficient) ─────────

    use crate::ecs::core::change_detection::{
        CHECK_TICK_PREEMPT_MARGIN, CHECK_TICK_THRESHOLD, Tick,
    };

    /// One 64 Hz step.
    const STEP: Duration = Duration::from_nanos(15_625_000);

    /// Builds a finished Main+Fixed app (one trivial system each) and runs one
    /// warm frame so both schedules have established `last_run` ticks.
    fn warm_two_schedule_app() -> App {
        let mut app = serial_app();
        app.add_systems(|| {});
        app.add_systems_in(CoreSchedule::Fixed, || {});
        app.finish();
        app.update_with_delta(STEP); // 1 substep + main ⇒ 4 bumps
        app
    }

    /// PROVENANCE: with elapsed forced to exactly `T − MARGIN` at frame start,
    /// the APP pass fires at step ② — `last_check_tick` ends the frame at the
    /// frame-START current tick (set BEFORE any bump). Had a schedule's
    /// internal block won the crossing instead (the pre-★C1 race), the value
    /// would be that schedule's post-bump `this_run` (frame_start + 1 for
    /// Fixed, +3 for Main).
    #[test]
    fn c1_app_clamp_pass_preempts_internal_blocks() {
        let mut app = warm_two_schedule_app();
        let frame_start = app.world.current_tick();
        app.world.last_check_tick =
            Tick::new(frame_start.get().wrapping_sub(CHECK_TICK_THRESHOLD - CHECK_TICK_PREEMPT_MARGIN));

        app.update_with_delta(STEP);

        assert_eq!(
            app.world.last_check_tick.get(),
            frame_start.get(),
            "the App pass must fire at frame start (pre-bump provenance); a post-bump \
             value means an internal block won the crossing — the ★C1 race"
        );
    }

    /// BOUNDARY WALK: elapsed forced to `T − MARGIN − 2` at frame start — the
    /// App pass holds, and the frame's ≤4 bumps cannot reach `T` (margin ≫
    /// per-frame consumption), so the internal blocks hold too (counter
    /// unchanged). The NEXT frame start crosses `T − MARGIN` and the App pass
    /// fires — sibling starvation is impossible by construction.
    #[test]
    fn c1_mid_frame_crossing_waits_for_next_frame_start() {
        let mut app = warm_two_schedule_app();
        let frame_start = app.world.current_tick();
        let forced = Tick::new(
            frame_start
                .get()
                .wrapping_sub(CHECK_TICK_THRESHOLD - CHECK_TICK_PREEMPT_MARGIN - 2),
        );
        app.world.last_check_tick = forced;

        // Frame A: neither the App pass (elapsed = T−M−2 < T−M) nor any
        // internal block (mid-frame max = T−M+2 < T) fires.
        app.update_with_delta(STEP);
        assert_eq!(
            app.world.last_check_tick.get(),
            forced.get(),
            "frame A: no clamp pass may fire below the margin line"
        );

        // Frame B: frame start elapsed = T−M+2 ≥ T−M ⇒ the App pass fires,
        // with frame-B-start provenance.
        let frame_b_start = app.world.current_tick();
        app.update_with_delta(STEP);
        assert_eq!(
            app.world.last_check_tick.get(),
            frame_b_start.get(),
            "frame B: the App pass fires at frame start, covering BOTH schedules"
        );
    }

    /// BEHAVIORAL: a dormant `Changed`-gated observation in the FIXED schedule
    /// survives the threshold crossing with a non-inverted window — the
    /// sibling clamp ran (the starvation outcome would be a spurious match
    /// after `MAX_CHANGE_AGE` wrap; here the window stays empty).
    #[test]
    fn c1_dormant_fixed_changed_window_survives_crossing() {
        use crate::ecs::core::iters::query::{Changed, Query};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let hits = Arc::new(AtomicU32::new(0));
        let h = Arc::clone(&hits);

        let mut app = serial_app();
        app.add_systems(|| {});
        app.add_systems_in(CoreSchedule::Fixed, move |q: Query<&Wob, Changed<Wob>>| {
            h.fetch_add(q.iter().count() as u32, Ordering::Relaxed);
        });
        app.finish();
        app.world
            .spawn_batch(std::iter::once(WobBundle { w: Wob(1) }))
            .expect("spawn");

        // Warm: the spawn's Changed window matches exactly once.
        app.update_with_delta(STEP);
        app.update_with_delta(STEP);
        assert_eq!(hits.load(Ordering::Relaxed), 1, "spawn observed once while warm");

        // Force the crossing; the App pass fires next frame and clamps BOTH
        // schedules' meta ticks.
        let cur = app.world.current_tick();
        app.world.last_check_tick =
            Tick::new(cur.get().wrapping_sub(CHECK_TICK_THRESHOLD - CHECK_TICK_PREEMPT_MARGIN));
        for _ in 0..3 {
            app.update_with_delta(STEP);
        }
        assert_eq!(
            hits.load(Ordering::Relaxed),
            1,
            "no spurious Changed match across the clamp crossing (window not inverted)"
        );
    }

    /// Component + 1-field bundle for the behavioral ★C1 test — the in-lib
    /// hand-written pattern (the derive lives in boyko-macros, a
    /// dev-dependency; cf. `impl_self_bundle!` used by `hierarchy::Children`).
    #[derive(Clone, Copy)]
    #[repr(C)]
    struct Wob(u32);

    impl crate::ecs::core::component::component::Component for Wob {
        #[inline]
        fn component_id() -> crate::ecs::identifiers::primitives::ComponentId {
            use crate::ecs::core::component::component_registry;
            use crate::ecs::identifiers::primitives::ComponentId;
            use std::sync::OnceLock;
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<Self>()))
        }
    }

    struct WobBundle {
        w: Wob,
    }
    impl crate::ecs::core::bundle::bundle::sealed::BundleSealed for WobBundle {}
    impl crate::ecs::core::bundle::Bundle for WobBundle {
        fn static_info() -> &'static crate::ecs::core::bundle::bundle::BundleStaticInfo {
            use crate::ecs::core::bundle::{bundle::BundleStaticInfo, bundle_type_registry};
            use crate::ecs::core::component::component::Component;
            use crate::ecs::identifiers::primitives::ComponentId;
            use std::sync::OnceLock;
            static INFO: OnceLock<BundleStaticInfo> = OnceLock::new();
            INFO.get_or_init(|| {
                let arr: [ComponentId; 1] = [<Wob as Component>::component_id()];
                let leaked: &'static [ComponentId; 1] = Box::leak(Box::new(arr));
                BundleStaticInfo {
                    type_id: bundle_type_registry::register_new(),
                    component_ids: leaked.as_slice(),
                }
            })
        }

        #[inline]
        fn cached_archetype_id(
            world: &mut EcsMaster,
        ) -> crate::ecs::identifiers::primitives::ArchetypeId {
            world.bundle_archetype_id_for::<Self>()
        }

        fn for_each_component_bytes<F>(self, mut f: F)
        where
            F: FnMut(crate::ecs::identifiers::primitives::ComponentId, &[u8]),
        {
            use crate::ecs::core::component::component::Component;
            let field = std::mem::ManuallyDrop::new(self.w);
            let id = <Wob as Component>::component_id();
            let ptr = &raw const *field as *const u8;
            let len = std::mem::size_of::<Wob>();
            // SAFETY (the reproduced derive C5 byte-erasure, hierarchy/bundles.rs
            // pattern): ptr derives from a live ManuallyDrop'd stack local, valid
            // for exactly size_of::<Wob>() bytes; the slice is the only borrow;
            // ownership transfers to the archetype on callback success.
            let bytes: &[u8] = unsafe { std::slice::from_raw_parts(ptr, len) };
            f(id, bytes);
        }
    }
}
