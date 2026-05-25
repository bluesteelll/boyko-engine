//! `Res<'w, R>` — shared resource access `SystemParam`.
//!
//! See Phase 8a plan §6 (Decision D4) and §14.1 (hot path algorithm). The
//! type-erased fast path goes through [`Resources::get_ptr_by_id`] using the
//! cached [`ResourceId`] stored in [`ResState`] (W1 RESOLUTION — no
//! `OnceLock` load per `get_param`).
//!
//! [`Resources::get_ptr_by_id`]: crate::ecs::core::resources::resources::Resources::get_ptr_by_id
//! [`ResourceId`]: crate::ecs::identifiers::primitives::ResourceId

use std::marker::PhantomData;
use std::ops::Deref;

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

/// Shared borrow of the world-global resource of type `R` for the system
/// invocation scope `'w`.
///
/// Produced by [`Res::<R> as SystemParam::get_param`] using the cached
/// [`ResourceId`] stashed in [`ResState`] during init. `Deref<Target = R>`
/// makes the wrapper transparent to system bodies.
///
/// # Lifetime contract (§6.4)
///
/// `Res<'w, R>` borrows the resource for `'w`, the world-access scope.
/// `SystemParam::Item<'w, 's>` resolves to `Res<'w, R>` — `'s` is unused for
/// `Res` because the state contains only the cached [`ResourceId`].
///
/// [`ResourceId`]: crate::ecs::identifiers::primitives::ResourceId
#[repr(transparent)]
pub struct Res<'w, R: Resource>(pub(crate) &'w R);

impl<R: Resource> Deref for Res<'_, R> {
    type Target = R;

    #[inline]
    fn deref(&self) -> &R {
        self.0
    }
}

/// Per-system state for [`Res<R>`].
///
/// Cached at [`SystemParam::init_state`] time; consumed unchanged by every
/// subsequent [`SystemParam::get_param`] call (W1 RESOLUTION — the
/// `OnceLock` load on `R::resource_id()` is paid once, never per access).
///
/// `Copy` is derived because the state carries only the `ResourceId` and a
/// zero-sized `PhantomData`; tuple states benefit from the cheap copy when
/// destructured. `PhantomData<fn() -> R>` keeps `R` invariant without
/// imposing `Send + Sync` constraints from `R` itself onto the state — the
/// state is independently `Send + Sync` (required by [`SystemParam::State`]).
#[derive(Clone, Copy)]
pub struct ResState<R: Resource> {
    /// Resource id cached on first init. Bound to `R` by construction —
    /// only `R::resource_id()` can mint a `ResourceId` matching this `R`.
    pub(crate) id: ResourceId,
    /// Type binding without forcing `R: Send + Sync` onto the state.
    /// `fn() -> R` is `Send + Sync` regardless of `R`'s bounds.
    pub(crate) _marker: PhantomData<fn() -> R>,
}

// SAFETY (SP1, SP2, SP4): the impl is parameterised over the outer
//   lifetime `'a`, matching Bevy's `unsafe impl<'a, R: Resource> SystemParam
//   for Res<'a, R>` shape (C2 RESOLUTION). The generic blanket satisfies
//   the trait's `Item<'w, 's>: SystemParam<State = Self::State>` bound for
//   all lifetimes.
//   - SP1: `init_access` declares a single resource read via
//     `FilteredAccessSet::add_resource_read` keyed on the cached id.
//   - SP2: `get_param` only dereferences the resource pointed to by the id
//     declared in `init_access`; aliasing is upheld by the caller via the
//     `FilteredAccessSet`/scheduler protocol.
//   - SP4: `init_state` mutates no archetype/resource registry — it only
//     reads `R::resource_id()` (a pure `OnceLock` cache) and stashes the
//     resulting id in the state.
unsafe impl<'a, R: Resource> SystemParam for Res<'a, R> {
    type State = ResState<R>;
    type Item<'w, 's> = Res<'w, R>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        // W1: pay the `R::resource_id()` `OnceLock` load once at init.
        let id = R::resource_id();
        ResState {
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
            .add_resource_read(state.id, std::any::type_name::<Self>())
            .unwrap_or_else(|conflict| intra_system_conflict_panic(conflict));
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP1, SP2, U_C2): `init_access` declared a read of
        //   `state.id` via `FilteredAccessSet::add_resource_read`. The
        //   protocol guarantees no `ResMut<R>` for the same id is being
        //   fetched in this stage (intra-system conflict caught at
        //   `init_access`; cross-system caught by Phase 9 scheduler).
        //   `world.resources()` is a by-value call on a `Copy` cell — no
        //   `&self` retag occurs (C1 RESOLUTION).
        let resources = unsafe { world.resources() };

        // W1 FAST PATH: the cached `state.id` flows directly into the
        //   untyped lookup, bypassing `R::resource_id()`'s `OnceLock`
        //   acquire-load on every `get_param`.
        let ptr = resources
            .get_ptr_by_id(state.id)
            .unwrap_or_else(|| missing_resource_panic::<R>());

        // SAFETY (SP2): `ptr` was minted from a populated slot whose
        //   registration was bound to `R` at insert time (R1: bit-implies-
        //   init). `ResState<R>` ties `state.id` to `R` at the type system
        //   level — the cast is type-correct. The `&` borrow's lifetime is
        //   `'w`, bounded by the world's borrow scope.
        Res(unsafe { &*(ptr as *const R) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::resources::resource_registry::register_new;

    /// Test resource used to exercise `Res<R>::get_param` and its lifetime
    /// contract.
    struct TestRes(u32);

    impl Resource for TestRes {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// Second test resource to exercise the disjoint-id intra-system add.
    struct TestRes2(#[allow(dead_code)] u64);

    impl Resource for TestRes2 {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// Compile-only shim — instantiating this proves `T: SystemParam`.
    fn assert_impl<T: SystemParam>() {}

    /// `Res<'_, TestRes>` satisfies the `SystemParam` bound.
    #[test]
    fn res_is_system_param() {
        assert_impl::<Res<'static, TestRes>>();
    }

    /// `Deref` round-trips the underlying value.
    #[test]
    fn res_deref_reads_back_value() {
        let value = TestRes(123);
        let res: Res<'_, TestRes> = Res(&value);
        // `res.0` is the wrapped `&R`; reach the inner `u32` either via
        // `Deref` (`*res`) or explicit `.0.0`.
        assert_eq!((*res).0, 123, "Deref must yield the borrowed value");
        assert_eq!(res.0.0, 123, "tuple-struct field access bypasses Deref");
    }

    /// `init_state` caches `R::resource_id()` exactly once and the cached id
    /// matches a subsequent direct call to `R::resource_id()`.
    #[test]
    fn init_state_caches_resource_id() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::new("test");
        let state = <Res<'_, TestRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        assert_eq!(
            state.id,
            TestRes::resource_id(),
            "ResState must cache R::resource_id()"
        );
    }

    /// `init_access` records a resource-read bit on the accumulator. The
    /// `FilteredAccessSet` carries the declared id forward.
    #[test]
    fn init_access_adds_resource_read_to_set() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::new("test");
        let state = <Res<'_, TestRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        let mut set = FilteredAccessSet::new();
        <Res<'_, TestRes> as SystemParam>::init_access(&state, &mut meta, &mut set, &mut ecs);
        set.finalize(&mut meta);
        // Probe with a writer on the same id — must conflict, proving the
        // read bit is set.
        let mut probe = crate::ecs::core::system::access::Access::new();
        probe.add_resource_write(state.id);
        assert!(
            meta.access().conflicts_with(&probe),
            "finalized meta must carry the Res<TestRes> read bit"
        );
    }

    /// `init_access` for two disjoint `Res` params accepts both.
    #[test]
    fn init_access_two_disjoint_reads_ok() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::new("test");
        let s1 = <Res<'_, TestRes> as SystemParam>::init_state(&mut ecs, &mut meta);
        let s2 = <Res<'_, TestRes2> as SystemParam>::init_state(&mut ecs, &mut meta);
        let mut set = FilteredAccessSet::new();
        // Both calls must succeed (no panic).
        <Res<'_, TestRes> as SystemParam>::init_access(&s1, &mut meta, &mut set, &mut ecs);
        <Res<'_, TestRes2> as SystemParam>::init_access(&s2, &mut meta, &mut set, &mut ecs);
    }

    /// `get_param` returns a `Res<R>` whose `Deref` matches the inserted
    /// value. Exercises the full hot path: `UnsafeEcsCell::resources()` →
    /// `Resources::get_ptr_by_id` → cast → `Res(&*ptr as *const R)`.
    #[test]
    fn get_param_returns_correct_value() {
        let mut ecs = EcsMaster::new();
        ecs.resources.insert(TestRes(99));
        let mut meta = SystemMeta::new("test");
        let mut state = <Res<'_, TestRes> as SystemParam>::init_state(&mut ecs, &mut meta);

        // SAFETY (U_C1): the cell does not outlive `&mut ecs`; the borrow
        //   scope is the line below.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        // SAFETY (SP1, SP2): `init_access` was *not* called in this test
        //   because we are exercising `get_param` directly. The contract is
        //   that the caller of `get_param` upholds the access discipline;
        //   in this test no other accessor is live, so SP2 holds. SP1 is a
        //   contractual obligation, not a soundness one for this direct
        //   call.
        let res: Res<'_, TestRes> =
            unsafe { <Res<'_, TestRes> as SystemParam>::get_param(&mut state, &meta, cell) };
        // `res.0` is `&TestRes`; reach the inner `u32` via Deref or `.0.0`.
        assert_eq!((*res).0, 99, "get_param must round-trip the inserted value");
    }
}
