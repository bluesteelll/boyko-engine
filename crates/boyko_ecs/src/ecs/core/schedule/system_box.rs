//! [`SystemBox`] — schedule-internal wrapper around a type-erased system.
//!
//! See Phase 9 plan §5.2 (Round 2 + Round 3) and Phase 4 D5 / CR-B — the
//! dispatch classifier is a build-time cache (`kind: SystemKind`) populated
//! from `Access::is_universal` plus the `SystemDescriptor::is_gpu` marker,
//! NOT a `SystemMeta` field. Wave 3 Step 8 shipped the minimum surface the
//! executor needs to pick dispatcher-only systems on the apply-window
//! barrier without touching the erased `dyn System` at dispatch time;
//! Phase 4 Wave 4 widened the cached `is_exclusive: bool` to the 3-valued
//! [`SystemKind`] (same 1-byte slot).
//!
//! # Why a cache (Round 2 C9 / OQ-4)
//!
//! `Access::is_universal` is a four-bitmask scan (192 B touched). Hot-path
//! readers (the executor's dispatch loop) consult `kind` once per system
//! per round; reading the cached 1-byte tag spares the bitmask scan and
//! keeps the dispatcher's L1d footprint tight. The build-time freeze is
//! sound because `System::initialize` runs exactly once per system before
//! the schedule starts (SCH1) and `Access` is write-once thereafter
//! (FilteredAccessSet contract).
//!
//! SCH15 invariant (Round 3 / Phase 4 CR-B): `Schedule::run` carries a
//! release-cheap `debug_assert!` that `(kind == CpuExclusive) ==
//! system.access().is_universal()` to catch a desync if a future refactor
//! forgets to recompute the cache after re-initialising a system.

use crate::ecs::core::system::system::System;
use crate::ecs::core::system::system_kind::SystemKind;

/// Type-erased, read-only **run condition** (Phase 16).
///
/// A condition is any `impl IntoSystem<(), bool, M>` boxed into a
/// `dyn System<Out = bool>`. It is stored entirely OUTSIDE [`SystemBox`]
/// (which is pinned to `Out = ()`); the executor reaches it only through
/// the gated Step 1.5 condition-eval pass, never on the no-condition hot
/// path. See `PHASE-16-PLAN.md` §2.1.
///
/// The boxed system is `Send + Sync + 'static` because [`System`] carries
/// the same bound, so a `BoolSystem` migrates across worker threads with
/// the owning [`Schedule`] just like a regular system.
pub(crate) type BoolSystem = Box<dyn System<Out = bool>>;

/// Schedule-owned wrapper around a type-erased system body plus cached
/// dispatcher metadata.
///
/// # Dead-code allowance
///
/// Wave 3 Step 8 lands the struct + `new` constructor without a
/// consumer; `ScheduleBuilder::build` (Wave 4 Step 9) is the sole call
/// site. The lint is silenced here rather than crate-wide so the absence
/// of consumers is intentional at this checkpoint.
#[allow(dead_code)]
///
/// Constructed by [`ScheduleBuilder::build`] (Wave 4 Step 9) after every
/// system has been `initialize`d — the cache reads `access.is_universal()`
/// at that point and freezes it. The struct is `pub(crate)` because the
/// scheduling internals (`Schedule`, `ConflictGraph`, `Executor`) consume
/// it directly without an abstraction layer.
///
/// # Field order
///
/// `system → kind → name` — by access frequency from the dispatcher's hot
/// loop. The boxed system pointer is touched every dispatch (to call
/// `run_unsafe` / `apply`); the `kind` tag is read ONLY at the EXC2 dispatch
/// branch (`runs_on_dispatcher()` in `try_dispatch_ready`) and the SCH15
/// `debug_assert!` in `Schedule::run` — NOT at the apply-window gate, which is
/// pending/running-count based and never reads `kind` (FIX-7 / SCH15-M2); and
/// `name` is touched only for diagnostics / panic messages.
///
/// # Layout (Phase 4 D5)
///
/// `Box<dyn System<Out = ()>>` is a fat pointer (16 B); the
/// [`SystemKind`] tag is a `#[repr(u8)]` 1-byte value (occupying the exact
/// slot the previous `is_exclusive: bool` did) + 7 B padding + `&'static
/// str` (16 B fat pointer) = 40 B total under natural alignment. The struct
/// fits in a single cache line and is unchanged in size by the
/// `is_exclusive → kind` swap.
///
/// [`ScheduleBuilder::build`]: <Wave 4 Step 9 — not yet implemented>
pub(crate) struct SystemBox {
    /// Erased system body. The trait object is `Send + Sync + 'static`
    /// because [`System`] carries the same bound.
    pub(crate) system: Box<dyn System<Out = ()>>,

    /// Build-time dispatch classification (Phase 4 D5). Resolved at
    /// [`ScheduleBuilder::build`] from `(is_gpu, access().is_universal())`:
    /// the GPU marker wins, else universal access ⇒ `CpuExclusive`, else
    /// `CpuConcurrent`. Read ONLY by the executor's EXC2 dispatcher branch
    /// (`runs_on_dispatcher()` in `try_dispatch_ready`) and the SCH15
    /// `debug_assert!` in `Schedule::run` — NOT by the apply-window gate, which
    /// is pending/running-count based (FIX-7 / SCH15-M2) — via
    /// [`SystemKind::runs_on_dispatcher`].
    pub(crate) kind: SystemKind,

    /// Build-time cache of `system.name()` — typically
    /// `std::any::type_name::<F>()` for `FunctionSystem` /
    /// `ExclusiveFunctionSystem`. Read only on diagnostic paths (panic
    /// messages, `scheduler-trace` counters).
    pub(crate) name: &'static str,
}

#[allow(dead_code)] // Wave 3 Step 8 — consumed by ScheduleBuilder::build (Wave 4 Step 9).
impl SystemBox {
    /// Wraps an already-initialised system, resolving its [`SystemKind`]
    /// from access alone (`CpuExclusive` iff `access().is_universal()`, else
    /// `CpuConcurrent`) and caching its diagnostic name.
    ///
    /// The GPU-marker promotion to [`SystemKind::GpuCompute`] is NOT done
    /// here — it rides the `SystemDescriptor::is_gpu` flag and is applied at
    /// the `ScheduleBuilder::build` resolution site, which re-reads access
    /// after `initialize` and overwrites `kind` accordingly. `new` therefore
    /// produces a value byte-identical to the previous `is_exclusive`
    /// derivation, widened to 2-of-3 of the enum.
    ///
    /// # Invariants
    ///
    /// * The caller must invoke `system.initialize(world)` **before**
    ///   constructing the box. `Access::is_universal()` reads the
    ///   `Access` surface; calling it on an un-initialised system would
    ///   resolve `CpuConcurrent` (false negative — the system would never
    ///   be recognised as exclusive).
    /// * Once boxed, the system's `Access` is treated as frozen. Any
    ///   subsequent change to the access surface desyncs the cache and
    ///   trips the `debug_assert!` in `Schedule::run` (Round 3 SCH15 /
    ///   Phase 4 CR-B).
    ///
    /// `ScheduleBuilder::build` (Wave 4 Step 9) is the sole call site.
    #[inline]
    pub(crate) fn new(system: Box<dyn System<Out = ()>>) -> Self {
        let kind = if system.access().is_universal() {
            SystemKind::CpuExclusive
        } else {
            SystemKind::CpuConcurrent
        };
        let name = system.name();
        Self { system, kind, name }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::system::access::Access;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

    /// Minimal `System` impl whose declared access is controlled by the
    /// test — used to drive `SystemBox::new` through both the exclusive and
    /// non-exclusive code paths without dragging in `FunctionSystem` /
    /// `SystemParam` machinery.
    struct ProbeSystem {
        meta: SystemMeta,
    }

    // SAFETY (S1): `run_unsafe` is empty; the trait-level aliasing contract
    //   is vacuously upheld.
    unsafe impl System for ProbeSystem {
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

        fn set_change_ticks(
            &mut self,
            last_run: crate::ecs::core::change_detection::Tick,
            this_run: crate::ecs::core::change_detection::Tick,
        ) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }

        fn check_change_tick(&mut self, current: crate::ecs::core::change_detection::Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
        }
    }

    /// `SystemBox::new` resolves `CpuConcurrent` for a system whose
    /// `Access` is empty.
    #[test]
    fn new_resolves_non_universal_access_as_cpu_concurrent() {
        let sys = ProbeSystem {
            meta: SystemMeta::for_testing("probe_empty"),
        };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let sb = SystemBox::new(boxed);
        assert_eq!(
            sb.kind,
            SystemKind::CpuConcurrent,
            "empty access must resolve CpuConcurrent"
        );
        assert!(
            !sb.kind.runs_on_dispatcher(),
            "a CpuConcurrent system is worker-eligible"
        );
        assert_eq!(sb.name, "probe_empty");
    }

    /// `SystemBox::new` resolves `CpuExclusive` when the wrapped system
    /// declares `Access::universal()`. This is the path
    /// `ExclusiveFunctionSystem` exercises post-`initialize`.
    #[test]
    fn new_resolves_universal_access_as_cpu_exclusive() {
        let mut meta = SystemMeta::for_testing("probe_universal");
        meta.access = Access::universal();
        let sys = ProbeSystem { meta };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let sb = SystemBox::new(boxed);
        assert_eq!(
            sb.kind,
            SystemKind::CpuExclusive,
            "Access::universal() must resolve CpuExclusive"
        );
        assert!(
            sb.kind.runs_on_dispatcher(),
            "a CpuExclusive system runs on the dispatcher"
        );
        assert_eq!(sb.name, "probe_universal");
    }

    /// SCH15 desync probe (Phase 4 CR-B) — the cached `CpuExclusive` axis
    /// must match a fresh `system.access().is_universal()` read at
    /// construction time.
    #[test]
    fn cache_matches_access_is_universal() {
        let mut meta = SystemMeta::for_testing("probe_check");
        meta.access = Access::universal();
        let sys = ProbeSystem { meta };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let sb = SystemBox::new(boxed);
        // SCH15 invariant — `debug_assert!` in `Schedule::run` will trip if
        // this equality is ever violated by a future refactor.
        assert_eq!(
            sb.kind == SystemKind::CpuExclusive,
            sb.system.access().is_universal()
        );
    }
}
