//! `NonSendResMut<'w, R>` — exclusive **non-`Send`** resource access
//! `SystemParam` (Phase 4 Seam 2 — D6 + CR-A + CR-B + IM-4).
//!
//! Mirror of [`NonSendRes`] with `&mut R` instead of `&R` — and the mutable
//! slab path `UnsafeEcsCell::nonsend_resources_mut` in place of
//! `nonsend_resources`. The access declaration is identical (universal +
//! `mark_requires_dispatcher`), since a NonSend system is dispatcher-solo
//! regardless of read vs write.
//!
//! [`NonSendRes`]: super::nonsend_res::NonSendRes

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::nonsend_resources::nonsend_id;
use crate::ecs::core::resources::resource::NonSendResource;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::missing_non_send_resource_panic;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::NonSendResourceId;

/// Exclusive borrow of the world-global **non-`Send`** resource `R` for the
/// system invocation scope `'w`. `Deref + DerefMut` make the wrapper
/// transparent to system bodies.
#[repr(transparent)]
pub struct NonSendResMut<'w, R: NonSendResource>(pub(crate) &'w mut R);

impl<R: NonSendResource> Deref for NonSendResMut<'_, R> {
    type Target = R;

    #[inline]
    fn deref(&self) -> &R {
        &*self.0
    }
}

impl<R: NonSendResource> DerefMut for NonSendResMut<'_, R> {
    #[inline]
    fn deref_mut(&mut self) -> &mut R {
        &mut *self.0
    }
}

/// Per-system state for [`NonSendResMut<R>`] — same shape as
/// [`NonSendResState<R>`](super::nonsend_res::NonSendResState).
#[derive(Clone, Copy)]
pub struct NonSendResMutState<R: NonSendResource> {
    /// Non-send resource id cached on first init.
    pub(crate) id: NonSendResourceId,
    /// Type binding; `fn() -> R` is `Send + Sync` regardless of `R`.
    pub(crate) _marker: PhantomData<fn() -> R>,
}

// SAFETY (SP1, SP2, SP4 + CR-A): same shape as `NonSendRes<'a, R>`. The
//   `get_param` returns `&mut R`; exclusivity is upheld by the system being
//   `CpuExclusive` (dispatcher-solo at `running == 0`), so no other reference
//   into the same NonSend slot can co-exist for the borrow's lifetime.
unsafe impl<'a, R: NonSendResource> SystemParam for NonSendResMut<'a, R> {
    type State = NonSendResMutState<R>;
    type Item<'w, 's> = NonSendResMut<'w, R>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        NonSendResMutState {
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
        // CR-B + IM-4: universal access ⇒ `CpuExclusive`; explicit dispatcher
        // mark. Identical to `NonSendRes` — read vs write does not change the
        // dispatcher-solo requirement.
        access_set.mark_universal();
        system_meta.mark_requires_dispatcher();
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP1, SP2, CR-A): `init_access` declared universal access →
        //   `CpuExclusive`, so this runs ONLY on the dispatcher thread at
        //   `running == 0`; no worker cell aliases the `&mut`, and the `!Send`
        //   payload is touched single-threaded on its owning thread. The cell
        //   was minted via `new_mutable` (debug-asserted in
        //   `nonsend_resources_mut`); the by-value receiver preserves
        //   write-capable provenance (no `&self` retag).
        let slab = unsafe { world.nonsend_resources_mut() }
            .unwrap_or_else(|| missing_non_send_resource_panic::<R>());

        let ptr = slab
            .get_mut_ptr_by_id(state.id)
            .unwrap_or_else(|| missing_non_send_resource_panic::<R>());

        // SAFETY (SP2): `ptr` was minted from a populated slot bound to `R`
        //   at insert time (N1); `NonSendResMutState<R>` ties `state.id` to
        //   `R`. The `&mut` borrow's lifetime is `'w`; exclusivity is upheld
        //   by the dispatcher-solo resolution above.
        NonSendResMut(unsafe { &mut *(ptr as *mut R) })
    }

    // `apply` / `new_archetype` default to no-ops (forwarded by the tuple
    // impl — IM-4).
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!Send` test resource (raw pointer interior) with a mutable counter.
    struct NonSendCounter {
        value: u32,
        _not_send: *const u8,
    }
    impl NonSendResource for NonSendCounter {}

    fn assert_impl<T: SystemParam>() {}

    #[test]
    fn nonsend_resmut_is_system_param() {
        assert_impl::<NonSendResMut<'static, NonSendCounter>>();
    }

    #[test]
    fn init_access_declares_universal_and_marks_dispatcher() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::for_testing("nonsend_mut_probe");
        let state = <NonSendResMut<'_, NonSendCounter> as SystemParam>::init_state(
            &mut ecs, &mut meta,
        );
        let mut set = FilteredAccessSet::new();
        <NonSendResMut<'_, NonSendCounter> as SystemParam>::init_access(
            &state, &mut meta, &mut set, &mut ecs,
        );
        set.finalize(&mut meta);
        assert!(meta.access().is_universal());
        assert!(meta.requires_dispatcher());
    }

    #[test]
    fn get_param_writes_through() {
        let mut ecs = EcsMaster::new();
        ecs.insert_non_send_resource(NonSendCounter {
            value: 1,
            _not_send: std::ptr::null(),
        });
        let mut meta = SystemMeta::for_testing("nonsend_mut_get");
        let mut state = <NonSendResMut<'_, NonSendCounter> as SystemParam>::init_state(
            &mut ecs, &mut meta,
        );

        // SAFETY (U_C1): cell does not outlive `&mut ecs`.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        {
            // SAFETY (SP1, SP2): direct call — no other accessor is live.
            let mut r: NonSendResMut<'_, NonSendCounter> = unsafe {
                <NonSendResMut<'_, NonSendCounter> as SystemParam>::get_param(
                    &mut state, &meta, cell,
                )
            };
            r.value = 555;
        }

        // The write must persist in the slab.
        assert_eq!(
            ecs.non_send_resource::<NonSendCounter>().value,
            555,
            "DerefMut write through NonSendResMut must persist"
        );
    }
}
