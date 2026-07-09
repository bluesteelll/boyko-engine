//! `Local<'s, T>` — per-system private state `SystemParam` (Phase 13).
//!
//! `Local<T>` injects a system-private `T` that is default-initialized once
//! (at `FunctionSystem::initialize`) and persisted across every run of the
//! cached system — frame-to-frame under the Phase 9 scheduler. It is the
//! simplest SystemParam in the engine: structurally a twin of
//! [`EventReader<'s, E>`](super::event_reader::EventReader) minus the cached
//! buffer pointer. It declares ZERO access, so it adds no conflict-graph edge
//! and never blocks parallel system execution.
//!
//! See Phase 13 plan §2 (Decisions A1/B1/F1/F2), §3 (this source), §7 (SAFETY).
//!
//! # Distinctness (no design mechanism — falls out of tuples)
//!
//! Two `Local<u32>` in one system get two independent `u32` slots, because
//! `Local` is a *positional* param: the system's `Param` is the tuple of its
//! arguments, and `(Local<u32>, Local<u32>)::State = (u32, u32)` (see
//! `tuple_impl.rs:91` + `:125`). The tuple position is the key; there is no
//! `TypeId` map. Verified by test, not by code (Phase 13 §6 test 2).

// `Local` is exposed through `boyko_ecs::ecs::core::system::params` but no
// consumer inside the lib build constructs it yet — Phase 13 integration
// tests (`tests/phase13_local_systemparam.rs`) exercise the public surface
// end-to-end. Mirror the suppression used by `commands.rs` / `event_reader.rs`.
#![allow(dead_code)]

use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// A system-private value of type `T`, persisted across runs.
///
/// `Local<T>` borrows the `T` cached in the system's state slot for the
/// invocation scope `'s`. The value is `T::default()`-initialized once when the
/// system is first initialized, and survives across frames. `Deref` /
/// `DerefMut` make the wrapper transparent to system bodies.
///
/// # Distinctness
///
/// Multiple `Local<T>` of the same `T` in one system each receive their own
/// independent storage (positional, not type-keyed — see module docs).
///
/// # Lifetime
///
/// `'s` is the state scope. The `'w` world-access lifetime is unused (dropped at
/// the `Item<'w, 's>` projection — same as [`EventReader`]). `Local` performs no
/// world access whatsoever.
///
/// # Bounds (Phase 13 Decision A1 + B1)
///
/// `T: Send + Sync + Default + 'static`. `Send + Sync` is required by
/// [`SystemParam::State`] (the containing system must migrate across Phase 9
/// workers). `Default` supplies the one-time initial value.
///
/// [`EventReader`]: super::event_reader::EventReader
#[repr(transparent)]
pub struct Local<'s, T: Send + Sync + Default + 'static>(pub(crate) &'s mut T);

impl<T: Send + Sync + Default + 'static> Deref for Local<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: Send + Sync + Default + 'static> DerefMut for Local<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.0
    }
}

// F2: conditional `Debug` — gated on `T: Debug` via a standalone impl so the
// struct definition does not force `T: Debug` onto every `Local<T>`.
impl<T: Send + Sync + Default + fmt::Debug + 'static> fmt::Debug for Local<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Local").field(&self.0).finish()
    }
}

// SAFETY (Phase 13 §7 — SP1, SP2, SP4):
//   - SP1: `init_access` declares NO component / resource access. A `Local`
//     touches only the system's own private state slot, owned solely by this
//     system (`FunctionSystem::state`). `Access` has only component / resource
//     bitmasks — no field a `Local` could register, so "no access" is complete.
//   - SP2: `get_param` performs a pure borrow rebind of the `&'s mut Self::State`
//     handed in by the caller — no `world` touch, no aliasing minted.
//   - SP4: `init_state` constructs `T::default()` — no archetype / resource
//     registry mutation (debug-asserted by `FunctionSystem::initialize`).
unsafe impl<T: Send + Sync + Default + 'static> SystemParam for Local<'_, T> {
    type State = T;
    type Item<'w, 's> = Local<'s, T>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        // B1: the one-time initial value. Runs once per system in `initialize`
        // (cold path). `T::default()` is infallible.
        T::default()
    }

    #[inline]
    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // Decision D: NO access declared. `Local` is invisible to the conflict
        // graph — mirror `Commands::init_access` / `EventReader::init_access`.
        // The required-method body is intentionally empty (the trait has no
        // default body — system_param.rs:125).
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP2): pure rebind of the caller-provided exclusive
        //   `&'s mut Self::State`. No `world` access, no pointer minting, no
        //   aliasing introduced. Identical shape to `EventReader::get_param`.
        Local(state)
    }
}

// F1: `#[repr(transparent)]` over a single `&mut T` ⇒ pointer-sized.
const _: () = assert!(
    core::mem::size_of::<Local<'_, u32>>() == core::mem::size_of::<&mut u32>(),
    "Local<'s, T> must be pointer-sized (Phase 13 F1: #[repr(transparent)])",
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-only shim — instantiating this proves `T: SystemParam`.
    fn assert_impl<T: SystemParam>() {}

    /// A custom `Default` with a non-zero initial value, to prove `init_state`
    /// forwards `T::default()` rather than zeroing.
    #[derive(PartialEq, Eq, Debug)]
    struct Counter(u32);

    impl Default for Counter {
        fn default() -> Self {
            Counter(42)
        }
    }

    /// `Local<'_, u32>` satisfies the `SystemParam` bound.
    #[test]
    fn local_is_system_param() {
        assert_impl::<Local<'static, u32>>();
    }

    /// `Deref` reads back the borrowed default value.
    #[test]
    fn local_deref_reads_back_default() {
        let mut v = 0u32;
        let l = Local(&mut v);
        assert_eq!(*l, 0, "Deref must yield the borrowed value");
    }

    /// `DerefMut` writes propagate to the underlying storage.
    #[test]
    fn local_deref_mut_writes_through() {
        let mut v = 0u32;
        {
            let mut l = Local(&mut v);
            *l = 7;
        }
        assert_eq!(v, 7, "DerefMut must write through to the borrowed slot");
    }

    /// `init_state` returns `T::default()` — `0` for `u32` and the custom `42`
    /// for a manual `Default` impl.
    #[test]
    fn init_state_returns_default() {
        let mut ecs = EcsMaster::new();
        let mut meta = SystemMeta::for_testing("test");
        let state = <Local<'_, u32> as SystemParam>::init_state(&mut ecs, &mut meta);
        assert_eq!(state, 0u32, "init_state must return u32::default()");

        let custom = <Local<'_, Counter> as SystemParam>::init_state(&mut ecs, &mut meta);
        assert_eq!(
            custom,
            Counter(42),
            "init_state must forward a custom Default value"
        );
    }
}
