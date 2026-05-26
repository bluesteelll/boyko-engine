//! [`SystemBox`] — schedule-internal wrapper around a type-erased system.
//!
//! See Phase 9 plan §5.2 (Round 2 + Round 3 — `is_exclusive` is a build-time
//! cache populated from `Access::is_universal`, not a `SystemMeta` field).
//! Wave 3 Step 8 ships the minimum surface the executor (Wave 4) needs to
//! pick exclusive systems on the apply-window barrier without touching the
//! erased `dyn System` at dispatch time.
//!
//! # Why a cache (Round 2 C9 / OQ-4)
//!
//! `Access::is_universal` is a four-bitmask scan (192 B touched). Hot-path
//! readers (the executor's dispatch loop) consult `is_exclusive` once per
//! system per round; reading the cached bool spares the bitmask scan and
//! keeps the dispatcher's L1d footprint tight. The build-time freeze is
//! sound because `System::initialize` runs exactly once per system before
//! the schedule starts (SCH1) and `Access` is write-once thereafter
//! (FilteredAccessSet contract).
//!
//! Round 3 SCH15 invariant: `Schedule::run` carries a release-cheap
//! `debug_assert!` that `is_exclusive == system.access().is_universal()` to
//! catch a desync if a future refactor forgets to recompute the cache after
//! re-initialising a system.

use crate::ecs::core::system::system::System;

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
/// `system → is_exclusive → name` — by access frequency from the
/// dispatcher's hot loop. The boxed system pointer is touched every
/// dispatch (to call `run_unsafe` / `apply`), the `is_exclusive` flag is
/// touched at the apply-window gate (SCH7) and the exclusive-path branch
/// (EXC2), and `name` is touched only for diagnostics / panic messages.
///
/// # Layout
///
/// `Box<dyn System<Out = ()>>` is a fat pointer (16 B); `bool` + 7 B
/// padding + `&'static str` (16 B fat pointer) = 40 B total under natural
/// alignment. The struct fits in a single cache line.
///
/// [`ScheduleBuilder::build`]: <Wave 4 Step 9 — not yet implemented>
pub(crate) struct SystemBox {
    /// Erased system body. The trait object is `Send + Sync + 'static`
    /// because [`System`] carries the same bound.
    pub(crate) system: Box<dyn System<Out = ()>>,

    /// Build-time cache of `system.access().is_universal()`.
    ///
    /// Read by the executor's apply-window gate (SCH7) and exclusive-path
    /// dispatcher branch (EXC2) on every round.
    pub(crate) is_exclusive: bool,

    /// Build-time cache of `system.name()` — typically
    /// `std::any::type_name::<F>()` for `FunctionSystem` /
    /// `ExclusiveFunctionSystem`. Read only on diagnostic paths (panic
    /// messages, `scheduler-trace` counters).
    pub(crate) name: &'static str,
}

#[allow(dead_code)] // Wave 3 Step 8 — consumed by ScheduleBuilder::build (Wave 4 Step 9).
impl SystemBox {
    /// Wraps an already-initialised system, caching its `is_exclusive` flag
    /// and diagnostic name.
    ///
    /// # Invariants
    ///
    /// * The caller must invoke `system.initialize(world)` **before**
    ///   constructing the box. `Access::is_universal()` reads the
    ///   `Access` surface; calling it on an un-initialised system would
    ///   cache an empty access (false negative — the system would never
    ///   be recognised as exclusive).
    /// * Once boxed, the system's `Access` is treated as frozen. Any
    ///   subsequent change to the access surface desyncs the cache and
    ///   trips the `debug_assert!` in `Schedule::run` (Round 3 SCH15).
    ///
    /// `ScheduleBuilder::build` (Wave 4 Step 9) is the sole call site.
    #[inline]
    pub(crate) fn new(system: Box<dyn System<Out = ()>>) -> Self {
        let is_exclusive = system.access().is_universal();
        let name = system.name();
        Self {
            system,
            is_exclusive,
            name,
        }
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
    }

    /// `SystemBox::new` caches `is_exclusive = false` for a system whose
    /// `Access` is empty.
    #[test]
    fn new_caches_non_universal_access_as_not_exclusive() {
        let sys = ProbeSystem {
            meta: SystemMeta::for_testing("probe_empty"),
        };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let sb = SystemBox::new(boxed);
        assert!(
            !sb.is_exclusive,
            "empty access must produce is_exclusive == false"
        );
        assert_eq!(sb.name, "probe_empty");
    }

    /// `SystemBox::new` caches `is_exclusive = true` when the wrapped
    /// system declares `Access::universal()`. This is the path
    /// `ExclusiveFunctionSystem` exercises post-`initialize`.
    #[test]
    fn new_caches_universal_access_as_exclusive() {
        let mut meta = SystemMeta::for_testing("probe_universal");
        meta.access = Access::universal();
        let sys = ProbeSystem { meta };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let sb = SystemBox::new(boxed);
        assert!(
            sb.is_exclusive,
            "Access::universal() must produce is_exclusive == true"
        );
        assert_eq!(sb.name, "probe_universal");
    }

    /// SCH15 desync probe — the cache must match a fresh
    /// `system.access().is_universal()` read at construction time.
    #[test]
    fn cache_matches_access_is_universal() {
        let mut meta = SystemMeta::for_testing("probe_check");
        meta.access = Access::universal();
        let sys = ProbeSystem { meta };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let sb = SystemBox::new(boxed);
        // SCH15 invariant — `debug_assert!` in `Schedule::run` will trip if
        // this equality is ever violated by a future refactor.
        assert_eq!(sb.is_exclusive, sb.system.access().is_universal());
    }
}
