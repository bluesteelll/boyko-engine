//! `NonSendRes<'w, R>` — shared **non-`Send`** resource access `SystemParam`
//! (Phase 4 Seam 2 — D6 + CR-A + CR-B + IM-4).
//!
//! Mirror of [`Res`] for types implementing [`NonSendResource`] (no
//! `Send + Sync` bound). The two surfaces differ in three ways:
//!
//! 1. **State** caches a [`NonSendResourceId`] from the *non-send* registry.
//! 2. **`init_access`** declares **universal access** (CR-B) — so the system
//!    resolves to `SystemKind::CpuExclusive` via the existing derivation and
//!    runs dispatcher-solo when `running == 0` — AND calls
//!    [`SystemMeta::mark_requires_dispatcher`].
//! 3. **`get_param`** dispatches through the lazy
//!    [`UnsafeEcsCell::nonsend_resources`] slab (`Option` — `None` if no
//!    NonSend resource was ever inserted → the missing-resource panic).
//!
//! # The full surface (IM-4 — the Phase-8cd/14b missed-forwarder lesson)
//!
//! `init_state` / `init_access` / `get_param` / `apply` (default no-op) /
//! `new_archetype` (default no-op) are ALL implemented; the variadic-tuple
//! `apply` forwarder in `tuple_impl.rs` already forwards `apply`, so a
//! single-arg `|r: NonSendRes<R>|` (whose `Param` is `(NonSendRes<R>,)`) is
//! not silently no-op'd. The behavioral test
//! `nonsend_system_runs_on_dispatcher_and_observes_resource` guards this.
//!
//! [`Res`]: super::res::Res
//! [`NonSendResource`]: crate::ecs::core::resources::resource::NonSendResource
//! [`NonSendResourceId`]: crate::ecs::identifiers::primitives::NonSendResourceId
//! [`UnsafeEcsCell::nonsend_resources`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::nonsend_resources

use std::marker::PhantomData;
use std::ops::Deref;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::nonsend_resources::nonsend_id;
use crate::ecs::core::resources::resource::NonSendResource;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::missing_non_send_resource_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::NonSendResourceId;

/// Shared borrow of the world-global **non-`Send`** resource `R` for the
/// system invocation scope `'w`. `Deref<Target = R>` makes the wrapper
/// transparent to system bodies.
#[repr(transparent)]
pub struct NonSendRes<'w, R: NonSendResource>(pub(crate) &'w R);

impl<R: NonSendResource> Deref for NonSendRes<'_, R> {
    type Target = R;

    #[inline]
    fn deref(&self) -> &R {
        self.0
    }
}

/// Per-system state for [`NonSendRes<R>`] — the cached non-send id.
///
/// `Copy`; `Send + Sync` regardless of `R`'s (absent) bounds because it
/// carries only the `NonSendResourceId` POD and a `PhantomData<fn() -> R>`
/// (which is `Send + Sync` for any `R`). The `State: Send + Sync + 'static`
/// bound on [`SystemParam`] is therefore satisfied even though `R` is
/// `!Send` — the value never lives inside the state.
#[derive(Clone, Copy)]
pub struct NonSendResState<R: NonSendResource> {
    /// Non-send resource id cached on first init.
    pub(crate) id: NonSendResourceId,
    /// Type binding without forcing any bound onto the state.
    pub(crate) _marker: PhantomData<fn() -> R>,
}

// SAFETY (SP1, SP2, SP4 + CR-A):
//   - SP1: `init_access` declares UNIVERSAL access (CR-B) — the honest
//     superset of every read this param performs — and marks
//     `requires_dispatcher`.
//   - SP2: `get_param` only borrows the NonSend slot for `state.id`; the
//     universal access resolves the system to `CpuExclusive`, so the
//     scheduler runs it dispatcher-solo (`running == 0`) — no concurrent
//     touch of the `!Send` payload (the CR-A single-thread-touch invariant).
//   - SP4: `init_state` mutates no registry structurally — it only mints /
//     reads the per-`R` non-send id (an `OnceLock`-cached registry slot).
unsafe impl<'a, R: NonSendResource> SystemParam for NonSendRes<'a, R> {
    type State = NonSendResState<R>;
    type Item<'w, 's> = NonSendRes<'w, R>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        // Pay the per-`R` non-send id mint once at init (mirrors `ResState`).
        NonSendResState {
            id: nonsend_id::<R>(),
            _marker: PhantomData,
        }
    }

    fn init_access(
        _state: &Self::State,
        system_meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // CR-B: declare UNIVERSAL access so the existing `SystemKind`
        // derivation resolves `CpuExclusive`; SCH15 stays an equality. The
        // conflict graph then serializes the system; EXC2 runs it solo.
        access_set.mark_universal();
        // IM-4: record the dispatcher requirement explicitly on the meta.
        system_meta.mark_requires_dispatcher();
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP1, SP2, CR-A): `init_access` declared universal access →
        //   the system is `CpuExclusive`, so this `get_param` runs ONLY on the
        //   dispatcher thread inside the apply window (`running == 0`); no
        //   worker holds a cell copy aliasing this borrow, and the `!Send`
        //   payload is touched single-threaded on its owning thread (the
        //   external-synchronisation contract). `world.nonsend_resources()` is
        //   a by-value call on a `Copy` cell — no `&self` retag.
        let slab = unsafe { world.nonsend_resources() }
            .unwrap_or_else(|| missing_non_send_resource_panic::<R>());

        let ptr = slab
            .get_ptr_by_id(state.id)
            .unwrap_or_else(|| missing_non_send_resource_panic::<R>());

        // SAFETY (SP2): `ptr` was minted from a populated slot whose
        //   registration was bound to `R` at insert time (N1). `NonSendResState<R>`
        //   ties `state.id` to `R` at the type level. The `&` borrow's lifetime
        //   is `'w`, bounded by the world's borrow scope.
        NonSendRes(unsafe { &*(ptr as *const R) })
    }

    // `apply` defaults to a no-op; the tuple forwarder still forwards it
    // (IM-4). `new_archetype` defaults to a no-op (NonSend access is
    // archetype-independent).
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!Send` test resource (raw pointer interior).
    struct NonSendCounter {
        value: u32,
        _not_send: *const u8,
    }
    impl NonSendResource for NonSendCounter {}

    /// Compile-only shim — instantiating this proves `T: SystemParam`.
    fn assert_impl<T: SystemParam>() {}

    #[test]
    fn nonsend_res_is_system_param() {
        assert_impl::<NonSendRes<'static, NonSendCounter>>();
    }

    /// `init_access` declares universal access AND marks the dispatcher
    /// requirement — the two facts CR-B/IM-4 hinge on.
    #[test]
    fn init_access_declares_universal_and_marks_dispatcher() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::for_testing("nonsend_probe");
        let state =
            <NonSendRes<'_, NonSendCounter> as SystemParam>::init_state(&mut ecs, &mut meta);
        let mut set = FilteredAccessSet::new();
        <NonSendRes<'_, NonSendCounter> as SystemParam>::init_access(
            &state, &mut meta, &mut set, &mut ecs,
        );
        set.finalize(&mut meta);
        assert!(
            meta.access().is_universal(),
            "NonSendRes::init_access must declare universal access (CR-B)"
        );
        assert!(
            meta.requires_dispatcher(),
            "NonSendRes::init_access must mark requires_dispatcher (IM-4)"
        );
    }

    /// `get_param` returns a `NonSendRes<R>` whose `Deref` matches the
    /// inserted value — the full fetch path through `nonsend_resources()`.
    #[test]
    fn get_param_returns_value() {
        let mut ecs = EcsMaster::new();
        ecs.insert_non_send_resource(NonSendCounter {
            value: 77,
            _not_send: std::ptr::null(),
        });
        let mut meta = SystemMeta::for_testing("nonsend_get");
        let mut state =
            <NonSendRes<'_, NonSendCounter> as SystemParam>::init_state(&mut ecs, &mut meta);

        // SAFETY (U_C1): the cell does not outlive `&mut ecs`.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        // SAFETY (SP1, SP2): direct call — no other accessor is live.
        let r: NonSendRes<'_, NonSendCounter> = unsafe {
            <NonSendRes<'_, NonSendCounter> as SystemParam>::get_param(&mut state, &meta, cell)
        };
        assert_eq!(r.value, 77, "get_param must round-trip the inserted value");
    }
}
