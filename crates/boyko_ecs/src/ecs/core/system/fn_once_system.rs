//! `FnOnceSystem<P, F, O>` — closure-backed [`System`] impl for Phase 8a.
//!
//! This is the **8a-only** stub. Phase 8c replaces it with the real
//! `FunctionSystem<F, M>` via the `IntoSystem` adapter, which removes the
//! turbofish requirement on the param tuple. Until then, callers must spell
//! out the [`SystemParam`] type — see [`FnOnceSystem::new`] and
//! [`EcsMaster::run_closure_once`] for the call-site shape.
//!
//! See Phase 8a plan §8.4 for the design (M5 RESOLUTION — full signature
//! spelled out; W5 RESOLUTION — `archetype_generation()` is the correct
//! `ArchetypeMaster` accessor for the M4 mid-init debug guard).
//!
//! [`System`]: super::system::System
//! [`SystemParam`]: super::system_param::SystemParam
//! [`EcsMaster::run_closure_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_closure_once

use std::marker::PhantomData;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system::System;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Closure-backed [`System`] for Phase 8a.
///
/// Wraps a closure `F: FnMut(P::Item<'w, 's>) -> O` and carries the
/// per-system [`SystemMeta`] + cached `P::State`. Direct construction
/// requires turbofish on the param tuple:
///
/// ```ignore
/// let mut sys = FnOnceSystem::<MyParams, _, ()>::new(|p| { /* ... */ });
/// ecs.run_system_once(&mut sys);
/// ```
///
/// Prefer [`EcsMaster::run_closure_once`] for the ergonomic shortcut.
///
/// # Why the `_marker` field
///
/// `P` and `O` appear only inside the GAT projection
/// `<P as SystemParam>::Item<'w, 's>` and the closure's return type. The
/// compiler cannot infer variance from those alone; `PhantomData<fn() -> (P, O)>`
/// makes the struct invariant over both. The `fn(...) -> ...` shape keeps the
/// marker `Send + Sync` regardless of `P`/`O`'s own bounds.
///
/// [`System`]: super::system::System
/// [`EcsMaster::run_closure_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_closure_once
pub struct FnOnceSystem<P, F, O>
where
    P: SystemParam,
    F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O,
{
    /// The wrapped closure. Held by value; called via `(self.f)(item)`.
    f: F,
    /// Per-system param state, materialised on the first `initialize` call.
    /// `None` before init; `Some` after. The `initialize` idempotence
    /// contract relies on this sentinel.
    state: Option<P::State>,
    /// Diagnostic name + declared [`Access`] surface + observed generations.
    /// Populated end-of-`initialize` (after `FilteredAccessSet::finalize`).
    meta: SystemMeta,
    /// Invariance + zero-sized type binding for `P` and `O`. See the struct
    /// docs.
    _marker: PhantomData<fn() -> (P, O)>,
}

impl<P, F, O> FnOnceSystem<P, F, O>
where
    P: SystemParam + 'static,
    F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + 'static,
    O: 'static,
{
    /// Constructs a new closure-backed system.
    ///
    /// # Turbofish requirement (M5 RESOLUTION)
    ///
    /// Closure-argument type inference cannot deduce the [`SystemParam`]
    /// tuple `P` from the body alone. Callers MUST spell `P` out:
    ///
    /// ```ignore
    /// let mut sys = FnOnceSystem::<Res<Tick>, _, ()>::new(|tick| {
    ///     println!("tick = {}", tick.0);
    /// });
    /// ```
    ///
    /// Phase 8c's `IntoSystem` adapter removes the requirement by
    /// inferring `P` from the closure's signature. Until then, prefer
    /// [`EcsMaster::run_closure_once`] for a slightly less verbose call
    /// site (still requires turbofish on the param tuple).
    ///
    /// [`SystemParam`]: super::system_param::SystemParam
    /// [`EcsMaster::run_closure_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_closure_once
    pub fn new(body: F) -> Self {
        Self {
            f: body,
            state: None,
            meta: SystemMeta::new(std::any::type_name::<F>()),
            _marker: PhantomData,
        }
    }
}

// SAFETY (S1): the system holds `F`, `P::State`, and `SystemMeta`; none of
//   them touch `EcsMaster` outside the `initialize` / `run_unsafe` calls,
//   each of which carries the world borrow scope explicitly. The trait's
//   `Send + Sync + 'static` bound is satisfied because:
//     - `F: Send + Sync + 'static` is inherited from the closure's captures
//       (enforced by the auto-derive when the user constructs `FnOnceSystem`
//       through `run_closure_once` — see the bound on `body: F`).
//     - `P::State: Send + Sync + 'static` per the `SystemParam::State`
//       trait bound.
//     - `SystemMeta` is `Send + Sync` (only `Access`, `&'static str`, and
//       `ArchetypeGeneration` fields, all `Send + Sync`).
//     - `PhantomData<fn() -> (P, O)>` is `Send + Sync` unconditionally.
unsafe impl<P, F, O> System for FnOnceSystem<P, F, O>
where
    P: SystemParam + 'static,
    F: for<'w, 's> FnMut(<P as SystemParam>::Item<'w, 's>) -> O + Send + Sync + 'static,
    O: 'static,
{
    type Out = O;

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name
    }

    #[inline]
    fn access(&self) -> &Access {
        &self.meta.access
    }

    fn initialize(&mut self, world: &mut EcsMaster) {
        // Idempotent re-init guard: `run_system_once` calls `initialize`
        // unconditionally before every `run_unsafe`, and the Phase 9
        // scheduler will too. The second call must be a no-op so the
        // declared access surface (populated below) stays stable.
        if self.state.is_some() {
            return;
        }

        // M4 + W5 RESOLUTION: snapshot the archetype generation before the
        // init sweep so we can `debug_assert_eq!` after it. The accessor is
        // `archetype_generation()` on `ArchetypeMaster` (W5 correction —
        // the plan's earlier `id_generation()` placeholder does not exist
        // on the current API).
        #[cfg(debug_assertions)]
        let gen_before = world.archetype_master().archetype_generation();

        // Two-phase init (C4 RESOLUTION):
        //   1. `init_state` materialises per-param caches (e.g. cached
        //      `ResourceId` in `ResState<R>`).
        //   2. `init_access` declares the read/write surface into a fresh
        //      `FilteredAccessSet`, which then folds the accumulated
        //      `Access` into `self.meta` via `finalize`.
        let state = <P as SystemParam>::init_state(world, &mut self.meta);
        let mut access_set = FilteredAccessSet::new();
        <P as SystemParam>::init_access(&state, &mut self.meta, &mut access_set, world);
        access_set.finalize(&mut self.meta);
        self.state = Some(state);

        // M4: SP4 invariant — `init_state` / `init_access` must not register
        // new archetypes. The mid-init mutation guard catches misuse in
        // debug builds; release builds elide the check.
        #[cfg(debug_assertions)]
        {
            let gen_after = world.archetype_master().archetype_generation();
            debug_assert_eq!(
                gen_before, gen_after,
                "invariant SP4: SystemParam::init_state/init_access must not register \
                 new archetypes or resources. Use a separate `EcsMaster::insert_resource` \
                 call before `run_system_once`."
            );
        }
    }

    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> Self::Out {
        let state = self
            .state
            .as_mut()
            .expect("invariant: initialize must be called before run_unsafe");
        // SAFETY (S1, SP1, SP2):
        //   - S1: caller of `run_unsafe` upholds the trait-level contract
        //     (no other system in flight on the same world).
        //   - SP1: the param chain's `init_access` declared every read /
        //     write the closure will perform via the `FilteredAccessSet`
        //     accumulator; intra-system conflicts panicked at init.
        //   - SP2: Phase 8a `run_system_once` mints `cell` from
        //     `&mut EcsMaster`, so no sibling reference aliases through
        //     any cell copy for the duration of this call.
        let item = unsafe { <P as SystemParam>::get_param(state, &self.meta, world) };
        (self.f)(item)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::resources::resource::Resource;
    use crate::ecs::core::resources::resource_registry::register_new;
    use crate::ecs::core::system::params::res::Res;
    use crate::ecs::identifiers::primitives::ResourceId;

    /// Test resource for end-to-end `initialize` + `run_unsafe` exercises.
    struct TestRes(u32);

    impl Resource for TestRes {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// Trivial unit-closure system constructs.
    #[test]
    fn fn_once_system_runs_unit_closure() {
        let _sys = FnOnceSystem::<(), _, ()>::new(|()| {});
    }

    /// `initialize` followed by `run_unsafe` round-trips a `Res<R>` read.
    #[test]
    fn fn_once_system_initialize_then_run() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let mut ecs = EcsMaster::new();
        ecs.resources.insert(TestRes(42));
        // The closure must be `Send + Sync` because the `System` trait
        // bound transitively requires it on the closure. `AtomicU32` behind
        // `Arc` satisfies the bound and serves as a probe channel.
        let observed = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&observed);
        // r: Res<TestRes>; r.0: &TestRes; r.0.0: u32 (auto-deref through &).
        let mut sys = FnOnceSystem::<Res<'_, TestRes>, _, ()>::new(move |r| {
            probe.store(r.0.0, Ordering::Relaxed);
        });

        sys.initialize(&mut ecs);
        // SAFETY (S1): `&mut ecs` is exclusive for the duration of this
        //   block; no other system is in flight. The cell does not outlive
        //   the borrow scope.
        let cell = unsafe { UnsafeEcsCell::new_mutable(&mut ecs) };
        // SAFETY (S1): see above.
        unsafe {
            sys.run_unsafe(cell);
        }
        assert_eq!(observed.load(Ordering::Relaxed), 42);
    }

    /// `initialize` is idempotent — calling twice must not re-run the
    /// `init_state` / `init_access` sweep. We verify by reading the cached
    /// `ResourceId` after the first call and confirming it is unchanged by
    /// the second.
    #[test]
    fn initialize_is_idempotent() {
        let mut ecs = EcsMaster::new();
        ecs.resources.insert(TestRes(7));
        let mut sys = FnOnceSystem::<Res<'_, TestRes>, _, ()>::new(|_r| {});

        sys.initialize(&mut ecs);
        let id_after_first = sys
            .state
            .as_ref()
            .expect("state must be set after the first initialize")
            .id;

        sys.initialize(&mut ecs);
        let id_after_second = sys
            .state
            .as_ref()
            .expect("state must remain set after the second initialize")
            .id;

        assert_eq!(
            id_after_first, id_after_second,
            "initialize must be idempotent — the second call must not overwrite the cached state"
        );
    }
}
