//! `ResMut<'w, R>` — exclusive resource access `SystemParam`.
//!
//! Mirror of [`Res`] with `&mut R` instead of `&R`. See Phase 8a plan §6
//! (Decision D4) and §14.1 (hot path algorithm) — the path is the same
//! shape with [`Resources::get_mut_ptr_by_id`] in place of `get_ptr_by_id`
//! and [`FilteredAccessSet::add_resource_write`] in place of
//! `add_resource_read`.
//!
//! [`Res`]: super::res::Res
//! [`Resources::get_mut_ptr_by_id`]: crate::ecs::core::resources::resources::Resources::get_mut_ptr_by_id
//! [`FilteredAccessSet::add_resource_write`]: crate::ecs::core::system::filtered_access_set::FilteredAccessSet::add_resource_write

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::diagnostics::{
    intra_system_conflict_panic, missing_resource_panic,
};
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::ResourceId;

/// Exclusive borrow of the world-global resource of type `R` for the system
/// invocation scope `'w`.
///
/// Produced by [`ResMut::<R> as SystemParam::get_param`] using the cached
/// [`ResourceId`] stashed in [`ResMutState`] during init.
/// `Deref + DerefMut` make the wrapper transparent to system bodies.
///
/// # Lifetime contract (§6.4)
///
/// `ResMut<'w, R>` mutably borrows the resource for `'w`. `'s` (state
/// lifetime) is unused for `ResMut` because the state contains only the
/// cached id.
///
/// [`ResourceId`]: crate::ecs::identifiers::primitives::ResourceId
#[repr(transparent)]
pub struct ResMut<'w, R: Resource>(pub(crate) &'w mut R);

impl<R: Resource> Deref for ResMut<'_, R> {
    type Target = R;

    #[inline]
    fn deref(&self) -> &R {
        &*self.0
    }
}

impl<R: Resource> DerefMut for ResMut<'_, R> {
    #[inline]
    fn deref_mut(&mut self) -> &mut R {
        &mut *self.0
    }
}

/// Per-system state for [`ResMut<R>`]. Same shape as
/// [`ResState<R>`](super::res::ResState) — only the trait-impl side differs
/// (write vs. read in `init_access`, mutable pointer in `get_param`).
#[derive(Clone, Copy)]
pub struct ResMutState<R: Resource> {
    /// Resource id cached on first init.
    pub(crate) id: ResourceId,
    /// Type binding; `fn() -> R` is `Send + Sync` regardless of `R`.
    pub(crate) _marker: PhantomData<fn() -> R>,
}

// SAFETY (SP1, SP2, SP4): same shape as `Res<'a, R>`'s impl (Bevy C2
//   generic blanket). Differences:
//   - SP1: `init_access` declares a resource WRITE via
//     `FilteredAccessSet::add_resource_write` — rejects any sibling read
//     OR write to the same id.
//   - SP2: `get_param` returns `&mut R`. Aliasing exclusivity is upheld by
//     the caller via the access protocol; no two `ResMut<R>` (nor a `Res<R>`
//     alongside a `ResMut<R>`) for the same id can co-exist past init.
//   - SP4: `init_state` mutates no registry.
unsafe impl<'a, R: Resource> SystemParam for ResMut<'a, R> {
    type State = ResMutState<R>;
    type Item<'w, 's> = ResMut<'w, R>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        // W1: pay the `R::resource_id()` `OnceLock` load once at init.
        let id = R::resource_id();
        ResMutState {
            id,
            _marker: PhantomData,
        }
    }

    fn init_access(
        state: &Self::State,
        _system_meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        access_set
            .add_resource_write(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP1, SP2, U_C3): `init_access` declared a write of
        //   `state.id` via `FilteredAccessSet::add_resource_write`. The
        //   protocol guarantees no `Res<R>` / `ResMut<R>` for the same id
        //   is being fetched concurrently (intra-system conflict caught at
        //   `init_access`; cross-system caught by Phase 9 scheduler).
        //   `world.resources_mut()` is a by-value call on a `Copy` cell —
        //   no `&self` retag (C1 RESOLUTION). The cell was minted via
        //   `new_mutable` (debug-asserted in `resources_mut`).
        let resources = unsafe { world.resources_mut() };

        // W1 FAST PATH: cached `state.id` flows directly into the untyped
        //   `get_mut_ptr_by_id`, bypassing `R::resource_id()`'s `OnceLock`
        //   acquire-load on every `get_param`.
        let ptr = resources
            .get_mut_ptr_by_id(state.id)
            .unwrap_or_else(|| missing_resource_panic::<R>());

        // SAFETY (SP2): `ptr` was minted from a populated slot whose
        //   registration was bound to `R` at insert time (R1).
        //   `ResMutState<R>` ties `state.id` to `R` at the type level —
        //   the cast is type-correct. The `&mut` borrow's lifetime is
        //   `'w`, bounded by the world's borrow scope; exclusivity is
        //   upheld by the access protocol.
        ResMut(unsafe { &mut *(ptr as *mut R) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::resources::resource_registry::register_new;
    use crate::ecs::core::system::params::res::Res;

    /// Test resource exercised by `ResMut` paths.
    struct TestMutRes(u32);

    impl Resource for TestMutRes {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// Compile-only shim — instantiating this proves `T: SystemParam`.
    fn assert_impl<T: SystemParam>() {}

    /// `ResMut<'_, TestMutRes>` satisfies the `SystemParam` bound.
    #[test]
    fn resmut_is_system_param() {
        assert_impl::<ResMut<'static, TestMutRes>>();
    }

    /// `DerefMut` mutates the underlying value; `Deref` reads it back.
    #[test]
    fn resmut_deref_mut_writes_through() {
        let mut value = TestMutRes(7);
        let mut res: ResMut<'_, TestMutRes> = ResMut(&mut value);
        // Write through DerefMut — `res.0` reaches the inner `R` via
        // `DerefMut`, then `.0` indexes the tuple-struct's `u32` field.
        // The two-step form (`(*res).0`) is equivalent.
        (*res).0 = 42;
        // Read back via Deref.
        assert_eq!((*res).0, 42, "DerefMut write must be observable via Deref");
        // And the original is observably mutated.
        assert_eq!(value.0, 42);
    }

    /// `init_state` caches `R::resource_id()`.
    #[test]
    fn init_state_caches_resource_id() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::for_testing("test");
        let state = <ResMut<'_, TestMutRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        assert_eq!(
            state.id,
            TestMutRes::resource_id(),
            "ResMutState must cache R::resource_id()"
        );
    }

    /// `init_access` records a resource-write bit.
    #[test]
    fn init_access_adds_resource_write() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::for_testing("test");
        let state = <ResMut<'_, TestMutRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        let mut set = FilteredAccessSet::new();
        <ResMut<'_, TestMutRes> as SystemParam>::init_access(
            &state, &mut meta, &mut set, &mut ecs,
        );
        set.finalize(&mut meta);
        // Probe with a reader on the same id — must conflict, proving the
        // write bit is set.
        let mut probe = crate::ecs::core::system::access::Access::new();
        probe.add_resource_read(state.id);
        assert!(
            meta.access().conflicts_with(&probe),
            "finalized meta must carry the ResMut<TestMutRes> write bit"
        );
    }

    /// `get_param` writes via `DerefMut` and the mutation persists in the
    /// underlying `Resources` slab.
    #[test]
    fn get_param_returns_mutable_reference() {
        let mut ecs = EcsMaster::new();
        ecs.resources.insert(TestMutRes(1));
        let mut meta = SystemMeta::for_testing("test");
        let mut state =
            <ResMut<'_, TestMutRes> as SystemParam>::init_state(&mut ecs, &mut meta);

        // SAFETY (U_C1): cell does not outlive `&mut ecs`.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        {
            // SAFETY (SP1, SP2, U_C3): direct `get_param` call — no other
            //   accessor is live in this test scope; the contract is upheld
            //   trivially.
            let mut res: ResMut<'_, TestMutRes> = unsafe {
                <ResMut<'_, TestMutRes> as SystemParam>::get_param(
                    &mut state, &meta, cell,
                )
            };
            // DerefMut → &mut R; `.0` reaches the `u32` inside `TestMutRes`.
            (*res).0 = 555;
        }

        // The mutation must be observable through a fresh `Resources` read.
        let ptr = ecs
            .resources
            .get_ptr_by_id(TestMutRes::resource_id())
            .expect("resource must still be present after ResMut write");
        // SAFETY: `ptr` is a `*const u8` minted from a `Box<TestMutRes>` and
        //   is valid for the lifetime of `&ecs.resources`.
        let v = unsafe { (*(ptr as *const TestMutRes)).0 };
        assert_eq!(v, 555, "DerefMut write through ResMut must persist");
    }

    /// End-to-end smoke: `(Res<X>, ResMut<X>)` in one system trips B0002 on
    /// `init_access` — the read sees the prior write bit (in this ordering
    /// the write goes first because tuple impls run in declaration order;
    /// the second add_resource_read finds the already-set write bit and
    /// returns the conflict).
    #[test]
    #[should_panic(expected = "boyko-B0002")]
    fn res_plus_resmut_same_type_panics_with_b0002() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::for_testing("test");
        let mut set = FilteredAccessSet::new();

        let mut_state =
            <ResMut<'_, TestMutRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        // First add — write succeeds.
        <ResMut<'_, TestMutRes> as SystemParam>::init_access(
            &mut_state, &mut meta, &mut set, &mut ecs,
        );

        let read_state =
            <Res<'_, TestMutRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        // Second add — read sees the write bit, panics with B0002.
        <Res<'_, TestMutRes> as SystemParam>::init_access(
            &read_state, &mut meta, &mut set, &mut ecs,
        );
    }
}
