//! The `SystemParamFunction` trait + `FunctionSystem<F, Marker>` struct.
//!
//! See Phase 8c+8d plan §4 (Decision C2) for the `SystemParamFunction`
//! trait shape (double-`FnMut` HRTB bound), §5 (Decision C3) for the
//! [`FunctionSystem`] field layout, and §6 (Decision C4) for the wiring
//! into [`IntoSystem`].
//!
//! Phase 8c Step 3 wires the cached-state [`System`] impl: `initialize`
//! materialises `<F::Param as SystemParam>::State` and freezes the
//! [`Access`] surface into [`SystemMeta`]; `run_unsafe` hits the hot path
//! with the cached state, satisfying the ≤30 ns hoisted target (plan
//! §1.2).
//!
//! [`Access`]: super::access::Access
//! [`IntoSystem`]: super::into_system::IntoSystem
//! [`System`]: super::system::System

use std::marker::PhantomData;

use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system::System;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Classifies a function `F: FnMut(P0, ..., Pn) -> Out` as a candidate
/// for wrapping in a [`FunctionSystem`].
///
/// The trait is keyed by a `Marker` so the variadic blanket impls (one
/// per arity, 0..=12; see plan §7) remain disjoint — `Marker` is typically
/// `fn(P0, P1, ..., Pn) -> Out`, which uniquely encodes the arity and
/// param tuple. The blanket impls land in Step 2; Phase 8c Step 1 ships
/// only the trait declaration.
///
/// # The double-`FnMut` HRTB bound (plan §4.7, invariant FS3)
///
/// Concrete blanket impls bound `for<'a> &'a mut Func: FnMut(P) -> Out +
/// FnMut(SystemParamItem<'_, '_, P>) -> Out`. The double bound is
/// load-bearing for rustc's closure-argument inference: it lets the user
/// write `|q: Query<&Position>|` without explicit `<'_, '_>` annotations.
/// The §4.7 reproducer (Step 2 acceptance gate) validates the pattern on
/// stable rustc ≥ 1.85.
///
/// # Type parameter — `Marker`
///
/// Disambiguator. Has no runtime meaning; the variadic blanket impls in
/// Step 2 use `Marker = fn(P0, ..., Pn) -> Out` to encode arity + param
/// tuple uniquely.
pub trait SystemParamFunction<Marker>: Send + Sync + 'static {
    /// Input type plumbed through to the function body. Phase 8c uses
    /// `()` exclusively; Phase 9's chained-system support will exercise
    /// non-unit `In`.
    type In;

    /// Output of the function body.
    type Out;

    /// The tuple of [`SystemParam`] types accepted by the function. Phase 8c
    /// covers tuples of arity 0..=12 via the variadic blanket impl in
    /// Step 2.
    type Param: SystemParam;

    /// Invokes `self` with the deconstructed param tuple.
    ///
    /// Step 2's variadic blanket impl forwards to `self(p0, p1, ..., pn)`
    /// after destructuring the `Item` GAT tuple; see plan §4.2.
    fn run(
        &mut self,
        input: Self::In,
        params: <Self::Param as SystemParam>::Item<'_, '_>,
    ) -> Self::Out;
}

/// Cached, runnable system built from any `F: SystemParamFunction<Marker>`.
///
/// Holds the function body, the cached `<F::Param as SystemParam>::State`,
/// the cached [`SystemMeta`], and the marker phantom. After the first call
/// to `initialize(world)`, the state is built; subsequent `run_unsafe`
/// calls hit the hot path without re-init.
///
/// # Field order (plan §19.1)
///
/// `func → state → meta → _marker` — access-frequency descending. The
/// function is invoked every call; the state every call (after first
/// init); the meta only at access-graph queries and final apply; the
/// marker is a ZST.
///
/// # Invariants (plan §18)
///
/// * **FS1** — `initialize` is idempotent. Re-initialising overwrites
///   `state` and `meta` deterministically.
/// * **FS2** — `state` and `meta` are cached across `run_unsafe` and
///   `apply` calls; the next `run_unsafe` skips re-init.
///
/// # Send + Sync
///
/// The struct inherits `Send + Sync` from:
/// * `F: SystemParamFunction<Marker>` — required `Send + Sync + 'static`.
/// * `<F::Param as SystemParam>::State` — required `Send + Sync + 'static`
///   by the [`SystemParam`] trait.
/// * [`SystemMeta`] — `Send + Sync` by composition.
/// * `PhantomData<fn() -> Marker>` — `Send + Sync` regardless of `Marker`
///   (function pointers are `Send + Sync`; the `fn()` wrapper makes the
///   phantom variance covariant and the bound trivially satisfied).
///
/// [`SystemMeta`]: super::system_meta::SystemMeta
/// [`SystemParam`]: super::system_param::SystemParam
pub struct FunctionSystem<F, Marker>
where
    F: SystemParamFunction<Marker>,
{
    pub(crate) func: F,
    pub(crate) state: Option<<F::Param as SystemParam>::State>,
    pub(crate) meta: SystemMeta,
    pub(crate) _marker: PhantomData<fn() -> Marker>,
}

impl<F, Marker> FunctionSystem<F, Marker>
where
    F: SystemParamFunction<Marker>,
{
    /// Constructs an uninitialised [`FunctionSystem`].
    ///
    /// `state` starts as `None`; the first call to `initialize` populates
    /// it. `meta` starts seeded via [`SystemMeta::for_testing`] (sentinel
    /// `current_tick = Tick::new(1)`) so the value is buildable without
    /// `world` access — Wave D Step 14 refit defers the world-aware tick
    /// seeding to `initialize` (Option B; plan §15.2 W5).
    ///
    /// `meta.name` is set to `std::any::type_name::<F>()` so panics and
    /// access-graph dumps point at the user's function.
    ///
    /// # Phase 10 Wave D Step 14 — defer-to-initialize refit (Option B)
    ///
    /// `FunctionSystem::new(func)` does not have `world` access, so the
    /// constructor uses the `for_testing` sentinel. [`Self::initialize`]
    /// then re-seeds both ticks to `world.current_tick() - MAX_CHANGE_AGE`
    /// (mirroring [`SystemMeta::new`]) on the first init call (FS1
    /// short-circuits subsequent re-inits). Until `initialize` runs the
    /// meta carries the sentinel value, which is functionally equivalent
    /// for the pre-first-run analysis (plan §9.4 PHASE9.4) because the
    /// dispatcher's first `set_change_ticks` call promotes the values
    /// before any worker observes them.
    pub fn new(func: F) -> Self {
        Self {
            func,
            state: None,
            meta: SystemMeta::for_testing(std::any::type_name::<F>()),
            _marker: PhantomData,
        }
    }
}

// SAFETY (S1): `FunctionSystem` holds only `F: SystemParamFunction<Marker>`
//   (`Send + Sync + 'static` by the trait bound), `Option<P::State>`
//   (`P::State: Send + Sync + 'static` by `SystemParam::State`),
//   `SystemMeta` (`Send + Sync` by composition), and a phantom — all
//   `Send + Sync + 'static`. `run_unsafe` upholds S1 by delegating
//   exclusively to `<P as SystemParam>::get_param` under invariant SP2,
//   which the caller of `run_unsafe` is contractually bound to satisfy
//   (typically `EcsMaster::run_system_once`, which mints
//   `UnsafeEcsCell::new_mutable` from `&mut EcsMaster` — see Phase 8a §13).
unsafe impl<F, Marker> System for FunctionSystem<F, Marker>
where
    F: SystemParamFunction<Marker, In = ()>,
    Marker: 'static,
{
    type Out = <F as SystemParamFunction<Marker>>::Out;

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name
    }

    #[inline]
    fn access(&self) -> &Access {
        &self.meta.access
    }

    fn initialize(&mut self, world: &mut EcsMaster) {
        // FS1: idempotent re-init. The scheduler / `run_system_once` call
        // `initialize` unconditionally before every `run_unsafe`; the
        // second call must be a no-op so the declared access surface
        // (populated below) stays stable.
        if self.state.is_some() {
            return;
        }

        // Phase 10 Wave D Step 14 refit (plan §15.2 W5 + Option B):
        // replace the Wave A `for_testing` sentinel tick snapshot with
        // world-aware values. `FunctionSystem::new` does not have access
        // to `world`, so it seeds `meta` with the `Tick(1)` sentinel;
        // here, at the first `initialize` call (FS1-guarded), we re-seed
        // both ticks to mirror `SystemMeta::new(name, world.current_tick())`.
        //
        // Both fields equal `current - MAX_CHANGE_AGE` (pre-first-run
        // sentinel — plan §9.4 PHASE9.4): on the first `Schedule::run`
        // dispatch, `set_change_ticks(prev_this_run, this_run)` promotes
        // `last_run = current - MAX_CHANGE_AGE` and `this_run = current + 1`,
        // so every pre-existing per-row tick reports as "Changed since
        // last run" — the desired late-added-system semantic.
        let current = world.current_tick();
        let last_run = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        self.meta.last_run = last_run;
        self.meta.this_run = last_run;

        // SP4 debug guard: snapshot the archetype generation before the
        // init sweep so we can `debug_assert_eq!` after it. `init_state` /
        // `init_access` must not register new archetypes or resources.
        #[cfg(debug_assertions)]
        let gen_before = world.archetype_master().archetype_generation();

        // Two-phase init (C4 RESOLUTION):
        //   1. `init_state` materialises per-param caches (e.g. cached
        //      `ResourceId` in `ResState<R>`).
        //   2. `init_access` declares the read/write surface into a fresh
        //      `FilteredAccessSet`, which folds the accumulated `Access`
        //      into `self.meta` via `finalize`.
        let state = <F::Param as SystemParam>::init_state(world, &mut self.meta);
        let mut access_set = FilteredAccessSet::new();
        <F::Param as SystemParam>::init_access(
            &state,
            &mut self.meta,
            &mut access_set,
            world,
        );
        access_set.finalize(&mut self.meta);
        self.state = Some(state);

        // SP4 enforcement (release-elided): no structural mutation
        // permitted during init.
        #[cfg(debug_assertions)]
        {
            let gen_after = world.archetype_master().archetype_generation();
            debug_assert_eq!(
                gen_before, gen_after,
                "invariant SP4: SystemParam::init_state/init_access must not register \
                 new archetypes or resources. Use a separate `EcsMaster::insert_resource` \
                 / `EcsMaster::get_or_create_archetype` call before running the system."
            );
        }
    }

    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out {
        // FS2: `state` is cached; `initialize` must have run first.
        let state = self
            .state
            .as_mut()
            .expect("invariant FS2: initialize must be called before run_unsafe");

        // SAFETY (S1, SP1, SP2):
        //   - S1: the caller of `run_unsafe` upholds the trait-level
        //     contract (no other system in flight on the same world).
        //   - SP1: the param chain's `init_access` declared every read /
        //     write the function will perform via the `FilteredAccessSet`
        //     accumulator; intra-system conflicts panicked at init.
        //   - SP2: `run_system_once` mints `world` from `&mut EcsMaster`,
        //     so no sibling reference aliases through any cell copy for
        //     the duration of this call.
        let params = unsafe {
            <F::Param as SystemParam>::get_param(state, &self.meta, world)
        };

        self.func.run((), params)
    }

    /// APP1' + APP3 — forward to `<F::Param as SystemParam>::apply`.
    ///
    /// Drains every per-param deferred buffer (Phase 8d `Commands` flushes
    /// its `CommandQueue` here). No-op when the system was never
    /// `initialize`d (state is `None`).
    #[inline]
    fn apply(&mut self, world: &mut EcsMaster) {
        if let Some(state) = self.state.as_mut() {
            <F::Param as SystemParam>::apply(state, &self.meta, world);
        }
    }

    /// Phase 10 Round 2 C1 — read-only meta accessor.
    #[inline]
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }

    /// Phase 10 Round 2 C1 — dispatcher-only tick snapshot write.
    ///
    /// Writes `last_run` and `this_run` into the cached [`SystemMeta`].
    /// Workers read these through `&SystemMeta` captured by Query /
    /// SystemChangeTick (Wave B+); the spawn happens-before chain
    /// (plan §8.2) makes the writes visible.
    #[inline]
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.last_run = last_run;
        self.meta.this_run = this_run;
    }

    /// Phase 16.1 (Gap #2) — wraparound clamp for this system's tick snapshot.
    ///
    /// Clamps both ticks behind `current` on the cold check-ticks path. Phase
    /// 16.1 stamps a gated system's ticks only on a frame it runs, so a
    /// long-dormant `last_run` can drift past `MAX_CHANGE_AGE`; this guard
    /// pulls it back to the oldest still-valid tick.
    #[inline]
    fn check_change_tick(&mut self, current: Tick) {
        self.meta.last_run = self.meta.last_run.check_tick(current);
        self.meta.this_run = self.meta.this_run.check_tick(current);
    }
}
