//! [`ExclusiveFunctionSystem`] — `System` adapter for `fn(&mut EcsMaster)`.
//!
//! See Phase 9 plan §5.3 (struct shape) and §2.5 EXC1..EXC4 (exclusive
//! system contract). Wave 3 Step 8 lands the adapter so user-written
//! exclusive bodies can flow through [`IntoSystem`] into the schedule via
//! the [`ExclusiveSystemMarker`] blanket (Q9.1 coherence proof).
//!
//! # Exclusive vs SystemParam-based systems
//!
//! `ExclusiveFunctionSystem<F>` mirrors [`FunctionSystem<F, Marker>`] (Phase
//! 8c) in shape — function body, cached [`SystemMeta`], initialised flag —
//! but differs in two key respects:
//!
//! 1. **Signature** — the body is `FnMut(&mut EcsMaster) -> ()`, not a
//!    `SystemParamFunction`. There is no param tuple, no per-param state,
//!    no `FilteredAccessSet` accumulation.
//! 2. **Access** — `meta.access` is set to [`Access::universal()`] at
//!    construction time (EXC2). The system conflicts with every other
//!    system in the conflict graph, forcing the scheduler to drain the
//!    apply window (`running == 0`) before dispatching it.
//!
//! # `Self::Out = ()`
//!
//! The plan's §5.3 signature shows `(self.func)(world_ref)` with no value
//! returned. The schedule executes systems via `Box<dyn System<Out = ()>>`
//! (SCH10 — Round 2 Q1), so non-unit outputs cannot flow through the
//! scheduler. Users who need a return value must call
//! `EcsMaster::run_system_once` outside the scheduler.
//!
//! [`FunctionSystem<F, Marker>`]: super::function_system::FunctionSystem
//! [`IntoSystem`]: super::into_system::IntoSystem
//! [`ExclusiveSystemMarker`]: super::into_system::ExclusiveSystemMarker
//! [`Access::universal()`]: super::access::Access::universal

use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::system::System;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// `System` adapter wrapping an `FnMut(&mut EcsMaster)` body.
///
/// Constructed via [`IntoSystem::into_system`] using the
/// [`ExclusiveSystemMarker`] blanket (see Phase 9 plan §3 Q9.1) or directly
/// via [`ExclusiveFunctionSystem::new`].
///
/// # Field order (plan §5.3)
///
/// `func → meta → initialized` — by access frequency from the dispatcher
/// hot path. The function body is invoked every dispatch; `meta.access`
/// is consulted by the conflict graph at build time and (cheaply) on the
/// `debug_assert!` in `Schedule::run`; `initialized` is a one-shot guard.
///
/// # Send + Sync
///
/// The struct inherits `Send + Sync` from `F: FnMut(&mut EcsMaster) +
/// Send + Sync + 'static` and `SystemMeta: Send + Sync`. The trait bound
/// is non-negotiable — Phase 9's scheduler migrates exclusive systems
/// across worker threads even though the body itself only runs on the
/// dispatcher (the system is stored in `SystemBox`, which travels through
/// `Schedule`'s `Box<[SystemBox]>`).
///
/// [`IntoSystem::into_system`]: super::into_system::IntoSystem::into_system
/// [`ExclusiveSystemMarker`]: super::into_system::ExclusiveSystemMarker
pub struct ExclusiveFunctionSystem<F>
where
    F: FnMut(&mut EcsMaster) + Send + Sync + 'static,
{
    /// User-supplied function body — invoked once per scheduler dispatch
    /// with an exclusive `&mut EcsMaster` reborrow (EXC1).
    func: F,

    /// Cached metadata. `meta.access` is seeded with [`Access::universal()`]
    /// in [`new`](Self::new) so the conflict graph picks up the
    /// exclusivity at build time without waiting on `initialize`.
    meta: SystemMeta,

    /// One-shot guard: `false` until `initialize` runs once. Subsequent
    /// `initialize` calls are no-ops (idempotent — matches FS1 from
    /// `FunctionSystem`).
    initialized: bool,
}

impl<F> ExclusiveFunctionSystem<F>
where
    F: FnMut(&mut EcsMaster) + Send + Sync + 'static,
{
    /// Constructs an exclusive system adapter from a function body.
    ///
    /// The constructor seeds [`SystemMeta::access`] with
    /// [`Access::universal()`] so the system's access surface is correct
    /// the instant the value is produced — before `initialize` runs. This
    /// matches EXC2: exclusive systems declare universal access at
    /// construction time, not at init time, so a builder that consults
    /// `system.access()` between construction and `initialize` (e.g. for
    /// early validation) sees the correct value.
    ///
    /// The diagnostic name is `std::any::type_name::<F>()`, matching
    /// `FunctionSystem::new`.
    ///
    /// [`Access::universal()`]: super::access::Access::universal
    /// [`SystemMeta::access`]: super::system_meta::SystemMeta
    #[inline]
    pub fn new(func: F) -> Self {
        // Phase 10 Wave D Step 14 — defer-to-initialize refit (Option B).
        // Constructor uses `for_testing` (sentinel current_tick = 1)
        // because no `world` is in scope; [`Self::initialize`] re-seeds
        // both ticks to `world.current_tick() - MAX_CHANGE_AGE` on first
        // init (mirroring `FunctionSystem::initialize`). The sentinel
        // values are functionally equivalent until init runs because the
        // dispatcher's first `set_change_ticks` promotes them before any
        // worker observes the meta (plan §9.4 PHASE9.4).
        let mut meta = SystemMeta::for_testing(std::any::type_name::<F>());
        // EXC2: exclusive systems declare universal access at construction
        // time. The conflict graph (Wave 4 Step 10) reads `access()` for
        // every system after `initialize`; seeding here is also defensive
        // against early `access()` consumers.
        meta.access = Access::universal();
        Self {
            func,
            meta,
            initialized: false,
        }
    }
}

// SAFETY (S1, EXC1):
//
//   * S1 — `run_unsafe` is gated by the exclusive-system contract: the
//     scheduler delivers an exclusive system to the dispatcher only when
//     `running.count_ones() == 0` (SCH7 apply-window barrier + EXC2). The
//     dispatcher then calls `run_unsafe` on the calling thread. No other
//     `System::run_unsafe` is in flight on the same world for the
//     duration of this call, satisfying the trait-level S1 invariant.
//
//   * EXC1 — the body receives `&mut EcsMaster` reborrowed from
//     `cell.world_mut()`. The cell was minted from `&mut EcsMaster` in
//     the current dispatch round (Round 2 O3 — per-round mint); the
//     reborrow is the sole live reference at the moment of the call. The
//     body must NOT retain any cell-derived borrow past return — the
//     dispatcher reborrows from the same cell immediately afterwards to
//     invoke `apply` (here a no-op, but the contract must hold for
//     symmetry with `FunctionSystem`).
//
//   * Send + Sync — `F: FnMut(&mut EcsMaster) + Send + Sync + 'static` and
//     `SystemMeta: Send + Sync` (composition of `Access` (Send + Sync by
//     value), `&'static str`, two `ArchetypeGeneration` values).
unsafe impl<F> System for ExclusiveFunctionSystem<F>
where
    F: FnMut(&mut EcsMaster) + Send + Sync + 'static,
{
    type Out = ();

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name()
    }

    #[inline]
    fn access(&self) -> &Access {
        self.meta.access()
    }

    fn initialize(&mut self, world: &mut EcsMaster) {
        // FS1-equivalent idempotency: skip the tick refit on re-init so the
        // dispatcher's `set_change_ticks` writes from the previous frame
        // are not clobbered if `initialize` is called again (e.g. user
        // code routing the same system through `run_cached_system`
        // multiple times after a manual rebuild).
        if !self.initialized {
            // Phase 10 Wave D Step 14 refit (plan §15.2 W5 + Option B):
            // mirror `FunctionSystem::initialize` — replace the Wave A
            // `for_testing` sentinel ticks with world-aware values now
            // that `world` is in scope. Both fields land at
            // `current - MAX_CHANGE_AGE` (pre-first-run sentinel; plan
            // §9.4 PHASE9.4); the dispatcher's first `set_change_ticks`
            // call promotes them.
            let current = world.current_tick();
            let last_run = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
            self.meta.last_run = last_run;
            self.meta.this_run = last_run;
        }

        // Exclusive systems have no `SystemParam` state to materialise and
        // no `FilteredAccessSet` to fold — `meta.access` was seeded by
        // `new`. The flag matters only for the `debug_assert!` in
        // `run_unsafe`; subsequent calls are idempotent (matches FS1).
        self.initialized = true;
    }

    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out {
        debug_assert!(
            self.initialized,
            "invariant EXC1: ExclusiveFunctionSystem::initialize must be \
             called before run_unsafe"
        );
        // SAFETY (EXC1):
        //   - Scheduler guarantees `running == 0` before dispatching an
        //     exclusive system (SCH7 + EXC2 apply-window barrier). No
        //     other worker holds a cell-mediated borrow of this world.
        //   - `world` was minted from `&mut EcsMaster` in the current
        //     round (Round 2 O3 — per-round mint). The reborrow yields
        //     the canonical exclusive borrow.
        //   - The body MUST NOT retain any reference past return; the
        //     dispatcher reborrows from the same cell for `apply` (here a
        //     no-op default, but the contract must hold for symmetry
        //     with `FunctionSystem` and any future override).
        let world_ref: &mut EcsMaster = unsafe { world.world_mut() };
        (self.func)(world_ref);
    }

    // `apply` defaults to a no-op — exclusive systems already took
    // `&mut World` and performed any flushing themselves. The default
    // trait body satisfies APP1'/APP4 trivially.

    /// Phase 10 Round 2 C1 — read-only meta accessor.
    #[inline]
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }

    /// Phase 10 Round 2 C1 — dispatcher-only tick snapshot write.
    ///
    /// Exclusive systems still participate in change detection (e.g. they
    /// may consume `SystemChangeTick` for diagnostics, Wave B+).
    #[inline]
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.last_run = last_run;
        self.meta.this_run = this_run;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Marker resource used inside the closure below. `AtomicUsize` is
    /// `Send + Sync` so the closure is too, satisfying the bound.
    static PROBE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// `new` seeds `meta.access` with `Access::universal()` immediately —
    /// before `initialize` runs. EXC2 acceptance criterion.
    #[test]
    fn new_seeds_universal_access() {
        let sys = ExclusiveFunctionSystem::new(|_w: &mut EcsMaster| {});
        assert!(
            sys.access().is_universal(),
            "ExclusiveFunctionSystem::new must seed Access::universal()"
        );
    }

    /// `initialize` is idempotent; double-calling does not corrupt the
    /// access surface (matches FS1).
    #[test]
    fn initialize_is_idempotent() {
        let mut ecs = EcsMaster::new();
        let mut sys = ExclusiveFunctionSystem::new(|_w: &mut EcsMaster| {});
        sys.initialize(&mut ecs);
        assert!(sys.access().is_universal());
        sys.initialize(&mut ecs);
        assert!(sys.access().is_universal());
    }

    /// Driving the system through `run_system_once` exercises the full
    /// `initialize → run_unsafe` path. The body increments a static
    /// counter so the side effect is observable from the test.
    #[test]
    fn run_unsafe_invokes_body() {
        PROBE_COUNTER.store(0, Ordering::Relaxed);
        let mut ecs = EcsMaster::new();
        let mut sys = ExclusiveFunctionSystem::new(|_w: &mut EcsMaster| {
            PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
        });
        ecs.run_system_once(&mut sys);
        assert_eq!(PROBE_COUNTER.load(Ordering::Relaxed), 1);
    }

    /// Diagnostic name is the function's `type_name`, matching the
    /// `FunctionSystem` convention.
    #[test]
    fn name_is_type_name_of_body() {
        fn exclusive_body(_w: &mut EcsMaster) {}
        let sys = ExclusiveFunctionSystem::new(exclusive_body);
        // The fn-item name is implementation-defined but stable across
        // calls; we only assert it is non-empty and contains the function
        // identifier.
        assert!(
            sys.name().contains("exclusive_body"),
            "name() should contain the function identifier, got {}",
            sys.name()
        );
    }
}
