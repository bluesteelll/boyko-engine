//! The `System` trait — type-erased handle every concrete system implements.
//!
//! See Phase 8a plan §8 (Decision D6) and the Phase 8c plan §5 (Decision C3).
//! Phase 8c ships [`FunctionSystem<F, M>`] in [`super::function_system`] via
//! the `IntoSystem` adapter; it plugs into both
//! [`EcsMaster::run_system_once`] (typed `&mut S`) and the new
//! [`EcsMaster::run_system`] / [`EcsMaster::run_cached_system`] entry points
//! generically, with no virtual dispatch.
//!
//! # Why `unsafe trait`
//!
//! Implementations of [`System::run_unsafe`] take an [`UnsafeEcsCell`] —
//! the by-value, raw-pointer-mediated handle on the world. Calling
//! `run_unsafe` while another `System` is live on the same world is UB
//! (raw-pointer aliasing without the borrow checker's protection). The
//! invariant cannot be expressed in the type system, so the trait carries
//! it as a safety contract — see invariant S1 below.
//!
//! [`FunctionSystem<F, M>`]: super::function_system::FunctionSystem
//! [`UnsafeEcsCell`]: super::unsafe_ecs_cell::UnsafeEcsCell
//! [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
//! [`EcsMaster::run_system`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system
//! [`EcsMaster::run_cached_system`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_cached_system

use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::dispatcher_token::DispatcherToken;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Type-erased system handle. Every concrete system implements this trait;
/// [`EcsMaster::run_system_once`] is generic over `S: System` so the caller's
/// system survives across calls without `Box<dyn System>`.
///
/// # `Send + Sync + 'static`
///
/// Required so the Phase 9 scheduler can migrate systems across worker
/// threads. The bound is non-negotiable — Phase 8c's [`FunctionSystem`] impl
/// satisfies it trivially (closures wrapped via `IntoSystem` inherit
/// `Send + Sync` from their captures; [`SystemMeta`] and `P::State` are
/// `Send + Sync` by construction).
///
/// [`FunctionSystem`]: super::function_system::FunctionSystem
///
/// # Safety
///
/// **S1** — The caller of [`run_unsafe`](Self::run_unsafe) must assert that
/// no other `System::run_unsafe` is in flight on the same [`EcsMaster`].
/// Phase 9's scheduler enforces this via the [`Access`] conflict graph;
/// Phase 8a's [`EcsMaster::run_system_once`] enforces it trivially by
/// taking `&mut EcsMaster` (the borrow checker forbids re-entry while the
/// system holds the exclusive borrow).
///
/// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
/// [`SystemMeta`]: super::system_meta::SystemMeta
pub unsafe trait System: Send + Sync + 'static {
    /// Output of the system body — typically `()`, occasionally a value
    /// extracted by the body for the caller (e.g. a probe in tests).
    type Out;

    /// Returns the diagnostic name of the system. For
    /// [`FunctionSystem<F, M>`](super::function_system::FunctionSystem)
    /// this is `std::any::type_name::<F>()`.
    fn name(&self) -> &'static str;

    /// Returns the declared [`Access`] surface. Empty until
    /// [`initialize`](Self::initialize) runs; populated thereafter by the
    /// system's `SystemParam` chain.
    fn access(&self) -> &Access;

    /// Two-phase initialisation: builds the per-system state and declares
    /// the access surface. Idempotent — calling twice has no additional
    /// effect.
    ///
    /// Called once per system before the first [`run_unsafe`](Self::run_unsafe)
    /// invocation. The Phase 8a [`EcsMaster::run_system_once`] calls it
    /// implicitly; Phase 9's scheduler will call it at system registration
    /// time.
    ///
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    fn initialize(&mut self, world: &mut EcsMaster);

    /// Runs the system body once.
    ///
    /// # Safety
    ///
    /// **S1** — The caller must guarantee that no other `System::run_unsafe`
    /// is in flight on the same world for the duration of this call. See
    /// the trait-level safety section for the full rationale; Phase 8a's
    /// [`EcsMaster::run_system_once`] enforces the invariant trivially by
    /// taking `&mut EcsMaster`.
    ///
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out;

    /// Phase 5 Option C — defense-in-depth GPU-compute marker.
    ///
    /// Returns `false` by default (every CPU system). A hand-written
    /// out-of-crate `System` that dispatches GPU work (the `boyko_render`
    /// `GpuSystem`) overrides this to `true`. The schedule builder consults it
    /// (in addition to the explicit `SystemConfig::gpu()` descriptor flag) so a
    /// system that IS GPU-resident cannot be mis-resolved to a CPU kind even if
    /// the caller forgot the explicit opt-in — the dispatcher-solo discipline a
    /// `GpuCompute` system relies on is then never silently dropped.
    #[inline]
    fn is_gpu(&self) -> bool {
        false
    }

    /// Phase 5 Option C — the dispatcher-solo run entry point.
    ///
    /// Called instead of [`run_unsafe`](Self::run_unsafe) on the dispatcher-solo
    /// path (the scheduler's `running == 0` window and
    /// [`EcsMaster::run_system_once`]). The default forwards to `run_unsafe`
    /// through `DispatcherToken::into_cell`, so every existing system is
    /// byte-identical — only a system needing `!Send` access (the
    /// `boyko_render` `GpuSystem`) overrides it to project through the
    /// [`DispatcherToken`] instead of the [`UnsafeEcsCell`].
    ///
    /// # Safety
    ///
    /// **S1'** — The caller MUST guarantee `running == 0` on the dispatcher for
    /// the duration of this call (no worker live, no other `run_unsafe` /
    /// `run_dispatcher` in flight on the same world). The [`DispatcherToken`] is
    /// mintable ONLY in that context (see `DispatcherToken::new`), so passing
    /// one IS the witness that S1' holds.
    ///
    /// [`DispatcherToken`]: super::dispatcher_token::DispatcherToken
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    #[inline]
    unsafe fn run_dispatcher(&mut self, token: DispatcherToken<'_>) -> Self::Out {
        // SAFETY (S1'): the caller guarantees `running == 0` (the token is
        //   mintable only there). `token.into_cell()` reconstructs the
        //   write-capable cell from the same `&mut EcsMaster` provenance that
        //   minted the token; the dispatcher-solo context upholds the cell's S1
        //   contract.
        unsafe { self.run_unsafe(token.into_cell()) }
    }

    /// Hook for deferred mutations after [`run_unsafe`](Self::run_unsafe)
    /// returns. The default implementation is a no-op; concrete systems
    /// that need to flush per-`SystemParam` buffers override this and
    /// forward to [`SystemParam::apply`].
    ///
    /// # Phase 8d usage
    ///
    /// `FunctionSystem<F, M>` overrides `apply` to invoke
    /// `<F::Param as SystemParam>::apply(state, &mut self.meta, world)`,
    /// which flushes `Commands<'s>`'s `CommandQueue` into the world.
    ///
    /// # Invariants
    ///
    /// * **APP1' (O3' — Round 3 documented)** — `apply` is a SAFE method.
    ///   The caller holds `&mut EcsMaster` exclusively; there is no
    ///   aliasing risk, no [`UnsafeEcsCell`] involved.
    /// * **APP4** — Implementations MUST NOT re-enter
    ///   [`EcsMaster::run_system_once`] / `run_closure_once` while inside
    ///   `apply`. The borrow checker enforces this trivially (apply
    ///   already holds `&mut EcsMaster`).
    ///
    /// [`SystemParam::apply`]: super::system_param::SystemParam::apply
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    #[inline]
    fn apply(&mut self, _world: &mut EcsMaster) {}

    /// Phase 10 Round 2 C1 — read-only accessor for [`SystemMeta`].
    ///
    /// Returns this system's cached meta so the dispatcher can read the
    /// system's previous `this_run` (to derive the new `last_run`),
    /// inspect declared [`Access`], and forward `&SystemMeta` to
    /// `SystemParam::get_param` / `Query` constructors that consume the
    /// tick snapshot (Wave B+).
    ///
    /// # Why a trait method
    ///
    /// `Box<dyn System>` erases the concrete type, so the scheduler has
    /// no way to read `self.meta` directly. A trait getter is the
    /// minimal extension that preserves the type erasure while still
    /// exposing the snapshot — see plan §4.5 ("`sb.meta()` is a new safe
    /// getter").
    fn meta(&self) -> &SystemMeta;

    /// Phase 10 Round 2 C1 — single dispatcher→system channel for tick
    /// snapshot writes.
    ///
    /// Called by `Schedule::run` (Wave D Step 13) before each
    /// [`run_unsafe`](Self::run_unsafe) dispatch and inside the
    /// post-`check_ticks` clamp loop (Wave D Step 13). Implementations
    /// MUST write `last_run` and `this_run` into the cached
    /// [`SystemMeta`] in place; they MUST NOT allocate, lock, or
    /// re-enter the scheduler.
    ///
    /// # Why no default body
    ///
    /// The plan §2.6 SCT4 + §5.4-bis make this a load-bearing contract:
    /// a System impl that "forgets" to update its tick snapshot would
    /// silently report wrong `Changed<T>` results for every frame after
    /// the first. Forcing every impl to declare `set_change_ticks`
    /// without a default body makes that mistake unrepresentable.
    ///
    /// # Invariants
    ///
    /// * Called only during the apply window (no worker live on this
    ///   system) — guaranteed by `Schedule::run`'s scope-spawn happens-before
    ///   chain (plan §2.6 SCT4 flow + §8.2).
    /// * On first dispatch after construction, `last_run` is the system's
    ///   constructor-time `last_run` (set by
    ///   [`SystemMeta::new`](super::system_meta::SystemMeta::new) to
    ///   `current_tick - MAX_CHANGE_AGE`); `this_run` is the current
    ///   frame's value.
    /// * On every subsequent dispatch, `last_run` is the PREVIOUS frame's
    ///   `this_run`.
    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick);

    /// Phase 16.1 (Gap #2) — wraparound clamp for this system's tick snapshot.
    ///
    /// Clamps both `last_run` and `this_run` to be no older than
    /// [`MAX_CHANGE_AGE`] ticks behind `current` (via [`Tick::check_tick`]).
    /// Called by `Schedule::check_change_ticks` on the cold
    /// `CHECK_TICK_THRESHOLD` path, right after the per-row pool scan, so that
    /// a `last_run` left un-refreshed across a long dormant span (Phase 16.1
    /// advances ticks only when a system actually runs) can never silently
    /// flip [`Tick::is_newer_than`].
    ///
    /// # Why no default body
    ///
    /// Mirrors [`set_change_ticks`](Self::set_change_ticks): the clamp is a
    /// load-bearing correctness contract, so an impl that "forgets" it must
    /// not compile. Forcing every impl to declare `check_change_tick` without
    /// a default body makes that mistake unrepresentable.
    ///
    /// # Invariants
    ///
    /// * Called only during the apply window (no worker live on this system),
    ///   the same exclusivity guarantee as `set_change_ticks`.
    ///
    /// [`MAX_CHANGE_AGE`]: crate::ecs::core::change_detection::MAX_CHANGE_AGE
    /// [`Tick::check_tick`]: crate::ecs::core::change_detection::Tick::check_tick
    /// [`Tick::is_newer_than`]: crate::ecs::core::change_detection::Tick::is_newer_than
    fn check_change_tick(&mut self, current: Tick);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal compile-only impl proving the trait shape is buildable. The
    /// concrete `FunctionSystem` impl lives in
    /// [`super::super::function_system`]; this one exists so unit tests in
    /// the same crate can exercise the trait's bare contract without
    /// pulling in `SystemParam` machinery.
    struct NoopSystem {
        meta: SystemMeta,
    }

    // SAFETY (S1): `run_unsafe` performs no world access; the contract is
    //   vacuously upheld for any caller.
    unsafe impl System for NoopSystem {
        type Out = ();

        fn name(&self) -> &'static str {
            self.meta.name()
        }

        fn access(&self) -> &Access {
            self.meta.access()
        }

        fn initialize(&mut self, _world: &mut EcsMaster) {}

        unsafe fn run_unsafe(&mut self, _world: UnsafeEcsCell<'_>) -> Self::Out {}

        fn meta(&self) -> &SystemMeta {
            &self.meta
        }

        fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }

        fn check_change_tick(&mut self, current: Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
        }
    }

    /// Compile-only assertion that `NoopSystem: System` (and therefore
    /// `Send + Sync + 'static`).
    fn assert_impl<S: System>() {
        let _ = std::marker::PhantomData::<S>;
    }

    #[test]
    fn noop_system_implements_trait() {
        assert_impl::<NoopSystem>();
        let sys = NoopSystem {
            meta: SystemMeta::for_testing("noop"),
        };
        assert_eq!(sys.name(), "noop");
    }

    /// **Phase 10 Round 2 C1 — load-bearing regression test (plan §13.1).**
    ///
    /// `System::set_change_ticks` writes both ticks into the cached
    /// `SystemMeta`. The dispatcher's contract relies on this being the
    /// only write site per frame.
    #[test]
    fn noop_system_set_change_ticks_writes_meta() {
        let mut sys = NoopSystem {
            meta: SystemMeta::for_testing("set_change_ticks_probe"),
        };
        let last = Tick::new(7);
        let this = Tick::new(11);
        sys.set_change_ticks(last, this);
        assert_eq!(sys.meta().last_run(), last);
        assert_eq!(sys.meta().this_run(), this);
    }
}
